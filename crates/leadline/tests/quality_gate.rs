//! In-source suppression and the named quality gate, through the CLI.
//!
//! Rating bands, effort weights, and gate evaluation have unit tests in
//! `src/quality.rs`; these tests pin the observable surfaces: which
//! suppression lines silence which violation, what `check` reports, and which
//! exit code the run produces.

use std::path::Path;
use std::process::{Command, Output};
mod common;
use common::temporary_directory;

fn run(dir: &Path, args: &[&str]) -> Output {
    common::leadline()
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout was not JSON ({error}): {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn write(dir: &Path, name: &str, body: &str) {
    if let Some(parent) = dir.join(name).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(dir.join(name), body).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

fn function_names(output: &Output) -> Vec<String> {
    json(output)["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|file| file["functions"].as_array().unwrap())
        .map(|function| function["name"].as_str().unwrap().to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// Suppression
// ---------------------------------------------------------------------------

#[test]
fn nosonar_suppresses_every_dimension_of_its_function() {
    let root = temporary_directory();
    write(
        &root,
        "src/a.ts",
        "function hairy(x: boolean) { if (x) { if (x) { return 1; } } return 0; } // NOSONAR\n",
    );
    let analyzed = run(&root, &["analyze", ".", "--json"]);
    assert!(analyzed.status.success());
    let suppressed = json(&analyzed)["files"][0]["functions"][0]["suppressed_metrics"].clone();
    assert_eq!(
        suppressed,
        serde_json::json!(["cognitive", "cyclomatic", "max_nesting", "crap"])
    );

    let gated = run(
        &root,
        &[
            "check",
            ".",
            "--cyclomatic",
            "1",
            "--cognitive",
            "0",
            "--json",
        ],
    );
    assert_eq!(
        gated.status.code(),
        Some(0),
        "suppression must lift the gate"
    );
    assert!(json(&gated)["files"].as_array().unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn selective_ignore_suppresses_only_the_named_dimensions() {
    let root = temporary_directory();
    write(
        &root,
        "src/a.ts",
        "// leadline: ignore cognitive\nfunction mixed(x: boolean) { if (x) { return 1; } return 0; }\n",
    );
    let only_cognitive = run(&root, &["check", ".", "--cognitive", "0", "--json"]);
    assert_eq!(only_cognitive.status.code(), Some(0));

    // The unsuppressed cyclomatic violation still fails the run.
    let both = run(
        &root,
        &[
            "check",
            ".",
            "--cognitive",
            "0",
            "--cyclomatic",
            "1",
            "--json",
        ],
    );
    assert_eq!(both.status.code(), Some(1));
    assert_eq!(function_names(&both), vec!["mixed"]);

    let analyzed = run(&root, &["analyze", ".", "--json"]);
    assert_eq!(
        json(&analyzed)["files"][0]["functions"][0]["suppressed_metrics"],
        serde_json::json!(["cognitive"])
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn comments_in_every_supported_comment_style_work() {
    let root = temporary_directory();
    // Python hash comments and C block comments carry the same marker.
    write(
        &root,
        "a.py",
        "def hairy(x):\n    # leadline: ignore cognitive\n    if x:\n        return 1\n    return 0\n",
    );
    write(
        &root,
        "b.c",
        "int hairy(int x) {\n  /* NOSONAR */\n  if (x) { return 1; }\n  return 0;\n}\n",
    );
    let output = run(&root, &["analyze", ".", "--json"]);
    assert!(output.status.success());
    let report = json(&output);
    let mut suppressed = Vec::new();
    for file in report["files"].as_array().unwrap() {
        for function in file["functions"].as_array().unwrap() {
            if let Some(metrics) = function.get("suppressed_metrics") {
                suppressed.push(metrics.clone());
            }
        }
    }
    assert_eq!(suppressed.len(), 2, "both files must carry a suppression");
    assert_eq!(suppressed[0], serde_json::json!(["cognitive"]));
    assert_eq!(
        suppressed[1],
        serde_json::json!(["cognitive", "cyclomatic", "max_nesting", "crap"])
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_metric_names_suppress_nothing() {
    let root = temporary_directory();
    write(
        &root,
        "src/a.ts",
        "// leadline: ignore complexity\nfunction plain(x: boolean) { if (x) { return 1; } return 0; }\n",
    );
    let output = run(&root, &["check", ".", "--cyclomatic", "1", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        json(&output)["files"][0]["functions"][0]
            .get("suppressed_metrics")
            .is_none(),
        "an unknown name must not silence a dimension"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_nested_function_comment_never_suppresses_its_enclosing_function() {
    let root = temporary_directory();
    write(
        &root,
        "src/a.ts",
        "function outer(x: boolean) {\n  const inner = () => { // NOSONAR\n    if (x) { return 1; }\n    return 0;\n  };\n  if (x) { return inner(); }\n  return 0;\n}\n",
    );
    let output = run(&root, &["check", ".", "--cyclomatic", "1", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        function_names(&output),
        vec!["outer"],
        "only the enclosing function may remain a finding"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn suppression_never_hides_a_parse_error() {
    let root = temporary_directory();
    write(
        &root,
        "src/broken.ts",
        "// NOSONAR\nfunction broken( { return 1; }\n",
    );
    let output = run(&root, &["check", ".", "--cyclomatic", "100", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        !json(&output)["files"][0]["parse_errors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---------------------------------------------------------------------------
// Named quality gate
// ---------------------------------------------------------------------------

const WORKING_TREE_GATE: &str = "\
[thresholds.function]
cyclomatic = 2
[quality_gate]
name = \"team\"
maintainability_rating = \"B\"
effort_minutes = 12
";

/// Four `if`s over a cyclomatic limit of 2: three overage units at four
/// minutes each, over seven source lines.
const BRANCHY: &str = "\
function hairy(x: boolean) {
  if (x) { return 1; }
  if (x) { return 2; }
  if (x) { return 3; }
  if (x) { return 4; }
  return 0;
}
";

#[test]
fn gate_reports_ratings_and_effort_alongside_the_violation() {
    let root = temporary_directory();
    write(&root, "leadline.toml", WORKING_TREE_GATE);
    write(&root, "src/a.ts", BRANCHY);

    let output = run(&root, &["check", ".", "--json"]);
    // The threshold violation is still a finding: the exit code is the run's
    // overall verdict, while `quality_gate.status` is the gate's own.
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    let gate = &report["quality_gate"];
    assert_eq!(gate["name"], "team");
    assert_eq!(gate["status"], "passed");
    assert_eq!(gate["scope"], "overall");
    assert_eq!(gate["effort"]["model"], "metric-overage");
    assert_eq!(gate["effort"]["minutes"], 12);
    assert_eq!(gate["effort"]["functions"], 1);
    assert_eq!(gate["effort"]["by_metric"]["cyclomatic"], 12);
    assert_eq!(gate["ratings"]["maintainability"], "B");
    assert!(gate["ratings"]["reliability"].is_null());
    assert!(gate["ratings"]["security"].is_null());
    assert_eq!(gate["suppressed_violations"], 0);
    let conditions = gate["conditions"].as_array().unwrap();
    assert_eq!(conditions.len(), 2);
    assert_eq!(conditions[0]["metric"], "maintainability_rating");
    assert_eq!(conditions[0]["operator"], "<=");
    assert_eq!(conditions[0]["threshold"], "B");
    assert_eq!(conditions[0]["status"], "passed");
    assert_eq!(conditions[1]["metric"], "effort_minutes");
    assert_eq!(conditions[1]["threshold"], 12);
    assert_eq!(conditions[1]["actual"], 12);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn gate_fails_closed_when_a_condition_is_missed() {
    let root = temporary_directory();
    write(
        &root,
        "leadline.toml",
        "[thresholds.function]\ncyclomatic = 2\n[quality_gate]\nname = \"strict\"\nmaintainability_rating = \"A\"\neffort_minutes = 0\n",
    );
    write(&root, "src/a.ts", BRANCHY);

    let output = run(&root, &["check", ".", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let gate = json(&output)["quality_gate"].clone();
    assert_eq!(gate["status"], "failed");
    assert_eq!(gate["ratings"]["maintainability"], "B");
    for condition in gate["conditions"].as_array().unwrap() {
        assert_eq!(condition["status"], "failed");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn suppressed_violations_leave_a_clean_run_and_a_count() {
    let root = temporary_directory();
    write(
        &root,
        "leadline.toml",
        "[thresholds.function]\ncyclomatic = 1\n[quality_gate]\nname = \"team\"\nmaintainability_rating = \"A\"\n",
    );
    write(
        &root,
        "src/a.ts",
        "// leadline: ignore cyclomatic\nfunction quiet(x: boolean) { if (x) { return 1; } return 0; }\n",
    );

    let output = run(&root, &["check", ".", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    assert!(report["files"].as_array().unwrap().is_empty());
    let gate = &report["quality_gate"];
    assert_eq!(gate["suppressed_violations"], 1);
    assert_eq!(gate["effort"]["minutes"], 0);
    assert_eq!(gate["status"], "passed");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_rating_decides_the_exit_code_on_its_own() {
    let root = temporary_directory();
    write(&root, "src/a.ts", "function a() { return 1; }\n");
    write(
        &root,
        "sarif.json",
        r#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"semgrep","rules":[{"id":"x"}]}},"results":[{"ruleId":"x","level":"error","message":{"text":"m"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/a.ts"},"region":{"startLine":1}}}]}]}]}"#,
    );

    write(
        &root,
        "leadline.toml",
        "[quality_gate]\nname = \"sec\"\nsecurity_rating = \"A\"\n",
    );
    let failing = run(&root, &["check", ".", "--sarif", "sarif.json", "--json"]);
    assert_eq!(failing.status.code(), Some(1));
    let gate = json(&failing)["quality_gate"].clone();
    assert_eq!(gate["status"], "failed");
    assert_eq!(gate["ratings"]["security"], "D");
    assert_eq!(gate["conditions"][0]["actual"], "D");

    write(
        &root,
        "leadline.toml",
        "[quality_gate]\nname = \"sec\"\nsecurity_rating = \"D\"\n",
    );
    let passing = run(&root, &["check", ".", "--sarif", "sarif.json", "--json"]);
    assert_eq!(passing.status.code(), Some(0));
    assert_eq!(json(&passing)["quality_gate"]["status"], "passed");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_gate_condition_without_its_input_is_a_usage_error() {
    let root = temporary_directory();
    write(&root, "src/a.ts", "function a() { return 1; }\n");
    write(
        &root,
        "leadline.toml",
        "[quality_gate]\nname = \"sec\"\nsecurity_rating = \"A\"\n",
    );
    let no_scanner = run(&root, &["check", "."]);
    assert_eq!(no_scanner.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&no_scanner.stderr).contains("no scanner input"),
        "{}",
        String::from_utf8_lossy(&no_scanner.stderr)
    );

    write(
        &root,
        "leadline.toml",
        "[quality_gate]\nname = \"team\"\nmaintainability_rating = \"A\"\n",
    );
    let no_thresholds = run(&root, &["check", "."]);
    assert_eq!(no_thresholds.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&no_thresholds.stderr).contains("at least one metric threshold"),
        "{}",
        String::from_utf8_lossy(&no_thresholds.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_gate_configuration_alone_satisfies_the_check_input_requirement() {
    let root = temporary_directory();
    write(
        &root,
        "leadline.toml",
        "[thresholds.function]\ncognitive = 5\n[quality_gate]\nname = \"team\"\neffort_minutes = 100\n",
    );
    write(&root, "src/a.ts", "function a() { return 1; }\n");
    let output = run(&root, &["check", ".", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(json(&output)["quality_gate"]["name"], "team");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_scope_gates_only_the_changed_functions() {
    let root = temporary_directory();
    write(
        &root,
        "leadline.toml",
        "[thresholds.function]\ncyclomatic = 2\n[quality_gate]\nname = \"team\"\neffort_minutes = 100\n",
    );
    write(
        &root,
        "src/a.ts",
        "function legacy(x: boolean) {\n  if (x) { return 1; }\n  if (x) { return 2; }\n  if (x) { return 3; }\n  return 0;\n}\n\nfunction touched(x: boolean) {\n  return x ? 1 : 0;\n}\n",
    );
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "baseline", "-q"]);

    // Only `touched` changes; `legacy` keeps its over-limit cyclomatic count.
    write(
        &root,
        "src/a.ts",
        "function legacy(x: boolean) {\n  if (x) { return 1; }\n  if (x) { return 2; }\n  if (x) { return 3; }\n  return 0;\n}\n\nfunction touched(x: boolean) {\n  if (x) { return 1; }\n  if (x) { return 2; }\n  if (x) { return 3; }\n  return 0;\n}\n",
    );

    let output = run(&root, &["check", ".", "--base", "HEAD", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let gate = json(&output)["quality_gate"].clone();
    assert_eq!(gate["scope"], "changed");
    // `touched` now carries three `if`s against a limit of 2: two overage
    // units at four minutes each. `legacy` is untouched and out of scope.
    assert_eq!(gate["effort"]["minutes"], 8);
    assert_eq!(gate["effort"]["functions"], 1);
    std::fs::remove_dir_all(root).unwrap();
}

// ---------------------------------------------------------------------------
// Cobertura coverage
// ---------------------------------------------------------------------------

#[test]
fn cobertura_coverage_feeds_crap_through_both_flags() {
    let root = temporary_directory();
    write(
        &root,
        "src/a.ts",
        "function pay(ok: boolean) {\n  if (ok) { return 1; }\n  return 0;\n}\n",
    );
    write(
        &root,
        "coverage.xml",
        r#"<coverage line-rate="0.6"><packages><package name="src"><classes><class name="pay" filename="src/a.ts"><lines><line number="1" hits="1"/><line number="2" hits="0" branch="true" condition-coverage="50% (1/2)"/><line number="3" hits="0"/></lines></class></classes></package></packages></coverage>"#,
    );
    for flag in ["--cobertura", "--coverage"] {
        let output = run(
            &root,
            &["check", ".", flag, "coverage.xml", "--crap", "1", "--json"],
        );
        assert_eq!(
            output.status.code(),
            Some(1),
            "{flag} must fail the CRAP gate"
        );
        let crap = json(&output)["files"][0]["functions"][0]["metrics"]["crap"]
            .as_f64()
            .expect("CRAP must be known from Cobertura line and branch data");
        // (covered lines + covered branches) / (known lines + known branches)
        // = (1 + 1) / (3 + 2); CRAP = 2^2 * (1 - 0.4)^3 + 2.
        assert!((crap - 2.864).abs() < 1e-9, "crap was {crap}");
    }
    std::fs::remove_dir_all(root).unwrap();
}
