use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn check_uses_distinct_quality_exit_code() {
    let root = temporary_directory();
    let file = root.join("branch.ts");
    std::fs::write(
        &file,
        "function branch(x: boolean) { if (x) return 1; return 0; }\n",
    )
    .unwrap();

    let failed = check(&file, &["--cyclomatic", "1", "--json"]);
    assert_eq!(failed.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(report["files"][0]["functions"][0]["name"], "branch");

    let passed = check(&file, &["--cyclomatic", "2", "--json"]);
    assert!(passed.status.success());
    let report: serde_json::Value = serde_json::from_slice(&passed.stdout).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_treats_parse_errors_as_findings() {
    let root = temporary_directory();
    let file = root.join("broken.ts");
    std::fs::write(&file, "function broken( { return 1; }\n").unwrap();
    let output = check(&file, &["--cyclomatic", "100", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !report["files"][0]["parse_errors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_requires_a_threshold() {
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .args(["check", "."])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires at least one"));
}

#[test]
fn version_and_subcommand_help_are_available() {
    let version = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        concat!("leadline ", env!("CARGO_PKG_VERSION"))
    );

    let help = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .args(["analyze", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("leadline analyze"));
}

#[test]
fn mcp_serves_tool_list_over_stdio() {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n";
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(request.as_bytes())
        .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["id"], 1);
    let names: Vec<&str> = value["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"analyze"));
    assert!(names.contains(&"analyze_changed"));
    assert!(names.contains(&"analyze_function"));
    assert!(names.contains(&"check"));
    assert!(names.contains(&"explain_metric"));
}

fn check(file: &Path, options: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("check")
        .arg(file)
        .args(options)
        .output()
        .unwrap()
}

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-cli-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn version_subcommand_matches_version_flag() {
    let subcommand = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("version")
        .output()
        .unwrap();
    assert!(subcommand.status.success());
    assert_eq!(
        String::from_utf8(subcommand.stdout).unwrap().trim(),
        concat!("leadline ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_command_is_usage_error_with_clean_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .args(["frobnicate"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("leadline: unknown command"));
}

#[test]
fn json_and_agent_json_together_is_usage_error() {
    let root = temporary_directory();
    let file = root.join("tiny.ts");
    std::fs::write(&file, "function tiny() { return 1; }\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&file)
        .args(["--json", "--format", "agent-json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_function_is_usage_error() {
    let root = temporary_directory();
    let file = root.join("tiny.ts");
    std::fs::write(&file, "function tiny() { return 1; }\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("function")
        .arg(&file)
        .arg("absent")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analyze_without_files_is_incomplete() {
    let missing = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .args(["analyze", "does-not-exist-xyz"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(3));

    let root = temporary_directory();
    let empty = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(empty.status.code(), Some(3));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bad_coverage_input_is_coverage_error() {
    let root = temporary_directory();
    let file = root.join("tiny.ts");
    std::fs::write(&file, "function tiny() { return 1; }\n").unwrap();
    let missing = root.join("missing.info");

    let unreadable = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&file)
        .arg("--lcov")
        .arg(&missing)
        .output()
        .unwrap();
    assert_eq!(unreadable.status.code(), Some(4));

    let unknown_format = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&file)
        .args(["--coverage", "data.bin"])
        .output()
        .unwrap();
    assert_eq!(unknown_format.status.code(), Some(4));

    let bad_xml = root.join("bad.xml");
    std::fs::write(
        &bad_xml,
        "<report><package name=\"p\"><sourcefile name=\"A.java\">\
         <line nr=\"nope\" ci=\"1\"/></sourcefile></package></report>",
    )
    .unwrap();
    let malformed = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&file)
        .arg("--jacoco")
        .arg(&bad_xml)
        .output()
        .unwrap();
    assert_eq!(malformed.status.code(), Some(4));

    let lcov = root.join("cov.info");
    std::fs::write(&lcov, "TN:\nSF:tiny.ts\nDA:1,1\nend_of_record\n").unwrap();
    let detected = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&file)
        .arg("--coverage")
        .arg(&lcov)
        .arg("--json")
        .output()
        .unwrap();
    assert!(detected.status.success());
    let report: serde_json::Value = serde_json::from_slice(&detected.stdout).unwrap();
    assert_eq!(report["files"][0]["functions"][0]["name"], "tiny");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn doctor_reports_sections_successfully() {
    let root = temporary_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("doctor")
        .arg(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for section in [
        "parser: java OK",
        "parser: javascript OK",
        "parser: typescript OK",
        "parser: tsx OK",
        "coverage: lcov OK",
        "coverage: jacoco OK",
        "git: OK",
        "config: OK",
        "harness:",
    ] {
        assert!(stdout.contains(section), "missing {section} in:\n{stdout}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn doctor_rejects_invalid_config() {
    let root = temporary_directory();
    std::fs::write(root.join("leadline.toml"), "[bogus]\nkey = 1\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("doctor")
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("config: INVALID"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn agent_json_shape_on_analyze() {
    let root = temporary_directory();
    std::fs::write(root.join("sample.ts"), "function alpha() { return 1; }\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&root)
        .args(["--format", "agent-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["metric_profile"], "default-v1");
    assert_eq!(value["summary"]["functions"], 1);
    assert_eq!(value["files"][0]["functions"][0]["name"], "alpha");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn agent_json_shape_on_changed() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: boolean) { return x; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "init", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: boolean) { if (x) { return true; } return false; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["changed", "--base", "HEAD", "--format", "agent-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["changed_functions"], 1);
    assert!(value["regressions"].is_array());
    assert!(value["improvements"].is_array());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn config_exclude_filters_directory_but_not_explicit_file() {
    let root = temporary_directory();
    std::fs::write(root.join("keep.ts"), "function keep() { return 1; }\n").unwrap();
    std::fs::write(root.join("skip.ts"), "function skip() { return 2; }\n").unwrap();
    std::fs::write(
        root.join("leadline.toml"),
        "[analysis]\nexclude = [\"skip.ts\"]\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&root)
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let paths: Vec<&str> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["keep.ts"]);

    let explicit = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(root.join("skip.ts"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(explicit.status.success());
    let report: serde_json::Value = serde_json::from_slice(&explicit.stdout).unwrap();
    assert_eq!(report["files"][0]["functions"][0]["name"], "skip");
    std::fs::remove_dir_all(root).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed: {output:?}");
}
