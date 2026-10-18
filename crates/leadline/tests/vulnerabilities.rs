use leadline::security::{FindingState, SecuritySeverity};
use leadline::vulnerabilities::{VulnerabilityInputs, read_vulnerability_reports};
use std::path::PathBuf;
mod common;
use common::{temporary_directory, write_temp};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from("tests/fixtures/vulnerabilities").join(name)
}

#[test]
fn normalizes_osv_packages_and_fixed_versions() {
    let osv = [fixture("osv.json")];
    let report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &osv,
        trivy: &[],
        baseline_osv: &[],
        baseline_trivy: &[],
    })
    .unwrap();
    let row = &report.findings[0];
    assert_eq!((&row.ecosystem[..], &row.package[..]), ("npm", "lodash"));
    assert_eq!(row.advisory_id, "GHSA-xxxx-yyyy-zzzz");
    assert_eq!(row.state, FindingState::New);
    assert_eq!(row.severity, SecuritySeverity::High);
    assert_eq!(row.fixed_versions, vec!["4.17.21"]);
    assert_eq!(row.manifest_path.as_deref(), Some("package-lock.json"));
    assert_eq!(row.reachable_from_changed, None);
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("DESCRIPTION_SENTINEL")
    );
}

#[test]
fn normalizes_trivy_without_descriptions() {
    let trivy = [fixture("trivy.json")];
    let report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &[],
        trivy: &trivy,
        baseline_osv: &[],
        baseline_trivy: &[],
    })
    .unwrap();
    let row = &report.findings[0];
    assert_eq!((&row.ecosystem[..], &row.package[..]), ("npm", "minimist"));
    assert_eq!(row.advisory_id, "CVE-2021-9999");
    assert_eq!(row.severity, SecuritySeverity::Medium);
    assert_eq!(row.fixed_versions, vec!["1.2.6"]);
    assert_eq!(row.installed_version, "1.2.5");
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("DESCRIPTION_SENTINEL")
    );
}

#[test]
fn merges_duplicate_advisories_and_provenance() {
    let root = temporary_directory();
    // Same semantic identity as the osv.json lodash row, via trivy shape.
    let dup = r#"{"Results": [{"Target": "package-lock.json", "Type": "npm", "Vulnerabilities": [{"VulnerabilityID": "GHSA-xxxx-yyyy-zzzz", "PkgName": "lodash", "InstalledVersion": "4.17.20", "FixedVersion": "4.17.21", "Severity": "HIGH"}]}]}"#;
    let dup_file = write_temp(&root, "dup.json", dup.as_bytes());
    let osv = [fixture("osv.json")];
    let trivy = [dup_file];
    let report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &osv,
        trivy: &trivy,
        baseline_osv: &[],
        baseline_trivy: &[],
    })
    .unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].report_ids, vec!["dup.json", "osv.json"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn classifies_against_baselines() {
    let osv = [fixture("osv.json")];
    let baseline = [fixture("osv.json")];
    let report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &osv,
        trivy: &[],
        baseline_osv: &baseline,
        baseline_trivy: &[],
    })
    .unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].state, FindingState::Existing);
    let trivy = [fixture("trivy.json")];
    let report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &osv,
        trivy: &trivy,
        baseline_osv: &[],
        baseline_trivy: &[],
    })
    .unwrap();
    assert_eq!(report.findings.len(), 2);
    // Severity descending: high lodash before medium minimist.
    assert_eq!(report.findings[0].package, "lodash");
    assert_eq!(report.findings[1].package, "minimist");
    assert!(
        report
            .findings
            .iter()
            .all(|row| row.state == FindingState::New)
    );
}

