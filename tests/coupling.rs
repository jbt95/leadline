//! Temporal (change) coupling tests.
//!
//! Each test builds a synthetic Git repository with deterministic commits and
//! asserts co-change counts, directional coupling, and Jaccard similarity.

mod common;

use leadline::coupling::{
    COUPLING_SCHEMA_VERSION, CouplingOptions, MAX_COMMIT_FILES, analyze_coupling,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit(root: &Path, message: &str, files: &[&str]) {
    for file in files {
        let path = root.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut contents = std::fs::read_to_string(&path).unwrap_or_default();
        contents.push_str(&format!("line {message}\n"));
        std::fs::write(path, contents).unwrap();
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", message]);
}

fn init(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "a@example.invalid"]);
    git(root, &["config", "user.name", "Author A"]);
}

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-coupling-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn options(min_co_changes: u64, limit: usize) -> CouplingOptions {
    CouplingOptions {
        min_co_changes,
        limit,
    }
}

fn related_paths(report: &leadline::coupling::CouplingReport) -> Vec<&str> {
    report
        .related
        .iter()
        .map(|related| related.path.as_str())
        .collect()
}

#[test]
fn repeated_co_change_reports_directional_and_jaccard() {
    let root = temporary_directory();
    init(&root);
    commit(&root, "one", &["a.ts", "b.ts"]);
    commit(&root, "two", &["a.ts", "b.ts"]);
    commit(&root, "three", &["a.ts", "b.ts"]);
    commit(&root, "four", &["a.ts"]);
    commit(&root, "five", &["b.ts"]);

    let report = analyze_coupling(&root, "a.ts", &options(2, 10)).unwrap();
    assert_eq!(report.schema_version, COUPLING_SCHEMA_VERSION);
    assert!(report.git_available);
    assert_eq!(report.target, "a.ts");
    assert_eq!(report.target_commits, 4);
    assert_eq!(report.pair_commits, 4);
    assert_eq!(report.max_commit_files, MAX_COMMIT_FILES);
    assert_eq!(related_paths(&report), ["b.ts"]);
    let related = &report.related[0];
    assert_eq!(related.commits, 4);
    assert_eq!(related.co_changes, 3);
    assert!((related.directional - 0.75).abs() < 1e-9);
    assert!((related.reverse_directional - 0.75).abs() < 1e-9);
    assert!((related.jaccard - 0.6).abs() < 1e-9);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rare_co_changes_are_filtered_by_default_threshold() {
    let root = temporary_directory();
    init(&root);
    commit(&root, "one", &["a.ts", "b.ts"]);
    commit(&root, "two", &["a.ts", "b.ts"]);
    commit(&root, "three", &["a.ts", "c.ts"]);

    let filtered = analyze_coupling(&root, "a.ts", &options(2, 10)).unwrap();
    assert_eq!(related_paths(&filtered), ["b.ts"]);

    let unfiltered = analyze_coupling(&root, "a.ts", &options(1, 10)).unwrap();
    assert_eq!(related_paths(&unfiltered), ["b.ts", "c.ts"]);
    let c = unfiltered
        .related
        .iter()
        .find(|related| related.path == "c.ts")
        .unwrap();
    assert_eq!(c.co_changes, 1);
    assert!((c.directional - 1.0 / 3.0).abs() < 1e-9);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directional_and_jaccard_differ_for_asymmetric_history() {
    let root = temporary_directory();
    init(&root);
    // b changes four times, always with a; a changes ten times total.
    for index in 0..4 {
        commit(&root, &format!("pair-{index}"), &["a.ts", "b.ts"]);
    }
    for index in 0..6 {
        commit(&root, &format!("alone-{index}"), &["a.ts"]);
    }

    let report = analyze_coupling(&root, "a.ts", &options(1, 10)).unwrap();
    assert_eq!(report.target_commits, 10);
    let related = &report.related[0];
    assert_eq!(related.commits, 4);
    assert_eq!(related.co_changes, 4);
    assert!((related.directional - 0.4).abs() < 1e-9);
    assert!((related.reverse_directional - 1.0).abs() < 1e-9);
    assert!((related.jaccard - 0.4).abs() < 1e-9);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn very_wide_commits_count_but_do_not_pair() {
    let root = temporary_directory();
    init(&root);
    let wide: Vec<String> = (0..MAX_COMMIT_FILES)
        .map(|index| format!("wide{index}.ts"))
        .collect();
    let mut wide_refs: Vec<&str> = wide.iter().map(String::as_str).collect();
    wide_refs.push("a.ts");
    commit(&root, "wide", &wide_refs);
    commit(&root, "pair", &["a.ts", "b.ts"]);

    let report = analyze_coupling(&root, "a.ts", &options(1, 10)).unwrap();
    assert_eq!(
        report.target_commits, 2,
        "wide commit still counts for a.ts"
    );
    assert_eq!(report.pair_commits, 1, "only the small commit pairs");
    assert_eq!(related_paths(&report), ["b.ts"]);
    let b = &report.related[0];
    assert_eq!(b.co_changes, 1);
    assert!((b.directional - 0.5).abs() < 1e-9);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rename_carries_co_change_history() {
    let root = temporary_directory();
    init(&root);
    commit(&root, "one", &["src/old.ts", "src/companion.ts"]);
    git(&root, &["mv", "src/old.ts", "src/new.ts"]);
    commit(&root, "two", &["src/new.ts", "src/companion.ts"]);

    let report = analyze_coupling(&root, "src/new.ts", &options(1, 10)).unwrap();
    assert_eq!(related_paths(&report), ["src/companion.ts"]);
    let related = &report.related[0];
    assert_eq!(related.commits, 2);
    assert_eq!(related.co_changes, 2);
    assert!((related.directional - 1.0).abs() < 1e-9);
    assert!((related.jaccard - 1.0).abs() < 1e-9);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_git_history_reports_unavailable() {
    let root = temporary_directory();
    std::fs::write(root.join("a.ts"), "export const a = 1;\n").unwrap();

    let report = analyze_coupling(&root, "a.ts", &options(1, 10)).unwrap();
    assert!(!report.git_available);
    assert_eq!(report.target_commits, 0);
    assert!(report.related.is_empty());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn limit_truncates_ranked_related_files() {
    let root = temporary_directory();
    init(&root);
    commit(&root, "one", &["a.ts", "b.ts", "c.ts", "d.ts"]);
    commit(&root, "two", &["a.ts", "b.ts", "c.ts", "d.ts"]);

    let report = analyze_coupling(&root, "a.ts", &options(1, 2)).unwrap();
    assert_eq!(report.related.len(), 2);
    assert!(report.truncated);

    let full = analyze_coupling(&root, "a.ts", &options(1, 10)).unwrap();
    assert_eq!(full.related.len(), 3);
    assert!(!full.truncated);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn repeated_analysis_is_deterministic() {
    let root = temporary_directory();
    init(&root);
    commit(&root, "one", &["a.ts", "b.ts"]);
    commit(&root, "two", &["a.ts", "b.ts"]);

    let first = analyze_coupling(&root, "a.ts", &options(1, 10)).unwrap();
    let second = analyze_coupling(&root, "a.ts", &options(1, 10)).unwrap();
    assert_eq!(first, second);

    std::fs::remove_dir_all(root).unwrap();
}

fn git_repo() -> PathBuf {
    let root = temporary_directory();
    init(&root);
    commit(&root, "one", &["src/a.ts", "src/b.ts"]);
    commit(&root, "two", &["src/a.ts", "src/b.ts"]);
    commit(&root, "three", &["src/a.ts", "src/b.ts"]);
    root
}

#[test]
fn cli_coupling_json_exposes_all_three_measures() {
    let root = git_repo();
    let output = common::leadline()
        .current_dir(&root)
        .args(["coupling", "src/a.ts", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["target"], "src/a.ts");
    assert_eq!(value["target_commits"], 3);
    assert_eq!(value["related"][0]["path"], "src/b.ts");
    assert_eq!(value["related"][0]["co_changes"], 3);
    assert!((value["related"][0]["directional"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    assert!((value["related"][0]["jaccard"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_coupling_terminal_lists_related_files() {
    let root = git_repo();
    let output = common::leadline()
        .current_dir(&root)
        .args(["coupling", "src/a.ts"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Historically related files"));
    assert!(stdout.contains("src/b.ts"));
    assert!(stdout.contains("100%"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_coupling_requires_an_existing_target() {
    let root = git_repo();
    let output = common::leadline()
        .current_dir(&root)
        .args(["coupling", "src/missing.ts"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.ts"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_coupling_rejects_targets_outside_the_scope() {
    let root = git_repo();
    std::fs::write(root.join("outside.ts"), "export const outside = 1;\n").unwrap();
    let output = common::leadline()
        .current_dir(&root)
        .args(["coupling", "../outside.ts", "--path", "src"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the scope"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_coupling_help_lists_the_command() {
    let output = common::leadline()
        .args(["coupling", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("leadline coupling"));
}

#[test]
fn cli_coupling_agent_json_is_compact() {
    let root = git_repo();
    let output = common::leadline()
        .current_dir(&root)
        .args(["coupling", "src/a.ts", "--format", "agent-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["target"], "src/a.ts");
    assert_eq!(value["related"][0]["path"], "src/b.ts");
    assert!(value["related"][0].get("directional").is_some());
    assert!(value["related"][0].get("reverse_directional").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_coupling_json_is_deterministic() {
    let root = git_repo();
    let args = ["coupling", "src/a.ts", "--json"];
    let first = common::leadline()
        .current_dir(&root)
        .args(args)
        .output()
        .unwrap();
    let second = common::leadline()
        .current_dir(&root)
        .args(args)
        .output()
        .unwrap();
    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout);
    std::fs::remove_dir_all(root).unwrap();
}
