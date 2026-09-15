use leadline::security::{
    FindingState, SecurityFinding, SecurityGate, SecurityInputs, SecurityReport, SecuritySeverity,
    enrich, read_security_reports,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
mod common;
use common::{temporary_directory, write_temp};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from("tests/fixtures/security").join(name)
}

#[test]
fn normalizes_sarif_security_fields_without_messages() {
    let current = [fixture("basic.sarif")];
    let report = read_security_reports(SecurityInputs {
        current: &current,
        baseline: &[],
    })
    .unwrap();
    // Numeric result-level severity wins over the rule-level value and the
    // SARIF level, and sorts first.
    assert_eq!(report.findings[0].severity, SecuritySeverity::Critical);
    assert_eq!(report.findings[0].path.as_deref(), Some("src/auth.ts"));
    assert_eq!(report.findings[0].state, FindingState::New);
    assert_eq!(report.findings[0].report_ids, vec!["basic.sarif"]);
    // Lexicographically first partial fingerprint wins.
    assert_eq!(report.findings[0].fingerprint, "abc123");
    // Windows separators normalize; level-only findings map note to low.
    let win = report
        .findings
        .iter()
        .find(|finding| finding.path.as_deref() == Some("src/win.ts"))
        .unwrap();
    assert_eq!(win.severity, SecuritySeverity::Critical);
    let low = report
        .findings
        .iter()
        .find(|finding| finding.rule_id == "low-info")
        .unwrap();
    assert_eq!(low.severity, SecuritySeverity::Low);
    // Location-free findings keep a null path; messages never serialize.
    let pathless = report
        .findings
        .iter()
        .find(|finding| finding.rule_id == "generic-api-key")
        .unwrap();
    assert_eq!(pathless.path, None);
    assert_eq!(pathless.fingerprint, "fp-first");
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(!serialized.contains("SECRET_SENTINEL"));
}

