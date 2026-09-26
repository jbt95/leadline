use std::path::{Path, PathBuf};
use std::process::Command;
mod common;
use common::temporary_directory;

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
    let output = common::leadline().args(["check", "."]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires at least one"));
}

#[test]
fn check_reads_thresholds_from_project_config() {
    let root = temporary_directory();
    std::fs::write(
        root.join("leadline.toml"),
        "[thresholds.function]\ncyclomatic = 1\n",
    )
    .unwrap();
    std::fs::write(
        root.join("branch.ts"),
        "function branch(x: boolean) { if (x) return 1; return 0; }\n",
    )
    .unwrap();

    // No flags: the project config alone must satisfy the threshold check and
    // gate the violating function.
    let output = common::leadline()
        .current_dir(&root)
        .args(["check", ".", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["files"][0]["functions"][0]["name"], "branch");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn version_and_subcommand_help_are_available() {
    let version = common::leadline().arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        concat!("leadline ", env!("CARGO_PKG_VERSION"))
    );

    let help = common::leadline()
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
        let output = common::leadline().args(args).output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
}

#[test]
fn embedded_skill_matches_security_contract() {
    let skill = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("integrations/common/leadline-skill/SKILL.md"),
    )
    .unwrap();
    for term in [
        "security",
        "security_findings",
        "--baseline-sarif",
        "--new-only",
    ] {
        assert!(skill.contains(term), "skill is missing {term}");
    }
    assert!(
        skill.contains("never include") || skill.contains("omitted"),
        "skill must state that scanner messages/source are omitted"
    );
    let help = common::leadline().arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&help.stdout);
    for term in ["security", "--baseline-sarif", "--new-only"] {
        assert!(help.contains(term), "help is missing {term}");
    }
}

#[test]
fn mcp_serves_tool_list_over_stdio() {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = common::leadline()
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
    assert!(names.contains(&"repo_summary"));
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

    let mut child = common::leadline()
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
    common::leadline()
        .arg("check")
        .arg(file)
        .args(options)
        .output()
        .unwrap()
}

