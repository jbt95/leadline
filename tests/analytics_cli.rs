use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "leadline-analytics-cli-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

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

fn fixture() -> PathBuf {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "a@example.invalid"]);
    git(&root, &["config", "user.name", "Author A"]);
    write(
        &root,
        "leadline.toml",
        "[thresholds.function]\ncognitive = 2\n",
    );
    write(
        &root,
        "src/a.ts",
        "export function calm(x: number): number {\n  return x;\n}\n",
    );
    commit(&root, "base");
    write(
        &root,
        "src/a.ts",
        "export function calm(x: number): number {\n  if (x > 1) { return 1; }\n  if (x > 2) { return 2; }\n  if (x > 3) { return 3; }\n  return x;\n}\n",
    );
    commit(&root, "regression");
    root
}

fn leadline(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn project_json_and_agent_json_are_valid_and_exclusive() {
    let root = fixture();
    let json = leadline(&root, &["project", ".", "--json"]);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["meta"]["schema_version"], 1);
    assert_eq!(value["meta"]["git_available"], true);
    assert!(value["summary"]["files"].as_u64().unwrap() >= 1);
    assert!(!value["risk"]["rows"].as_array().unwrap().is_empty());

    let agent = leadline(&root, &["project", "--format", "agent-json"]);
    assert!(agent.status.success());
    let value: serde_json::Value = serde_json::from_slice(&agent.stdout).unwrap();
    assert!(value["summary"]["files"].as_u64().unwrap() >= 1);
    assert!(value["risk"].as_array().is_some());

    let exclusive = leadline(&root, &["project", "--json", "--format", "agent-json"]);
    assert_eq!(exclusive.status.code(), Some(2));

    let unknown = leadline(&root, &["project", "--bogus"]);
    assert_eq!(unknown.status.code(), Some(2));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn debt_json_reports_new_debt_and_gates_with_flag() {
    let root = fixture();
    let output = leadline(&root, &["debt", "--base", "HEAD~1", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["model"], "change-risk-diff-v1");
    assert!(value["summary"]["new"].as_u64().unwrap() >= 1);

    let gate = leadline(&root, &["debt", "--base", "HEAD~1", "--fail-on-regression"]);
    assert_eq!(gate.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&gate.stdout).contains("new"));

    let clean = leadline(&root, &["debt", "--base", "HEAD", "--fail-on-regression"]);
    assert_eq!(clean.status.code(), Some(0));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_adds_unchanged_and_requires_output() {
    let root = fixture();
    let store = root.join("trends.json");
    let first = leadline(
        &root,
        &["snapshot", ".", "--output", store.to_str().unwrap()],
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("added"));

    let second = leadline(
        &root,
        &["snapshot", ".", "--output", store.to_str().unwrap()],
    );
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("unchanged"));

    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
    assert_eq!(stored["schema_version"], 1);
    assert_eq!(stored["points"].as_array().unwrap().len(), 1);

    let missing = leadline(&root, &["snapshot"]);
    assert_eq!(missing.status.code(), Some(2));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn help_lists_the_new_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .args(["--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for line in ["leadline project", "leadline debt", "leadline snapshot"] {
        assert!(help.contains(line), "missing {line}");
    }
}
