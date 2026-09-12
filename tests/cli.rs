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
fn skill_prints_canonical_skill_text() {
    let expected = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("integrations/common/leadline-skill/SKILL.md"),
    )
    .unwrap();
    for args in [&["skill"][..], &["--skill"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
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

/// Live MCP clients keep stdin open while waiting for responses, so every
/// response must reach stdout immediately. The test above drops stdin first,
/// which lets EOF flush the buffer and hides a missing per-response flush.
#[test]
fn mcp_responds_while_stdin_stays_open() {
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;

    let mut child = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\
              \"params\":{\"protocolVersion\":\"2025-11-25\"}}\n",
        )
        .unwrap();
    stdin.flush().unwrap();

    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let _ = sender.send(reader.read_line(&mut line).map(|_| line));
    });
    let line = match receiver.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(line)) => line,
        other => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("server did not answer while stdin stayed open: {other:?}");
        }
    };
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["id"], 1);
    assert_eq!(value["result"]["protocolVersion"], "2025-11-25");
    drop(stdin);
    let _ = child.wait();
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
    assert!(value["regressions"][0].get("causes").is_none());

    let explained = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args([
            "changed",
            "--base",
            "HEAD",
            "--format",
            "agent-json",
            "--explain",
        ])
        .output()
        .unwrap();
    assert!(explained.status.success());
    let value: serde_json::Value = serde_json::from_slice(&explained.stdout).unwrap();
    assert_eq!(value["regressions"][0]["causes"][0]["line"], 1);
    assert_eq!(value["regressions"][0]["causes"][0]["rule"], "if");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_base_gates_only_changed_violations() {
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
        "function calc(x: number) { if (x > 2) { if (x > 5) { if (x > 9) { return 3; } return 2; } return 1; } return 0; }\n",
    )
    .unwrap();
    let failing = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["check", "--base", "HEAD", "--cognitive", "0"])
        .output()
        .unwrap();
    assert_eq!(failing.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&failing.stdout).contains("calc"),
        "expected calc finding, got:\n{}",
        String::from_utf8_lossy(&failing.stdout)
    );
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: boolean) { return x; }\n",
    )
    .unwrap();
    let passing = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["check", "--base", "HEAD", "--cognitive", "0"])
        .output()
        .unwrap();
    assert!(passing.status.success());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn function_explain_reports_contributions() {
    let root = temporary_directory();
    let file = root.join("branch.ts");
    std::fs::write(
        &file,
        "function pick(x: boolean) { if (x) { return 1; } return 0; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("function")
        .arg(&file)
        .arg("pick")
        .arg("--explain")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("pick"),
        "missing function block in:\n{stdout}"
    );
    assert!(
        stdout.contains("if line 1"),
        "missing contribution line in:\n{stdout}"
    );
    let budgeted = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("function")
        .arg(&file)
        .arg("pick")
        .args(["--format", "agent-json", "--explain"])
        .output()
        .unwrap();
    assert!(budgeted.status.success());
    let value: serde_json::Value = serde_json::from_slice(&budgeted.stdout).unwrap();
    assert!(value["files"][0]["functions"][0]["contributions"].is_array());
    let plain = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("function")
        .arg(&file)
        .arg("pick")
        .args(["--format", "agent-json"])
        .output()
        .unwrap();
    assert!(plain.status.success());
    let value: serde_json::Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert!(
        value["files"][0]["functions"][0]
            .get("contributions")
            .is_none()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sarif_output_has_version_runs_results() {
    let root = temporary_directory();
    std::fs::write(root.join("sample.ts"), "function alpha() { return 1; }\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&root)
        .args(["--format", "sarif"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["version"], "2.1.0");
    assert!(value["runs"].is_array());
    assert!(value["runs"][0]["results"].is_array());
    let conflict = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&root)
        .args(["--json", "--format", "sarif"])
        .output()
        .unwrap();
    assert_eq!(conflict.status.code(), Some(2));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn agent_json_top_marks_truncated() {
    let root = temporary_directory();
    std::fs::write(
        root.join("two.ts"),
        "function alpha() { return 1; }\nfunction beta() { return 2; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("analyze")
        .arg(&root)
        .args(["--format", "agent-json", "--top", "1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["truncated"], true);
    assert_eq!(value["files"][0]["functions"].as_array().unwrap().len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_dir_reuses_results_across_runs() {
    let root = temporary_directory();
    std::fs::write(root.join("sample.ts"), "function alpha() { return 1; }\n").unwrap();
    let cache = root.join("cache");
    let run = |cache: &Path| {
        Command::new(env!("CARGO_BIN_EXE_leadline"))
            .arg("analyze")
            .arg(&root)
            .arg("--cache-dir")
            .arg(cache)
            .arg("--json")
            .output()
            .unwrap()
    };
    let first = run(&cache);
    assert!(first.status.success());
    assert!(cache.join("file-cache.json").is_file());
    let second = run(&cache);
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
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

#[test]
fn changed_staged_and_target_are_mutually_exclusive() {
    let root = temporary_directory();
    git(&root, &["init"]);
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["changed", "--staged", "--target", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("exclusive"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_staged_flag_excludes_unstaged_edits() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return x; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "init", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) { if (x > 1) return 2; return x; } return 0; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["changed", "--base", "HEAD", "--staged", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["functions"][0]["after"]["metrics"]["cyclomatic"], 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_target_flag_compares_two_revisions() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return x; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "target", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return 1; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["changed", "--base", "HEAD~1", "--target", "HEAD", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["functions"][0]["after"]["metrics"]["cyclomatic"], 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_renames_flag_pairs_renamed_file_function() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("calc.ts"),
        "function keep(): number {\n  return 1;\n}\n\nfunction calc(x: number) {\n  return x;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    git(&root, &["mv", "calc.ts", "calc2.ts"]);
    std::fs::write(
        root.join("calc2.ts"),
        "function keep(): number {\n  return 1;\n}\n\nfunction calc(x: number) {\n  if (x > 0) return x;\n  return 0;\n}\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["changed", "--base", "HEAD", "--renames", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let functions = report["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0]["path"], "calc2.ts");
    assert_eq!(functions[0]["name"], "calc");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_help_documents_target_selectors() {
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .args(["--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("--staged"), "missing --staged in:\n{text}");
    assert!(
        text.contains("--target REV"),
        "missing --target REV in:\n{text}"
    );
    assert!(text.contains("--renames"), "missing --renames in:\n{text}");
}

#[test]
fn check_regressions_allows_regression_only_mode() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return x; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["check", ".", "--base", "HEAD", "--regressions", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["files"][0]["functions"][0]["name"], "calc");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_regressions_requires_base_revision() {
    let root = temporary_directory();
    let file = root.join("calc.ts");
    std::fs::write(
        &file,
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    let output = check(&file, &["--cyclomatic", "100", "--regressions"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires --base"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_regressions_reports_delta_only_findings_in_sarif() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("leadline.toml"),
        "[regressions]\ncognitive = 0\ncyclomatic = 100\nmax_nesting = 100\ncrap = 100.0\n",
    )
    .unwrap();
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return x; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args([
            "check",
            ".",
            "--base",
            "HEAD",
            "--regressions",
            "--cyclomatic",
            "100",
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();
    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["ruleId"], "leadline/cognitive");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_regressions_applies_coverage_for_crap_delta() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("leadline.toml"),
        "[regressions]\ncognitive = 100\ncyclomatic = 100\nmax_nesting = 100\ncrap = 0.0\n",
    )
    .unwrap();
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return x; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("coverage.info"),
        "TN:\nSF:calc.ts\nDA:1,1\nend_of_record\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args([
            "check",
            ".",
            "--base",
            "HEAD",
            "--regressions",
            "--coverage",
            "coverage.info",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_regressions_uses_configured_allowed_delta() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("leadline.toml"),
        "[regressions]\ncognitive = 1\ncyclomatic = 1\nmax_nesting = 1\ncrap = 0.0\n",
    )
    .unwrap();
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return x; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["check", ".", "--base", "HEAD", "--regressions", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_regressions_combines_absolute_and_delta_gates() {
    let root = temporary_directory();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return x; }\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args([
            "check",
            ".",
            "--base",
            "HEAD",
            "--regressions",
            "--cyclomatic",
            "100",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["files"][0]["functions"][0]["name"], "calc");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_targets_requires_coverage() {
    let root = temporary_directory();
    let file = root.join("branch.ts");
    std::fs::write(
        &file,
        "function branch(x: boolean) { if (x) return 1; return 0; }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("test-targets")
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .to_ascii_lowercase()
            .contains("coverage")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_targets_lists_uncovered_and_caps_output() {
    let root = temporary_directory();
    std::fs::write(
        root.join("a.ts"),
        "function alpha(x: boolean) { if (x) return 1; return 0; }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("b.ts"),
        "function beta(x: boolean) { if (x) return 2; return 0; }\n",
    )
    .unwrap();
    let lcov = root.join("lcov.info");
    std::fs::write(
        &lcov,
        "TN:\nSF:a.ts\nDA:1,0\nend_of_record\nTN:\nSF:b.ts\nDA:1,0\nend_of_record\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("test-targets")
        .arg(&root)
        .arg("--coverage")
        .arg(&lcov)
        .args(["--format", "agent-json", "--top", "1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let targets = value["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(value["total"], 2);
    assert_eq!(value["truncated"], true);
    assert!(!targets[0]["uncovered"].as_array().unwrap().is_empty());
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

#[test]
fn baseline_requires_an_output_file() {
    let root = temporary_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("baseline")
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--output"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn baseline_write_and_check_regression_gate() {
    let root = temporary_directory();
    let file = root.join("calc.ts");
    std::fs::write(&file, "function calc(x: boolean) { return 1; }\n").unwrap();
    let snapshot = root.join("baseline.json");

    let written = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("baseline")
        .arg(&root)
        .arg("--output")
        .arg(&snapshot)
        .output()
        .unwrap();
    assert!(
        written.status.success(),
        "{}",
        String::from_utf8_lossy(&written.stderr)
    );
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&snapshot).unwrap()).unwrap();
    assert_eq!(stored["schema_version"], 1);
    assert_eq!(stored["metric_profile"], "default-v1");
    assert_eq!(stored["functions"][0]["name"], "calc");

    std::fs::write(
        &file,
        "function calc(x: boolean) { if (x) { if (!x) { return 2; } return 1; } return 0; }\n",
    )
    .unwrap();
    let failed = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("check")
        .arg(&root)
        .arg("--baseline")
        .arg(&snapshot)
        .args(["--regressions", "--json"])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(report["files"][0]["functions"][0]["name"], "calc");

    std::fs::write(
        root.join("leadline.toml"),
        "[regressions]\ncognitive = 100\ncyclomatic = 100\nmax_nesting = 100\ncrap = 100.0\n",
    )
    .unwrap();
    let passed = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("check")
        .arg(&root)
        .arg("--baseline")
        .arg(&snapshot)
        .args(["--regressions", "--json"])
        .output()
        .unwrap();
    assert!(
        passed.status.success(),
        "{}",
        String::from_utf8_lossy(&passed.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn baseline_new_function_fails_only_on_absolute_thresholds() {
    let root = temporary_directory();
    let file = root.join("calc.ts");
    std::fs::write(&file, "function calc(x: boolean) { return 1; }\n").unwrap();
    let snapshot = root.join("baseline.json");
    let written = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("baseline")
        .arg(&root)
        .arg("--output")
        .arg(&snapshot)
        .output()
        .unwrap();
    assert!(
        written.status.success(),
        "{}",
        String::from_utf8_lossy(&written.stderr)
    );

    std::fs::write(
        &file,
        "function calc(x: boolean) { return 1; }\nfunction fresh() { return 2; }\n",
    )
    .unwrap();
    let passed = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("check")
        .arg(&root)
        .arg("--baseline")
        .arg(&snapshot)
        .args(["--regressions", "--json"])
        .output()
        .unwrap();
    assert!(
        passed.status.success(),
        "{}",
        String::from_utf8_lossy(&passed.stderr)
    );

    let failed = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("check")
        .arg(&root)
        .arg("--baseline")
        .arg(&snapshot)
        .args(["--cognitive", "0", "--cyclomatic", "0", "--json"])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_rejects_base_and_baseline_together() {
    let root = temporary_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("check")
        .arg(&root)
        .args([
            "--base",
            "HEAD",
            "--baseline",
            "snapshot.json",
            "--regressions",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("exclusive"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn help_lists_new_workflows() {
    let help = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    for expected in [
        "leadline baseline",
        "--baseline FILE",
        "--output FILE",
        "--regressions",
        "--staged",
        "--target REV",
        "--renames",
        "--explain",
        "leadline test-targets",
    ] {
        assert!(text.contains(expected), "help is missing {expected}");
    }
}
