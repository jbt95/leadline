//! `leadline select-tests` process contract: a real repository, a real diff,
//! and the selection the escalation ladder produces.
//!
//! Every case drives the built binary, because what this suite owns is the
//! flags, the rendering, and the tier each selected test is reported with.
//! The ladder itself is covered by `tia`'s own unit tests. Nothing here runs a
//! test suite: selection is emitted, never executed.

mod common;

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn commit(root: &Path, message: &str) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", message, "--no-gpg-sign"]);
}

/// A fresh repository with an identity configured, ready to commit a fixture.
fn repository() -> PathBuf {
    let root = common::temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "tia@example.invalid"]);
    git(&root, &["config", "user.name", "Tia Author"]);
    root
}

/// `b.test.ts` imports `b.ts`, which imports `a.ts`, so a change to `a.ts`
/// reaches the test file through the graph and reaches nothing else.
fn graph_repository() -> PathBuf {
    let root = repository();
    write(&root, "src/a.ts", "export const a = 1;\n");
    write(&root, "src/b.ts", "import './a';\nexport const b = 2;\n");
    write(
        &root,
        "src/b.test.ts",
        "import './b';\nexport const checked = 1;\n",
    );
    commit(&root, "base");
    root
}

fn select_tests(root: &Path, args: &[&str]) -> Output {
    common::leadline()
        .current_dir(root)
        .arg("select-tests")
        .args(args)
        .output()
        .unwrap()
}