#[test]
fn deduplicates_by_semantic_key_and_merges_report_ids() {
    let root = temporary_directory();
    let body = r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "t"}}, "results": [{"ruleId": "r", "level": "error", "message": {"text": "x"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/a.ts"}, "region": {"startLine": 1}}}]}]}]}"#;
    let first = write_temp(&root, "a.sarif", body.as_bytes());
    let second = write_temp(&root, "b.sarif", body.as_bytes());
    let current = [first, second];
    let report = read_security_reports(SecurityInputs {
        current: &current,
        baseline: &[],
    })
    .unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].report_ids, vec!["a.sarif", "b.sarif"]);
    // Distinct spans without scanner fingerprints stay distinct.
    let other = r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "t"}}, "results": [{"ruleId": "r", "level": "error", "message": {"text": "x"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/a.ts"}, "region": {"startLine": 2}}}]}]}]}"#;
    let third = write_temp(&root, "c.sarif", other.as_bytes());
    let current = [third];
    let report = read_security_reports(SecurityInputs {
        current: &current,
        baseline: &[],
    })
    .unwrap();
    assert_eq!(report.findings.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn classifies_against_baselines() {
    let root = temporary_directory();
    let body = r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "t"}}, "results": [{"ruleId": "r", "level": "error", "message": {"text": "x"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/a.ts"}, "region": {"startLine": 1}}}]}]}]}"#;
    let base = write_temp(&root, "base.sarif", body.as_bytes());
    let current = [fixture("basic.sarif")];
    let report = read_security_reports(SecurityInputs {
        current: &current,
        baseline: std::slice::from_ref(&base),
    })
    .unwrap();
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.state == FindingState::New),
        "fixture keys are absent from the unrelated baseline"
    );
    let same = [base.clone()];
    let report = read_security_reports(SecurityInputs {
        current: &same,
        baseline: &[base],
    })
    .unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].state, FindingState::Existing);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_absolute_parent_and_non_file_uris() {
    let root = temporary_directory();
    for (name, uri) in [
        ("absolute.sarif", "/etc/passwd"),
        ("parent.sarif", "../../etc/passwd"),
        ("drive.sarif", "C:/win/secret.ts"),
        ("https.sarif", "https://example.com/findings"),
        ("data.sarif", "data:application/sarif+json,eyJ9"),
        ("file-absolute.sarif", "file:///etc/passwd"),
        ("file-authority.sarif", "file://host/share.ts"),
        ("file-localhost.sarif", "file://localhost/src/a.ts"),
    ] {
        let body = format!(
            r#"{{"version": "2.1.0", "runs": [{{"tool": {{"driver": {{"name": "t"}}}}, "results": [{{"ruleId": "r", "level": "error", "message": {{"text": "x"}}, "locations": [{{"physicalLocation": {{"artifactLocation": {{"uri": "{uri}"}}}}}}]}}]}}]}}"#
        );
        let file = write_temp(&root, name, body.as_bytes());
        assert!(
            read_security_reports(SecurityInputs {
                current: &[file],
                baseline: &[],
            })
            .is_err(),
            "{uri} must be rejected"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reads_object_shaped_fingerprints_and_classifies_moved_findings() {
    let root = temporary_directory();
    // SARIF 2.1.0 `fingerprints` is an object property bag: a scanner
    // fingerprint must classify a moved finding as existing rather than
    // re-reporting it through the span fallback.
    let current = r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "t"}}, "results": [{"ruleId": "r", "level": "error", "message": {"text": "x"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/a.ts"}, "region": {"startLine": 2}}}], "fingerprints": {"primary": "obj-fp", "other": "z-fp"}}]}]}"#;
    let baseline = current.replace(r#""startLine": 2"#, r#""startLine": 9"#);
    let current_file = write_temp(&root, "current.sarif", current.as_bytes());
    let baseline_file = write_temp(&root, "baseline.sarif", baseline.as_bytes());
    let report = read_security_reports(SecurityInputs {
        current: &[current_file],
        baseline: &[baseline_file],
    })
    .unwrap();
    assert_eq!(report.findings[0].fingerprint, "obj-fp");
    assert_eq!(report.findings[0].state, FindingState::Existing);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn decodes_percent_encoded_artifact_uris() {
    let root = temporary_directory();
    let body = r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "t"}}, "results": [{"ruleId": "r", "level": "error", "message": {"text": "x"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/a%20b.ts"}, "region": {"startLine": 1}}}]}]}]}"#;
    let file = write_temp(&root, "encoded.sarif", body.as_bytes());
    let report = read_security_reports(SecurityInputs {
        current: &[file],
        baseline: &[],
    })
    .unwrap();
    assert_eq!(report.findings[0].path.as_deref(), Some("src/a b.ts"));
    // Decoding happens before the path checks, so an encoded traversal is
    // rejected and a malformed escape is an input error.
    for (name, uri) in [
        ("traversal.sarif", "..%2Fescape.ts"),
        ("malformed.sarif", "src/a%2.ts"),
    ] {
        let body = body.replace("src/a%20b.ts", uri);
        let file = write_temp(&root, name, body.as_bytes());
        assert!(
            read_security_reports(SecurityInputs {
                current: &[file],
                baseline: &[],
            })
            .is_err(),
            "{uri} must be rejected"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_oversized_or_overdeep_sarif() {
    let root = temporary_directory();
    let big = write_temp(&root, "big.sarif", &[0u8; 1]);
    std::fs::write(&big, vec![b'x'; (64 << 20) + 1]).unwrap();
    assert!(
        read_security_reports(SecurityInputs {
            current: &[big],
            baseline: &[],
        })
        .is_err()
    );
    let deep = "[".repeat(129);
    let file = write_temp(&root, "deep.sarif", deep.as_bytes());
    assert!(
        read_security_reports(SecurityInputs {
            current: &[file],
            baseline: &[],
        })
        .is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn finding(path: &str, line: u32) -> SecurityFinding {
    SecurityFinding {
        tool: "semgrep".to_owned(),
        rule_id: "r".to_owned(),
        severity: SecuritySeverity::High,
        path: Some(path.to_owned()),
        start_line: Some(line),
        end_line: Some(line),
        fingerprint: "fp".to_owned(),
        state: FindingState::New,
        changed: None,
        report_ids: vec!["t.sarif".to_owned()],
        function_id: None,
        cognitive: None,
        cyclomatic: None,
        crap: None,
        coverage: None,
        risk_score: None,
        fan_in: None,
        blast_radius: None,
        concentration_percent: None,
        reason: None,
    }
}

fn enrich_project(root: &std::path::Path) -> leadline::project::Project {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/auth.ts"),
        "function outer(x: number): number {\n  function login(y: number): number {\n    if (y > 0) { return 1; }\n    return 0;\n  }\n  return login(x);\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("src/empty.ts"), "export const ready = true;\n").unwrap();
    leadline::analytics::build(&leadline::analytics::ProjectRequest {
        path: root.to_path_buf(),
        target: leadline::source_snapshot::SnapshotTarget::Worktree,
        window: leadline::history::HistoryWindow::Days90,
        mutation_inputs: Vec::new(),
        test_maps: Vec::new(),
        ownership_mode: leadline::ownership::OwnershipMode::AggregateOnly,
        snapshots_path: None,
        coverage: None,
    })
    .unwrap()
}

#[test]
fn attributes_to_innermost_function_and_file_risk() {
    let root = temporary_directory();
    let project = enrich_project(&root);
    // Ruling: function_id reuses the existing ProjectFunction id
    // (path:kind:bytes) instead of the plan sketch `name@line`; the
    // existing identity is stable and needs no second format.
    let login = project
        .functions
        .iter()
        .find(|f| f.name == "login")
        .unwrap();
    let row = project
        .risk
        .rows
        .iter()
        .find(|r| r.path == "src/auth.ts")
        .unwrap();
    let mut report = SecurityReport {
        findings: vec![finding("src/auth.ts", 3)],
    };
    let changed = BTreeSet::from(["src/auth.ts".to_owned()]);
    enrich(&mut report, &project, Some(&changed));
    let enriched = &report.findings[0];
    assert_eq!(enriched.function_id.as_deref(), Some(login.id.as_str()));
    assert_eq!(enriched.cognitive, Some(login.cognitive));
    assert_eq!(enriched.cyclomatic, Some(login.cyclomatic));
    assert_eq!(enriched.changed, Some(true));
    assert_eq!(enriched.risk_score, Some(row.score));
    assert_eq!(enriched.fan_in, Some(row.fan_in as u64));
    assert_eq!(enriched.blast_radius, Some(row.blast_radius as u64));
    assert_eq!(enriched.concentration_percent, row.concentration_percent);
    assert_eq!(enriched.reason, None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn leaves_unknown_enrichment_as_null() {
    let root = temporary_directory();
    let project = enrich_project(&root);
    let mut report = SecurityReport {
        findings: vec![finding("src/missing.ts", 1)],
    };
    let changed = BTreeSet::from(["src/auth.ts".to_owned()]);
    enrich(&mut report, &project, Some(&changed));
    let enriched = &report.findings[0];
    assert_eq!(enriched.function_id, None);
    assert_eq!(enriched.cognitive, None);
    assert_eq!(enriched.risk_score, None);
    assert_eq!(enriched.changed, Some(false));
    assert!(enriched.reason.is_some());
    // Without a comparison every changed flag stays null.
    let mut report = SecurityReport {
        findings: vec![finding("src/auth.ts", 3)],
    };
    enrich(&mut report, &project, None);
    assert_eq!(report.findings[0].changed, None);
    assert!(report.findings[0].function_id.is_some());
    // Pathless findings stay unattributed with a null changed flag.
    let mut pathless = finding("src/auth.ts", 3);
    pathless.path = None;
    let mut report = SecurityReport {
        findings: vec![pathless],
    };
    enrich(&mut report, &project, Some(&changed));
    assert_eq!(report.findings[0].function_id, None);
    assert_eq!(report.findings[0].changed, None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn marks_line_less_findings_as_changed() {
    let root = temporary_directory();
    let project = enrich_project(&root);
    let mut line_less = finding("src/auth.ts", 3);
    line_less.start_line = None;
    let mut report = SecurityReport {
        findings: vec![line_less],
    };
    let changed = BTreeSet::from(["src/auth.ts".to_owned()]);
    enrich(&mut report, &project, Some(&changed));
    // A file-level SARIF result has no region, but its path is still Git
    // state: `--changed-only` must be able to gate it.
    assert_eq!(report.findings[0].changed, Some(true));
    assert_eq!(report.findings[0].function_id, None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn marks_functionless_changed_files() {
    let root = temporary_directory();
    let project = enrich_project(&root);
    let mut report = SecurityReport {
        findings: vec![finding("src/empty.ts", 1)],
    };
    let changed = BTreeSet::from(["src/empty.ts".to_owned()]);
    enrich(&mut report, &project, Some(&changed));
    let enriched = &report.findings[0];
    assert_eq!(enriched.function_id, None);
    assert_eq!(enriched.changed, Some(true));
    assert!(enriched.reason.is_some());
    std::fs::remove_dir_all(root).unwrap();
}

fn gated_finding(
    severity: SecuritySeverity,
    state: FindingState,
    changed: Option<bool>,
) -> SecurityFinding {
    SecurityFinding {
        tool: "t".to_owned(),
        rule_id: "r".to_owned(),
        severity,
        path: Some("src/a.ts".to_owned()),
        start_line: Some(1),
        end_line: Some(1),
        fingerprint: "fp".to_owned(),
        state,
        changed,
        report_ids: vec!["t.sarif".to_owned()],
        function_id: None,
        cognitive: None,
        cyclomatic: None,
        crap: None,
        coverage: None,
        risk_score: None,
        fan_in: None,
        blast_radius: None,
        concentration_percent: None,
        reason: None,
    }
}

#[test]
fn gate_violations_respect_minimum_new_and_changed() {
    let report = SecurityReport {
        findings: vec![
            gated_finding(SecuritySeverity::Critical, FindingState::New, Some(true)),
            gated_finding(SecuritySeverity::High, FindingState::Existing, Some(true)),
            gated_finding(SecuritySeverity::High, FindingState::New, Some(false)),
            gated_finding(SecuritySeverity::Low, FindingState::New, Some(true)),
        ],
    };
    let gate = SecurityGate {
        minimum: SecuritySeverity::High,
        new_only: false,
        changed_only: false,
    };
    assert_eq!(leadline::security::gate_violations(&report, &gate).len(), 3);
    let gate = SecurityGate {
        minimum: SecuritySeverity::High,
        new_only: true,
        changed_only: false,
    };
    assert_eq!(leadline::security::gate_violations(&report, &gate).len(), 2);
    let gate = SecurityGate {
        minimum: SecuritySeverity::High,
        new_only: false,
        changed_only: true,
    };
    assert_eq!(leadline::security::gate_violations(&report, &gate).len(), 2);
    let gate = SecurityGate {
        minimum: SecuritySeverity::Critical,
        new_only: true,
        changed_only: true,
    };
    assert_eq!(leadline::security::gate_violations(&report, &gate).len(), 1);
    // Unknown severities never meet a threshold.
    let report = SecurityReport {
        findings: vec![gated_finding(
            SecuritySeverity::Unknown,
            FindingState::New,
            Some(true),
        )],
    };
    let gate = SecurityGate {
        minimum: SecuritySeverity::Low,
        new_only: false,
        changed_only: false,
    };
    assert!(leadline::security::gate_violations(&report, &gate).is_empty());
}

#[test]
fn agent_json_caps_rows_with_truncated_flag() {
    let report = SecurityReport {
        findings: vec![
            gated_finding(SecuritySeverity::High, FindingState::New, Some(true)),
            gated_finding(SecuritySeverity::Low, FindingState::New, Some(true)),
        ],
    };
    let full = leadline::security::agent_json(&report, 50);
    assert_eq!(full["truncated"], false);
    assert_eq!(full["findings"].as_array().unwrap().len(), 2);
    let capped = leadline::security::agent_json(&report, 1);
    assert_eq!(capped["truncated"], true);
    assert_eq!(capped["findings"].as_array().unwrap().len(), 1);
}