#[test]
fn rejects_invalid_paths_depth_size_and_row_counts() {
    let root = temporary_directory();
    let bad_path = r#"{"results": [{"source": {"path": "/etc/passwd"}, "packages": []}]}"#;
    let file = write_temp(&root, "bad-path.json", bad_path.as_bytes());
    assert!(
        read_vulnerability_reports(VulnerabilityInputs {
            osv: &[file],
            trivy: &[],
            baseline_osv: &[],
            baseline_trivy: &[],
        })
        .is_err()
    );
    let deep = "[".repeat(129);
    let file = write_temp(&root, "deep.json", deep.as_bytes());
    assert!(
        read_vulnerability_reports(VulnerabilityInputs {
            osv: &[file],
            trivy: &[],
            baseline_osv: &[],
            baseline_trivy: &[],
        })
        .is_err()
    );
    let big = write_temp(&root, "big.json", &[0u8; 1]);
    std::fs::write(&big, vec![b'x'; (64 << 20) + 1]).unwrap();
    assert!(
        read_vulnerability_reports(VulnerabilityInputs {
            osv: &[big],
            trivy: &[],
            baseline_osv: &[],
            baseline_trivy: &[],
        })
        .is_err()
    );
    // More than 32 paths per input kind are rejected without reading them.
    let many: Vec<PathBuf> = (0..33).map(|i| root.join(format!("f{i}.json"))).collect();
    assert!(
        read_vulnerability_reports(VulnerabilityInputs {
            osv: &many,
            trivy: &[],
            baseline_osv: &[],
            baseline_trivy: &[],
        })
        .is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_import_evidence_marks_direct_matches() {
    use leadline::graph::external_packages_from_sources;
    use leadline::source_snapshot::SourceEntry;
    use leadline::vulnerabilities::add_changed_import_evidence;
    let osv = [fixture("osv.json")];
    let trivy = [fixture("trivy.json")];
    let mut report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &osv,
        trivy: &trivy,
        baseline_osv: &[],
        baseline_trivy: &[],
    })
    .unwrap();
    let entries = vec![SourceEntry {
        path: "src/new.ts".to_owned(),
        bytes: b"import _ from 'lodash';
"
        .to_vec(),
    }];
    let imports = external_packages_from_sources(&entries).unwrap();
    assert_eq!(imports[0].package, "lodash");
    add_changed_import_evidence(&mut report, &imports, true);
    let lodash = report
        .findings
        .iter()
        .find(|row| row.package == "lodash")
        .unwrap();
    assert_eq!(lodash.reachable_from_changed, Some(true));
    assert_eq!(lodash.changed_imports[0].path, "src/new.ts");
    // minimist has no changed direct import: proven absent.
    let minimist = report
        .findings
        .iter()
        .find(|row| row.package == "minimist")
        .unwrap();
    assert_eq!(minimist.reachable_from_changed, Some(false));
    assert!(minimist.changed_imports.is_empty());
}

#[test]
fn changed_import_evidence_stays_unknown_without_comparison() {
    use leadline::graph::external_packages_from_sources;
    use leadline::source_snapshot::SourceEntry;
    use leadline::vulnerabilities::add_changed_import_evidence;
    let root = temporary_directory();
    let java = r#"{"results": [{"source": {"path": "pom.xml"}, "packages": [{"package": {"name": "org.example:lib", "version": "1.0", "ecosystem": "Maven"}, "vulnerabilities": [{"id": "CVE-2024-0001", "database_specific": {"severity": "HIGH"}}]}]}]}"#;
    let java_file = write_temp(&root, "java.json", java.as_bytes());
    let mut report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &[java_file],
        trivy: &[],
        baseline_osv: &[],
        baseline_trivy: &[],
    })
    .unwrap();
    let entries = vec![SourceEntry {
        path: "src/new.ts".to_owned(),
        bytes: b"import _ from 'org.example';
"
        .to_vec(),
    }];
    let imports = external_packages_from_sources(&entries).unwrap();
    // Java advisories have no package-to-import mapping: always unknown,
    // even when the comparison succeeded.
    add_changed_import_evidence(&mut report, &imports, true);
    assert_eq!(report.findings[0].reachable_from_changed, None);
    // Without a comparison, npm findings also stay unknown.
    let osv = [fixture("osv.json")];
    let mut report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &osv,
        trivy: &[],
        baseline_osv: &[],
        baseline_trivy: &[],
    })
    .unwrap();
    add_changed_import_evidence(&mut report, &imports, false);
    assert_eq!(report.findings[0].reachable_from_changed, None);
    std::fs::remove_dir_all(root).unwrap();
}

fn gated_row(
    advisory: &str,
    severity: SecuritySeverity,
    state: FindingState,
    reachable: Option<bool>,
) -> leadline::vulnerabilities::VulnerabilityFinding {
    leadline::vulnerabilities::VulnerabilityFinding {
        ecosystem: "npm".to_owned(),
        package: "pkg".to_owned(),
        installed_version: "1.0".to_owned(),
        advisory_id: advisory.to_owned(),
        severity,
        fixed_versions: Vec::new(),
        manifest_path: None,
        state,
        reachable_from_changed: reachable,
        changed_imports: Vec::new(),
        report_ids: vec!["t.json".to_owned()],
    }
}

#[test]
fn gate_violations_select_new_reachable_findings() {
    use leadline::vulnerabilities::{VulnerabilityReport, vulnerability_gate_violations};
    let report = VulnerabilityReport {
        findings: vec![
            gated_row(
                "CVE-new-reachable",
                SecuritySeverity::High,
                FindingState::New,
                Some(true),
            ),
            gated_row(
                "CVE-new-unknown",
                SecuritySeverity::High,
                FindingState::New,
                None,
            ),
            gated_row(
                "CVE-new-absent",
                SecuritySeverity::High,
                FindingState::New,
                Some(false),
            ),
            gated_row(
                "CVE-existing",
                SecuritySeverity::Critical,
                FindingState::Existing,
                Some(true),
            ),
            gated_row(
                "CVE-low",
                SecuritySeverity::Low,
                FindingState::New,
                Some(true),
            ),
        ],
    };
    let violations = vulnerability_gate_violations(&report, SecuritySeverity::High);
    assert_eq!(
        violations
            .iter()
            .map(|row| row.advisory_id.as_str())
            .collect::<Vec<_>>(),
        ["CVE-new-reachable", "CVE-new-unknown"]
    );
}