#[test]
fn version_subcommand_matches_version_flag() {
    let subcommand = common::leadline().arg("version").output().unwrap();
    assert!(subcommand.status.success());
    assert_eq!(
        String::from_utf8(subcommand.stdout).unwrap().trim(),
        concat!("leadline ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_command_is_usage_error_with_clean_stdout() {
    let output = common::leadline().args(["frobnicate"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("leadline: unknown command"));
}

#[test]
fn json_and_agent_json_together_is_usage_error() {
    let root = temporary_directory();
    let file = root.join("tiny.ts");
    std::fs::write(&file, "function tiny() { return 1; }\n").unwrap();
    let output = common::leadline()
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
    let output = common::leadline()
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
    let missing = common::leadline()
        .args(["analyze", "does-not-exist-xyz"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(3));

    let root = temporary_directory();
    let empty = common::leadline()
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

    let unreadable = common::leadline()
        .arg("analyze")
        .arg(&file)
        .arg("--lcov")
        .arg(&missing)
        .output()
        .unwrap();
    assert_eq!(unreadable.status.code(), Some(4));

    let unknown_format = common::leadline()
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
    let malformed = common::leadline()
        .arg("analyze")
        .arg(&file)
        .arg("--jacoco")
        .arg(&bad_xml)
        .output()
        .unwrap();
    assert_eq!(malformed.status.code(), Some(4));

    let lcov = root.join("cov.info");
    std::fs::write(&lcov, "TN:\nSF:tiny.ts\nDA:1,1\nend_of_record\n").unwrap();
    let detected = common::leadline()
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
    let output = common::leadline()
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
        "parser: zig OK",
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
    let output = common::leadline()
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
    let output = common::leadline()
        .arg("analyze")
        .arg(&root)
        .args(["--format", "agent-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["metric_profile"], "default");
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
    let output = common::leadline()
        .current_dir(&root)
        .args(["changed", "--base", "HEAD", "--format", "agent-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["changed_functions"], 1);
    assert!(value["regressions"][0].get("causes").is_none());

    let explained = common::leadline()
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
    let failing = common::leadline()
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
    let passing = common::leadline()
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
    let output = common::leadline()
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
    let budgeted = common::leadline()
        .arg("function")
        .arg(&file)
        .arg("pick")
        .args(["--format", "agent-json", "--explain"])
        .output()
        .unwrap();
    assert!(budgeted.status.success());
    let value: serde_json::Value = serde_json::from_slice(&budgeted.stdout).unwrap();
    assert!(value["files"][0]["functions"][0]["contributions"].is_array());
    let plain = common::leadline()
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
    let output = common::leadline()
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
    let conflict = common::leadline()
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
    let output = common::leadline()
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
fn analyze_index_output_is_identical_to_a_cold_run() {
    let root = temporary_directory();
    std::fs::write(root.join("sample.ts"), "function alpha() { return 1; }\n").unwrap();
    let index = root.join("index");
    let run = |index: Option<&Path>| {
        let mut command = common::leadline();
        command.arg("analyze").arg(&root).arg("--json");
        if let Some(index) = index {
            command.arg("--index").arg(index);
        }
        command.output().unwrap()
    };

    let cold = run(None);
    assert!(cold.status.success());
    let first = run(Some(&index));
    assert!(first.status.success());
    assert!(index.join("index.json").is_file());
    let second = run(Some(&index));
    assert!(second.status.success());

    assert_eq!(
        cold.stdout, first.stdout,
        "warm output must equal cold output"
    );
    assert_eq!(
        first.stdout, second.stdout,
        "repeated warm output must equal itself"
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analyze_file_reuses_repository_index_without_clobbering_it() {
    let root = index_fixture();
    assert!(
        common::leadline()
            .arg("index")
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let index_file = root.join(".leadline").join("index.json");
    let file = root.join("alpha.ts");
    let cold = common::leadline()
        .arg("analyze")
        .arg(&file)
        .arg("--json")
        .output()
        .unwrap();
    assert!(cold.status.success());

    let warm = common::leadline()
        .arg("analyze")
        .arg(&file)
        .arg("--index")
        .arg(root.join(".leadline"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(warm.status.success());
    assert_eq!(
        warm.stdout, cold.stdout,
        "warm file output must equal cold output"
    );
    assert!(
        String::from_utf8_lossy(&warm.stderr).contains("1 reused"),
        "expected reuse, got: {}",
        String::from_utf8_lossy(&warm.stderr)
    );

    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&index_file).unwrap()).unwrap();
    assert_eq!(
        after["files"].as_object().unwrap().len(),
        2,
        "file-target warm must not clobber the repository index"
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_dir_is_no_longer_accepted() {
    let output = common::leadline()
        .args(["analyze", ".", "--cache-dir", ".leadline"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--cache-dir"));
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
    let output = common::leadline()
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

    let explicit = common::leadline()
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
    let output = common::leadline()
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
    let output = common::leadline()
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
    let output = common::leadline()
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
    let output = common::leadline()
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
    let output = common::leadline().args(["--help"]).output().unwrap();
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
    let output = common::leadline()
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
    let output = common::leadline()
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
    // Exactly one dimension regressed, so no other rule may appear from a
    // synthetic zero threshold, and the message carries the real delta.
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["ruleId"], "leadline/cognitive");
    let message = results[0]["message"]["text"].as_str().unwrap();
    assert!(message.contains("exceeds allowed delta 0"), "{message}");
    assert!(message.contains("cognitive complexity 0 -> 1"), "{message}");
    assert!(!message.contains("exceeds limit 0"), "{message}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_sarif_projects_scanner_gate_violations_only() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/risky.ts"),
        "function risky(x: number): number { if (x > 0) return 1; return 0; }\n",
    )
    .unwrap();
    std::fs::write(root.join("risky.sql"), "UPDATE users SET active = false;\n").unwrap();
    // SQL findings are informational without a gate, so SARIF stays empty
    // even though the report has findings.
    let output = common::leadline()
        .args([
            "check",
            root.to_str().unwrap(),
            "--cyclomatic",
            "100",
            "--sql",
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        sarif["runs"][0]["results"].as_array().unwrap().is_empty(),
        "{sarif}"
    );
    // With the gate on, only the failing family reaches SARIF.
    let output = common::leadline()
        .args([
            "check",
            root.to_str().unwrap(),
            "--cyclomatic",
            "100",
            "--sql",
            "--sql-fail-on-severity",
            "low",
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["ruleId"], "sql/update-delete-without-where");
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
    let output = common::leadline()
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
    let output = common::leadline()
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
    let output = common::leadline()
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
    let output = common::leadline()
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
    let output = common::leadline()
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
    let output = common::leadline()
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

    let written = common::leadline()
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
    assert_eq!(stored["metric_profile"], "default");
    assert_eq!(stored["functions"][0]["name"], "calc");

    std::fs::write(
        &file,
        "function calc(x: boolean) { if (x) { if (!x) { return 2; } return 1; } return 0; }\n",
    )
    .unwrap();
    let failed = common::leadline()
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
    let passed = common::leadline()
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

    let index = root.join("index");
    let warm = common::leadline()
        .arg("check")
        .arg(&root)
        .arg("--baseline")
        .arg(&snapshot)
        .args(["--regressions", "--json"])
        .arg("--index")
        .arg(&index)
        .output()
        .unwrap();
    assert!(
        warm.status.success(),
        "{}",
        String::from_utf8_lossy(&warm.stderr)
    );
    assert_eq!(warm.stdout, passed.stdout);
    assert!(index.join("index.json").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn baseline_new_function_fails_only_on_absolute_thresholds() {
    let root = temporary_directory();
    let file = root.join("calc.ts");
    std::fs::write(&file, "function calc(x: boolean) { return 1; }\n").unwrap();
    let snapshot = root.join("baseline.json");
    let written = common::leadline()
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
    let passed = common::leadline()
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

    let failed = common::leadline()
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
    let output = common::leadline()
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
    let help = common::leadline().arg("--help").output().unwrap();
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

fn sql_plan_dirs(cost_current: f64, cost_baseline: f64) -> (PathBuf, PathBuf, PathBuf) {
    let root = temporary_directory();
    let current = root.join("current");
    let baseline = root.join("baseline");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&baseline).unwrap();
    std::fs::write(
        current.join("q.json"),
        format!(r#"[{{"Plan": {{"Node Type": "Seq Scan", "Relation Name": "t", "Total Cost": {cost_current}, "Plan Rows": 10}}}}]"#),
    )
    .unwrap();
    std::fs::write(
        baseline.join("q.json"),
        format!(r#"[{{"Plan": {{"Node Type": "Seq Scan", "Relation Name": "t", "Total Cost": {cost_baseline}, "Plan Rows": 10}}}}]"#),
    )
    .unwrap();
    (root, current, baseline)
}

#[test]
fn sql_plan_cost_gate_retains_json() {
    let (root, current, baseline) = sql_plan_dirs(150.0, 100.0);
    let output = common::leadline()
        .args([
            "sql-plan",
            "--current",
            &current.display().to_string(),
            "--baseline",
            &baseline.display().to_string(),
            "--max-cost-increase-percent",
            "25",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["violations"][0]["kind"], "cost_increase");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_plan_requires_directories_and_finite_limits() {
    let missing = common::leadline()
        .args(["sql-plan", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    let (root, current, baseline) = sql_plan_dirs(10.0, 10.0);
    for flag in [
        "--max-cost-increase-percent",
        "--max-plan-rows-ratio",
        "--max-estimate-error-ratio",
    ] {
        for bad in ["-1", "nan", "inf"] {
            let output = common::leadline()
                .args([
                    "sql-plan",
                    "--current",
                    &current.display().to_string(),
                    "--baseline",
                    &baseline.display().to_string(),
                    flag,
                    bad,
                ])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{flag} {bad}");
        }
    }
    let unknown = common::leadline()
        .args([
            "sql-plan",
            "--current",
            &current.display().to_string(),
            "--baseline",
            &baseline.display().to_string(),
            "--bogus",
        ])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_plan_agent_json_sarif_and_determinism() {
    let (root, current, baseline) = sql_plan_dirs(150.0, 100.0);
    let agent = common::leadline()
        .args([
            "sql-plan",
            "--current",
            &current.display().to_string(),
            "--baseline",
            &baseline.display().to_string(),
            "--max-cost-increase-percent",
            "25",
            "--format",
            "agent-json",
            "--top",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(agent.status.code(), Some(1));
    let body: serde_json::Value = serde_json::from_slice(&agent.stdout).unwrap();
    assert_eq!(body["truncated"], false);
    assert!(body.get("violations").is_some());
    let sarif = common::leadline()
        .args([
            "sql-plan",
            "--current",
            &current.display().to_string(),
            "--baseline",
            &baseline.display().to_string(),
            "--max-cost-increase-percent",
            "25",
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();
    assert_eq!(sarif.status.code(), Some(1));
    let document: serde_json::Value = serde_json::from_slice(&sarif.stdout).unwrap();
    let results = document["runs"][0]["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert!(
        results[0]["ruleId"]
            .as_str()
            .unwrap()
            .starts_with("postgresql-plan/")
    );
    let first = common::leadline()
        .args([
            "sql-plan",
            "--current",
            &current.display().to_string(),
            "--baseline",
            &baseline.display().to_string(),
            "--json",
        ])
        .output()
        .unwrap();
    let second = common::leadline()
        .args([
            "sql-plan",
            "--current",
            &current.display().to_string(),
            "--baseline",
            &baseline.display().to_string(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(
        first.status.code(),
        Some(0),
        "no limits means index-clean plans pass"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_plan_malformed_input_is_input_error() {
    let root = temporary_directory();
    let current = root.join("current");
    let baseline = root.join("baseline");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&baseline).unwrap();
    std::fs::write(current.join("q.json"), b"not json").unwrap();
    std::fs::write(
        baseline.join("q.json"),
        "[{\"Plan\": {\"Node Type\": \"Seq Scan\"}}]",
    )
    .unwrap();
    let output = common::leadline()
        .args([
            "sql-plan",
            "--current",
            &current.display().to_string(),
            "--baseline",
            &baseline.display().to_string(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    std::fs::remove_dir_all(root).unwrap();
}

const SECURITY_CURRENT: &str = r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "semgrep"}}, "results": [{"ruleId": "sql-injection", "level": "error", "message": {"text": "BAD"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/auth.ts"}, "region": {"startLine": 2}}}], "baselineState": "new"}]}]}"#;
const SECURITY_BASELINE: &str = r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "semgrep"}}, "results": [{"ruleId": "other-rule", "level": "warning", "message": {"text": "OLD"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/other.ts"}, "region": {"startLine": 1}}}]}]}]}"#;

fn security_repo() -> (PathBuf, PathBuf, PathBuf) {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/auth.ts"),
        "function outer(x: number): number {\n  function login(y: number): number {\n    if (y > 0) { return 1; }\n    return 0;\n  }\n  return login(x);\n}\n",
    )
    .unwrap();
    let current = root.join("current.sarif");
    let baseline = root.join("base.sarif");
    std::fs::write(&current, SECURITY_CURRENT).unwrap();
    std::fs::write(&baseline, SECURITY_BASELINE).unwrap();
    (root, current, baseline)
}

#[test]
fn security_json_enriches_and_gates_new_high_findings() {
    let (root, current, baseline) = security_repo();
    let output = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--baseline-sarif",
            baseline.to_str().unwrap(),
            "--fail-on-severity",
            "high",
            "--new-only",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["findings"][0]["state"], "new");
    assert!(report["findings"][0]["function_id"].is_string());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("BAD"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_requires_sarif_and_parses_severity() {
    let missing = common::leadline().args(["security", "."]).output().unwrap();
    assert_eq!(missing.status.code(), Some(2));
    let (root, current, _) = security_repo();
    let bad_level = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--fail-on-severity",
            "bogus",
        ])
        .output()
        .unwrap();
    assert_eq!(bad_level.status.code(), Some(2));
    let unknown = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--bogus",
        ])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    // A missing analysis path is incomplete analysis (exit 3), not a report
    // input error: the CLI keeps its documented exit codes even though the
    // library assembly path is shared with `check` and the MCP tool.
    let missing_path = common::leadline()
        .args([
            "security",
            root.join("does-not-exist").to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(missing_path.status.code(), Some(3));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_rejects_exclusive_comparison_flags() {
    let (root, current, _) = security_repo();
    for extra in [
        vec!["--base", "HEAD", "--staged"],
        vec!["--staged", "--target", "HEAD"],
        vec!["--base", "HEAD", "--target", "HEAD"],
    ] {
        let mut args = vec![
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
        ];
        args.extend(extra.iter().copied());
        let output = common::leadline().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{extra:?}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_changed_only_requires_a_comparison() {
    let (root, current, baseline) = security_repo();
    // Without a comparison every finding stays `changed: null` and the gate
    // would silently pass: that is a usage error, never a silent pass.
    let output = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--baseline-sarif",
            baseline.to_str().unwrap(),
            "--fail-on-severity",
            "high",
            "--changed-only",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--changed-only requires"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_changed_only_narrows_gate_not_report() {
    let (root, current, baseline) = security_repo();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    std::fs::write(root.join("README.md"), "# test\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "second", "-q"]);
    // Nothing staged: the finding is not in changed code, so the gate passes
    // while the report still lists it.
    let args = [
        "security",
        root.to_str().unwrap(),
        "--sarif",
        current.to_str().unwrap(),
        "--baseline-sarif",
        baseline.to_str().unwrap(),
        "--fail-on-severity",
        "high",
        "--changed-only",
        "--staged",
        "--json",
    ];
    let output = common::leadline().args(args).output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!report["findings"].as_array().unwrap().is_empty());
    assert_eq!(report["findings"][0]["changed"], false);
    // Stage the finding's file: the same gate now fails.
    std::fs::write(
        root.join("src/auth.ts"),
        "function outer(x: number): number {\n  function login(y: number): number {\n    if (y > 0) { return 2; }\n    return 0;\n  }\n  return login(x);\n}\n",
    )
    .unwrap();
    git(&root, &["add", "src/auth.ts"]);
    let output = common::leadline().args(args).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["findings"][0]["changed"], true);
    // Below-threshold gates pass while retaining the report.
    let output = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--fail-on-severity",
            "critical",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_subdirectory_scope_matches_analysis_relative_artifact_paths() {
    let (root, _current, _baseline) = security_repo();
    git(&root, &["init"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("src/auth.ts"),
        "function outer(x: number): number {\n  function login(y: number): number {\n    if (y > 0) { return 2; }\n    return 0;\n  }\n  return login(x);\n}\n",
    )
    .unwrap();
    // The scanner ran with `src/` as its root, so its artifact URI is
    // analysis-root-relative; the changed set must be too.
    let scoped = root.join("scoped.sarif");
    std::fs::write(
        &scoped,
        r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "semgrep"}}, "results": [{"ruleId": "sql-injection", "level": "error", "locations": [{"physicalLocation": {"artifactLocation": {"uri": "auth.ts"}, "region": {"startLine": 2}}}]}]}]}"#,
    )
    .unwrap();
    let output = common::leadline()
        .args([
            "security",
            root.join("src").to_str().unwrap(),
            "--sarif",
            scoped.to_str().unwrap(),
            "--base",
            "HEAD",
            "--fail-on-severity",
            "high",
            "--changed-only",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["findings"][0]["changed"], true);
    assert!(report["findings"][0]["function_id"].is_string());
    // The innermost `login` function (cognitive 1) must be attributed, not
    // the enclosing `outer` (cognitive 0).
    assert_eq!(report["findings"][0]["cognitive"], 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_merges_repeated_files_and_renders_all_modes() {
    let (root, current, _) = security_repo();
    let second = root.join("second.sarif");
    std::fs::copy(&current, &second).unwrap();
    let output = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--sarif",
            second.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["findings"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["findings"][0]["report_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let agent = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--format",
            "agent-json",
            "--top",
            "1",
        ])
        .output()
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&agent.stdout).unwrap();
    assert_eq!(body["truncated"], false);
    let sarif = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();
    let document: serde_json::Value = serde_json::from_slice(&sarif.stdout).unwrap();
    let results = document["runs"][0]["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["ruleId"], "semgrep/sql-injection");
    let first = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    let second = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            current.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(first.stdout, second.stdout);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_malformed_sarif_is_input_error() {
    let root = temporary_directory();
    let bad = root.join("bad.sarif");
    std::fs::write(&bad, b"not json").unwrap();
    let output = common::leadline()
        .args([
            "security",
            root.to_str().unwrap(),
            "--sarif",
            bad.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_combines_complexity_and_security_violations() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/complex.ts"),
        "function tangled(x: number): number {\n  if (x > 0) {\n    if (x > 1) {\n      if (x > 2) { return 3; }\n    }\n  }\n  return 0;\n}\n",
    )
    .unwrap();
    let sarif = root.join("findings.sarif");
    std::fs::write(
        &sarif,
        r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "semgrep"}}, "results": [{"ruleId": "sql-injection", "level": "error", "message": {"text": "BAD"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/complex.ts"}, "region": {"startLine": 2}}}]}]}]}"#,
    )
    .unwrap();
    let output = common::leadline()
        .args([
            "check",
            root.to_str().unwrap(),
            "--cognitive",
            "1",
            "--sarif",
            sarif.to_str().unwrap(),
            "--fail-on-severity",
            "low",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!report["files"].as_array().unwrap().is_empty());
    assert!(!report["security_violations"].as_array().unwrap().is_empty());
    // Security-only checks work without metric thresholds and stay silent on stdout gates.
    let output = common::leadline()
        .args([
            "check",
            root.to_str().unwrap(),
            "--sarif",
            sarif.to_str().unwrap(),
            "--fail-on-severity",
            "low",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report["files"].as_array().unwrap().is_empty());
    assert!(!report["security_violations"].as_array().unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

fn vulnerabilities_repo() -> (PathBuf, PathBuf) {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/app.ts"), "import _ from 'lodash';\n").unwrap();
    let osv = root.join("current.json");
    std::fs::copy("tests/fixtures/vulnerabilities/osv.json", &osv).unwrap();
    (root, osv)
}

#[test]
fn vulnerabilities_json_prioritizes_changed_imports() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/app.ts"), "export const x = 1;\n").unwrap();
    for args in [
        &["init"][..],
        &["config", "user.email", "t@e.invalid"][..],
        &["config", "user.name", "T"][..],
        &["add", "."][..],
        &["commit", "-qm", "base"][..],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
    std::fs::write(root.join("src/app.ts"), "import _ from 'lodash';\n").unwrap();
    let osv = root.join("current.json");
    std::fs::copy("tests/fixtures/vulnerabilities/osv.json", &osv).unwrap();
    let output = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--base",
            "HEAD",
            "--fail-on-severity",
            "high",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["findings"][0]["reachable_from_changed"], true);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("DESCRIPTION_SENTINEL"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vulnerabilities_requires_input_and_parses_limits() {
    let missing = common::leadline()
        .args(["vulnerabilities", "."])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    let (root, osv) = vulnerabilities_repo();
    for extra in [
        vec!["--base", "HEAD", "--staged"],
        vec!["--staged", "--target", "HEAD"],
    ] {
        let mut args = vec![
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
        ];
        args.extend(extra.iter().copied());
        let output = common::leadline().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{extra:?}");
    }
    let bad_level = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--fail-on-severity",
            "bogus",
        ])
        .output()
        .unwrap();
    assert_eq!(bad_level.status.code(), Some(2));
    let malformed = root.join("bad.json");
    std::fs::write(&malformed, b"not json").unwrap();
    let output = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            malformed.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vulnerabilities_config_threshold_and_output_modes() {
    let (root, osv) = vulnerabilities_repo();
    std::fs::write(
        root.join("leadline.toml"),
        "[vulnerabilities]\nminimum_severity = 'critical'\n",
    )
    .unwrap();
    // Config floor without a CLI flag: high finding stays informational.
    let output = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    // CLI flag overrides config for one invocation.
    let output = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--fail-on-severity",
            "low",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let agent = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--format",
            "agent-json",
            "--top",
            "1",
        ])
        .output()
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&agent.stdout).unwrap();
    assert_eq!(body["truncated"], false);
    let sarif = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();
    let document: serde_json::Value = serde_json::from_slice(&sarif.stdout).unwrap();
    let results = document["runs"][0]["results"].as_array().unwrap();
    assert_eq!(
        results[0]["ruleId"],
        "vulnerability/npm/GHSA-xxxx-yyyy-zzzz"
    );
    let first = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    let second = common::leadline()
        .args([
            "vulnerabilities",
            root.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(first.stdout, second.stdout);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_vulnerabilities_combines_both_families() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/complex.ts"),
        "function tangled(x: number): number {\n  if (x > 0) {\n    if (x > 1) {\n      if (x > 2) { return 3; }\n    }\n  }\n  return 0;\n}\n",
    )
    .unwrap();
    let sarif = root.join("findings.sarif");
    std::fs::write(
        &sarif,
        r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "semgrep"}}, "results": [{"ruleId": "r", "level": "error", "message": {"text": "x"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/complex.ts"}, "region": {"startLine": 2}}}]}]}]}"#,
    )
    .unwrap();
    let osv = root.join("current.json");
    std::fs::copy("tests/fixtures/vulnerabilities/osv.json", &osv).unwrap();
    let output = common::leadline()
        .args([
            "check",
            root.to_str().unwrap(),
            "--cognitive",
            "1",
            "--sarif",
            sarif.to_str().unwrap(),
            "--osv",
            osv.to_str().unwrap(),
            "--fail-on-severity",
            "low",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!report["files"].as_array().unwrap().is_empty());
    assert!(!report["security_violations"].as_array().unwrap().is_empty());
    assert!(
        !report["vulnerability_violations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_risk_json_gates_high_findings() {
    let output = common::leadline()
        .args([
            "sql",
            "tests/fixtures/sql-risk",
            "--fail-on-severity",
            "high",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["findings"][0]["rule_id"],
        "sql/update-delete-without-where"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("LITERAL_SENTINEL"));
}

#[test]
fn sql_risk_requires_no_input_and_parses_flags() {
    // No input flags needed: the analysis path may simply contain no SQL.
    let empty = temporary_directory();
    let output = common::leadline()
        .args(["sql", empty.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let bad_level = common::leadline()
        .args([
            "sql",
            "tests/fixtures/sql-risk",
            "--fail-on-severity",
            "bogus",
        ])
        .output()
        .unwrap();
    assert_eq!(bad_level.status.code(), Some(2));
    let unknown = common::leadline()
        .args(["sql", "tests/fixtures/sql-risk", "--bogus"])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    std::fs::remove_dir_all(empty).unwrap();
}

#[test]
fn sql_risk_config_threshold_modes_and_determinism() {
    let root = temporary_directory();
    std::fs::write(
        root.join("q.sql"),
        "SELECT * FROM t ORDER BY id LIMIT 1 OFFSET 600;\n",
    )
    .unwrap();
    std::fs::write(root.join("leadline.toml"), "[sql]\nlarge_offset = 500\n").unwrap();
    // Config floor without a flag stays informational.
    let output = common::leadline()
        .args(["sql", root.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    // The configured threshold fires under a gate.
    let output = common::leadline()
        .args([
            "sql",
            root.to_str().unwrap(),
            "--fail-on-severity",
            "medium",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["rule_id"] == "sql/large-offset")
    );
    // Default threshold would pass the same file: CLI flag wins over config.
    let output = common::leadline()
        .args([
            "sql",
            root.to_str().unwrap(),
            "--large-offset",
            "1000",
            "--fail-on-severity",
            "medium",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let agent = common::leadline()
        .args([
            "sql",
            root.to_str().unwrap(),
            "--format",
            "agent-json",
            "--top",
            "1",
        ])
        .output()
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&agent.stdout).unwrap();
    assert_eq!(body["truncated"], false);
    let sarif = common::leadline()
        .args(["sql", root.to_str().unwrap(), "--format", "sarif"])
        .output()
        .unwrap();
    let document: serde_json::Value = serde_json::from_slice(&sarif.stdout).unwrap();
    let results = document["runs"][0]["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["ruleId"], "sql/large-offset");
    let first = common::leadline()
        .args(["sql", root.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    let second = common::leadline()
        .args(["sql", root.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert_eq!(first.stdout, second.stdout);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_risk_malformed_sql_is_input_error() {
    let root = temporary_directory();
    std::fs::write(root.join("bad.sql"), b"SELECT 'oops;").unwrap();
    let output = common::leadline()
        .args(["sql", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_sql_combines_both_families() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/complex.ts"),
        "function tangled(x: number): number {\n  if (x > 0) {\n    if (x > 1) {\n      if (x > 2) { return 3; }\n    }\n  }\n  return 0;\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("risky.sql"), "UPDATE users SET active = false;\n").unwrap();
    let output = common::leadline()
        .args([
            "check",
            root.to_str().unwrap(),
            "--cognitive",
            "1",
            "--sql",
            "--sql-fail-on-severity",
            "low",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!report["files"].as_array().unwrap().is_empty());
    assert!(!report["sql_violations"].as_array().unwrap().is_empty());
    // Without the gate flag, SQL findings stay informational.
    let output = common::leadline()
        .args([
            "check",
            root.to_str().unwrap(),
            "--cognitive",
            "100",
            "--sql",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    std::fs::remove_dir_all(root).unwrap();
}

fn index_fixture() -> PathBuf {
    let root = temporary_directory();
    std::fs::write(
        root.join("alpha.ts"),
        "function alpha(a: number) { return a + 1; }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("beta.ts"),
        "function beta(b: number) { let t = 0; for (let i = 0; i < b; i++) { t += i; } return t; }\n",
    )
    .unwrap();
    root
}

#[test]
fn index_builds_and_reuses_unchanged_files() {
    let root = index_fixture();
    let run = |extra: &[&str]| {
        common::leadline()
            .arg("index")
            .arg(&root)
            .args(extra)
            .output()
            .unwrap()
    };

    let first = run(&["--json"]);
    assert!(first.status.success());
    let first_json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first_json["index"]["files"], 2);
    assert_eq!(first_json["index"]["analyzed"], 2);
    assert_eq!(first_json["index"]["reused"], 0);

    let second = run(&["--json"]);
    assert!(second.status.success());
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_json["index"]["analyzed"], 0);
    assert_eq!(second_json["index"]["reused"], 2);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn index_verify_detects_a_tampered_index() {
    let root = index_fixture();
    assert!(
        common::leadline()
            .arg("index")
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );

    let path = root.join(".leadline").join("index.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let first_key = json["files"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    json["files"][&first_key]["key"] = serde_json::json!("tampered");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

    let verified = common::leadline()
        .arg("index")
        .arg(&root)
        .arg("--verify")
        .arg("--json")
        .output()
        .unwrap();
    assert!(verified.status.success());
    let verified_json: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(verified_json["index"]["verified"], false);

    // --verify must not repair or overwrite the tampered index.
    let still_tampered: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(still_tampered["files"][&first_key]["key"], "tampered");

    let second = common::leadline()
        .arg("index")
        .arg(&root)
        .arg("--verify")
        .arg("--json")
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_json["index"]["verified"], false);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn index_verify_reports_true_for_an_untampered_index() {
    let root = index_fixture();
    assert!(
        common::leadline()
            .arg("index")
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );

    let verified = common::leadline()
        .arg("index")
        .arg(&root)
        .arg("--verify")
        .arg("--json")
        .output()
        .unwrap();
    assert!(verified.status.success());
    let json: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(json["index"]["verified"], true);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn help_lists_the_index_command() {
    let output = common::leadline().arg("--help").output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("index [PATH]"));
}

/// `report` re-renders a saved `check --json` document without re-analyzing,
/// so the saved metrics and the thresholds `report` is given decide the
/// output: the document alone names no violation until a limit is applied.
#[test]
fn report_renders_a_saved_check_document() {
    let root = temporary_directory();
    let file = root.join("calc.ts");
    std::fs::write(
        &file,
        "function calc(x: number) { if (x > 0) { if (x > 1) { return x; } } return 0; }\n",
    )
    .unwrap();
    let saved = check(&file, &["--cognitive", "1", "--json"]);
    assert_eq!(saved.status.code(), Some(1), "fixture must violate");
    let document = root.join("check.json");
    std::fs::write(&document, &saved.stdout).unwrap();
    let from = document.to_str().unwrap().to_owned();

    // No limit means nothing to compare against, so the document passes.
    let unfiltered = common::leadline()
        .args(["report", "--from", &from, "--format", "markdown"])
        .output()
        .unwrap();
    assert!(unfiltered.status.success());
    let rendered = String::from_utf8_lossy(&unfiltered.stdout);
    assert!(
        rendered.contains("**passing** - 0 violations in 1 file"),
        "{rendered}"
    );

    // The same document fails once a limit the saved metrics exceed is applied,
    // which is what makes the saved JSON worth re-rendering.
    let failing = common::leadline()
        .args([
            "report",
            "--from",
            &from,
            "--format",
            "github-annotations",
            "--cognitive",
            "1",
        ])
        .output()
        .unwrap();
    assert!(failing.status.success());
    let annotation = String::from_utf8_lossy(&failing.stdout);
    let annotation = annotation.trim();
    assert!(annotation.starts_with("::warning file="), "{annotation}");
    assert!(annotation.contains("calc.ts"), "{annotation}");
    assert!(
        annotation.ends_with(",line=1,title=function::exceeds cognitive"),
        "{annotation}"
    );

    std::fs::remove_dir_all(root).unwrap();
}

/// Every `report` argument error must be a usage error naming the option, not
/// a panic or a silent empty render.
#[test]
fn report_rejects_bad_arguments_with_usage_errors() {
    let root = temporary_directory();
    let missing = root.join("absent.json");
    let not_json = root.join("plain.json");
    std::fs::write(&not_json, b"not json").unwrap();
    let absent = missing.to_str().unwrap().to_owned();
    let plain = not_json.to_str().unwrap().to_owned();

    for (args, expected) in [
        (vec!["report"], "report requires --from FILE"),
        (
            vec!["report", "--from", &absent],
            "report requires --format NAME",
        ),
        (vec!["report", "--from"], "--from requires a file"),
        (vec!["report", "--format"], "--format requires a value"),
        (vec!["report", "--bogus"], "unknown report option '--bogus'"),
        (
            vec!["report", "--from", &absent, "--format", "nope"],
            "unknown --format 'nope'",
        ),
        (
            vec!["report", "--from", &absent, "--format", "badge"],
            "cannot read",
        ),
        (
            vec!["report", "--from", &plain, "--format", "badge"],
            "is not JSON",
        ),
    ] {
        let output = common::leadline().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
    }

    std::fs::remove_dir_all(root).unwrap();
}

/// The `unused` subcommand is the reachability report. No test drove it
/// through the binary, so its argument parsing, config loading, and terminal
/// rendering were unexercised.
#[test]
fn unused_reports_unreachable_files_through_the_cli() {
    let root = temporary_directory();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("entry.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(src.join("orphan.ts"), "export const b = 2;\n").unwrap();

    let output = common::leadline()
        .args(["unused", root.to_str().unwrap(), "--entry", "src/entry.ts"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(
        rendered.contains("src/orphan.ts"),
        "an unreachable file must be listed: {rendered}"
    );

    let json = common::leadline()
        .args([
            "unused",
            root.to_str().unwrap(),
            "--entry",
            "src/entry.ts",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["complete"], true);
    assert_eq!(value["files_analyzed"], 2);
    assert_eq!(
        value["entry_points"],
        serde_json::json!([{"path": "src/entry.ts", "source": "entry"}])
    );
    assert_eq!(
        value["unused_files"],
        serde_json::json!([{"path": "src/orphan.ts"}])
    );

    // `--json` and `--format` are mutually exclusive across every command.
    let exclusive = common::leadline()
        .args([
            "unused",
            root.to_str().unwrap(),
            "--json",
            "--format",
            "agent-json",
        ])
        .output()
        .unwrap();
    assert_eq!(exclusive.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&exclusive.stderr).contains("exclusive"));

    let unknown = common::leadline()
        .args(["unused", root.to_str().unwrap(), "--bogus"])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown unused option"));

    let needs_pattern = common::leadline()
        .args(["unused", root.to_str().unwrap(), "--entry"])
        .output()
        .unwrap();
    assert_eq!(needs_pattern.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&needs_pattern.stderr).contains("--entry requires a pattern"));

    std::fs::remove_dir_all(root).unwrap();
}