/// Parse the contract JSON, with the command's stderr in the failure message.
fn json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "select-tests failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not the selection JSON: {error}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// `(path, tier)` for every selected test, in the order the selection emits.
fn selected(value: &Value) -> Vec<(String, String)> {
    value["selected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|test| {
            (
                test["path"].as_str().unwrap().to_owned(),
                test["tier"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_changed_module_selects_the_test_that_transitively_imports_it() {
    let root = graph_repository();
    write(&root, "src/a.ts", "export const a = 2;\n");
    let value = json(&select_tests(
        &root,
        &["--base", "HEAD", "--format", "json"],
    ));
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["model"], "test-selection");
    assert_eq!(value["base"], "HEAD");
    assert_eq!(value["changed"], serde_json::json!(["src/a.ts"]));
    assert_eq!(
        selected(&value),
        vec![("src/b.test.ts".to_owned(), "graph".to_owned())]
    );
    assert_eq!(value["selected"][0]["via"], serde_json::json!(["src/a.ts"]));
    assert_eq!(value["totals"]["test_files"], 1);
    assert_eq!(value["totals"]["selected"], 1);
    assert!(
        value.get("escalation").is_none(),
        "a graph hit must not escalate: {value}"
    );
    assert_eq!(value["confidence"]["graph_files"], 3);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_unsupported_extension_escalates_to_the_directory() {
    let root = graph_repository();
    write(&root, "src/notes.hbs", "no convention claims this file\n");
    let value = json(&select_tests(
        &root,
        &["--base", "HEAD", "--format", "json"],
    ));
    assert_eq!(value["changed"], serde_json::json!(["src/notes.hbs"]));
    assert_eq!(value["escalation"]["tier"], "directory");
    assert_eq!(
        value["escalation"]["reasons"],
        serde_json::json!(["unsupported extension: src/notes.hbs"])
    );
    assert_eq!(
        selected(&value),
        vec![("src/b.test.ts".to_owned(), "directory".to_owned())]
    );
    assert_eq!(
        value["selected"][0]["via"],
        serde_json::json!(["src/notes.hbs"])
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_documentation_only_change_is_skipped_and_never_silent() {
    let root = graph_repository();
    write(&root, "README.md", "prose that cannot change behavior\n");
    let value = json(&select_tests(
        &root,
        &["--base", "HEAD", "--format", "json"],
    ));
    assert_eq!(value["changed"], serde_json::json!(["README.md"]));
    assert_eq!(value["selected"], serde_json::json!([]));
    assert_eq!(
        value["skipped"],
        serde_json::json!([{ "path": "README.md", "reason": "documentation" }])
    );
    assert!(
        value.get("escalation").is_none(),
        "a skipped file is a decision, not an escalation: {value}"
    );
    let text =
        String::from_utf8_lossy(&select_tests(&root, &["--base", "HEAD"]).stdout).into_owned();
    assert!(
        text.contains("Skipped: README.md (documentation)") && text.contains("Escalation: none"),
        "the terminal view must explain an empty selection: {text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_always_run_match_selects_every_test_file() {
    let root = repository();
    write(
        &root,
        "leadline.toml",
        "[tia]\nalways_run = [\"**/migrations/**\"]\n",
    );
    write(&root, "src/a.test.ts", "export const checked = 1;\n");
    write(&root, "src/nested/b.test.ts", "export const checked = 2;\n");
    commit(&root, "base");
    write(&root, "db/migrations/001.sql", "select 1;\n");
    let value = json(&select_tests(
        &root,
        &["--base", "HEAD", "--format", "json"],
    ));
    assert_eq!(value["escalation"]["tier"], "all");
    assert_eq!(
        value["escalation"]["reasons"],
        serde_json::json!(["always run: db/migrations/001.sql"])
    );
    assert_eq!(
        selected(&value),
        vec![
            ("src/a.test.ts".to_owned(), "all".to_owned()),
            ("src/nested/b.test.ts".to_owned(), "all".to_owned()),
        ]
    );
    assert_eq!(value["totals"]["test_files"], 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn list_format_prints_bare_paths_for_a_runner() {
    let root = graph_repository();
    write(&root, "src/a.ts", "export const a = 2;\n");
    let output = select_tests(&root, &["--base", "HEAD", "--format", "list"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "src/b.test.ts\n",
        "list output carries the paths and nothing else"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explain_adds_the_tier_and_the_provenance() {
    let root = graph_repository();
    write(&root, "src/a.ts", "export const a = 2;\n");
    let output = select_tests(&root, &["--base", "HEAD", "--explain"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("  src/b.test.ts [graph] via src/a.ts"),
        "explain must name the tier and the changed file: {text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn max_ratio_warns_without_failing() {
    let root = graph_repository();
    write(&root, "src/a.ts", "export const a = 2;\n");
    let output = select_tests(
        &root,
        &["--base", "HEAD", "--format", "list", "--max-ratio", "0.5"],
    );
    assert!(
        output.status.success(),
        "--max-ratio is a diagnostic, not a gate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "src/b.test.ts\n");
    let warning = String::from_utf8_lossy(&output.stderr);
    assert!(warning.contains("exceeds --max-ratio"), "{warning}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn staged_compares_the_index_against_head() {
    let root = graph_repository();
    write(&root, "src/a.ts", "export const a = 2;\n");
    git(&root, &["add", "src/a.ts"]);
    let value = json(&select_tests(&root, &["--staged", "--format", "json"]));
    assert_eq!(value["base"], "HEAD");
    assert_eq!(value["changed"], serde_json::json!(["src/a.ts"]));
    assert_eq!(
        selected(&value),
        vec![("src/b.test.ts".to_owned(), "graph".to_owned())]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn staged_and_an_explicit_base_are_exclusive() {
    let root = graph_repository();
    let output = select_tests(&root, &["--staged", "--base", "HEAD"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--staged and --base are exclusive"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_malformed_tia_section_fails_like_a_malformed_analysis_section() {
    let root = graph_repository();
    write(&root, "leadline.toml", "[tia]\ntest_globs = \"nope\"\n");
    let tia = select_tests(&root, &["--base", "HEAD", "--format", "list"]);
    write(&root, "leadline.toml", "[analysis]\nexclude = \"nope\"\n");
    let analysis = select_tests(&root, &["--base", "HEAD", "--format", "list"]);
    assert_eq!(tia.status.code(), Some(2));
    assert_eq!(analysis.status.code(), Some(2));
    let tia_error = String::from_utf8_lossy(&tia.stderr).into_owned();
    assert!(tia_error.contains("test_globs"), "{tia_error}");
    std::fs::remove_dir_all(root).unwrap();
}
