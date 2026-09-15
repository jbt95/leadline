//! Bounded OSV-Scanner and Trivy vulnerability adapters.
//!
//! Normalizes untrusted scanner output into one deterministic
//! [`VulnerabilityReport`]. Descriptions, titles, URLs, and package-manager
//! output never survive normalization. CVSS vectors are not scored: only
//! literal labels and plain numeric scores map to severities.

use crate::external::{INPUT_BYTES_LIMIT, InputBudget, read_bounded, strict_relative_path};
use crate::security::{FindingState, SecuritySeverity, merge_report_ids};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Maximum report paths per input kind.
const MAX_INPUT_PATHS: usize = 32;

/// Current and baseline scanner inputs. Baselines contribute keys only.
pub struct VulnerabilityInputs<'a> {
    pub osv: &'a [PathBuf],
    pub trivy: &'a [PathBuf],
    pub baseline_osv: &'a [PathBuf],
    pub baseline_trivy: &'a [PathBuf],
}

/// One changed file importing a vulnerable package directly.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChangedImport {
    pub path: String,
    pub package: String,
}

/// One normalized advisory. Unavailable evidence stays `None` or empty;
/// reachability is filled by [`add_changed_import_evidence`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VulnerabilityFinding {
    pub ecosystem: String,
    pub package: String,
    pub installed_version: String,
    pub advisory_id: String,
    pub severity: SecuritySeverity,
    pub fixed_versions: Vec<String>,
    pub manifest_path: Option<String>,
    pub state: FindingState,
    pub reachable_from_changed: Option<bool>,
    pub changed_imports: Vec<ChangedImport>,
    pub report_ids: Vec<String>,
}

/// Deterministically ordered normalized advisories.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VulnerabilityReport {
    pub findings: Vec<VulnerabilityFinding>,
}

/// Read current scanner files, classify against baselines, deduplicate by
/// semantic identity while merging provenance report IDs, and sort.
///
/// Without baseline files every finding is new.
pub fn read_vulnerability_reports(
    inputs: VulnerabilityInputs<'_>,
) -> crate::Result<VulnerabilityReport> {
    for paths in [
        inputs.osv,
        inputs.trivy,
        inputs.baseline_osv,
        inputs.baseline_trivy,
    ] {
        if paths.len() > MAX_INPUT_PATHS {
            return Err(
                format!("too many vulnerability inputs (max {MAX_INPUT_PATHS} per kind)").into(),
            );
        }
    }
    let mut budget = InputBudget::new();
    let mut baseline_keys = BTreeSet::new();
    for path in inputs.baseline_osv {
        for finding in read_osv(path, &mut budget)?.0 {
            baseline_keys.insert(semantic_key(&finding));
        }
    }
    for path in inputs.baseline_trivy {
        for finding in read_trivy(path, &mut budget)?.0 {
            baseline_keys.insert(semantic_key(&finding));
        }
    }
    let with_baselines = !inputs.baseline_osv.is_empty() || !inputs.baseline_trivy.is_empty();
    let mut merged: BTreeMap<(String, String, String, String), VulnerabilityFinding> =
        BTreeMap::new();
    for path in inputs.osv {
        let (findings, report_id) = read_osv(path, &mut budget)?;
        merge_findings(
            &mut merged,
            findings,
            &report_id,
            &baseline_keys,
            with_baselines,
        );
    }
    for path in inputs.trivy {
        let (findings, report_id) = read_trivy(path, &mut budget)?;
        merge_findings(
            &mut merged,
            findings,
            &report_id,
            &baseline_keys,
            with_baselines,
        );
    }
    let mut findings: Vec<VulnerabilityFinding> = merged.into_values().collect();
    findings.sort_by(compare_findings);
    Ok(VulnerabilityReport { findings })
}

/// Classify one input's findings against the baselines and merge them by
/// semantic key, keeping every provenance report ID.
fn merge_findings(
    merged: &mut BTreeMap<(String, String, String, String), VulnerabilityFinding>,
    findings: Vec<VulnerabilityFinding>,
    report_id: &str,
    baseline_keys: &BTreeSet<(String, String, String, String)>,
    with_baselines: bool,
) {
    for mut finding in findings {
        finding.state = classify(&finding, baseline_keys, with_baselines);
        finding.report_ids = vec![report_id.to_owned()];
        merged
            .entry(semantic_key(&finding))
            .and_modify(|existing| merge_report_ids(&mut existing.report_ids, &finding.report_ids))
            .or_insert(finding);
    }
}

/// Attach direct-import evidence to npm findings.
///
/// With a successful comparison, npm findings importing a changed package
/// gain `Some(true)` plus the matching imports; npm findings with no match
/// gain `Some(false)`. Non-npm ecosystems have no package-to-import mapping
/// and stay `None`, as does everything when the comparison was never
/// available. Direct imports only: transitive reachability is never inferred.
pub fn add_changed_import_evidence(
    report: &mut VulnerabilityReport,
    imports: &[crate::graph::ExternalPackageImport],
    changed_source_available: bool,
) {
    if !changed_source_available {
        return;
    }
    let mut by_package: BTreeMap<&str, Vec<&crate::graph::ExternalPackageImport>> = BTreeMap::new();
    for import in imports {
        by_package
            .entry(import.package.as_str())
            .or_default()
            .push(import);
    }
    for finding in &mut report.findings {
        if finding.ecosystem != "npm" {
            continue;
        }
        let matched: Vec<ChangedImport> = by_package
            .get(finding.package.as_str())
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| ChangedImport {
                        path: entry.path.clone(),
                        package: entry.package.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if matched.is_empty() {
            finding.reachable_from_changed = Some(false);
        } else {
            finding.reachable_from_changed = Some(true);
            finding.changed_imports = matched;
        }
    }
}

/// Gate violations: new findings at or above `minimum` that are not proven
/// absent from changed direct imports.
///
/// `reachable_from_changed` of `Some(false)` suppresses failure; `true` and
/// `None` (including non-npm ecosystems without import evidence) remain
/// gate candidates. Report order is preserved.
pub fn vulnerability_gate_violations(
    report: &VulnerabilityReport,
    minimum: SecuritySeverity,
) -> Vec<&VulnerabilityFinding> {
    report
        .findings
        .iter()
        .filter(|finding| {
            finding.state == FindingState::New
                && finding.severity >= minimum
                && finding.reachable_from_changed != Some(false)
        })
        .collect()
}

/// One assembled vulnerability request shared by the CLI, `check`, and MCP.
///
/// `gate` of `None` keeps the surface informational: findings still render
/// but nothing violates.
pub struct VulnerabilityRequest {
    pub path: PathBuf,
    pub osv: Vec<PathBuf>,
    pub trivy: Vec<PathBuf>,
    pub baseline_osv: Vec<PathBuf>,
    pub baseline_trivy: Vec<PathBuf>,
    pub comparison: Option<crate::security::ChangeComparison>,
    pub gate: Option<SecuritySeverity>,
}

/// Assembled report plus owned gate violations.
pub struct VulnerabilityOutcome {
    pub report: VulnerabilityReport,
    pub violations: Vec<VulnerabilityFinding>,
}

/// Read scanner inputs, attach changed-import evidence, and apply the gate:
/// the single assembly path for every `vulnerabilities` surface.
pub fn assemble(request: &VulnerabilityRequest) -> crate::Result<VulnerabilityOutcome> {
    let mut report = read_vulnerability_reports(VulnerabilityInputs {
        osv: &request.osv,
        trivy: &request.trivy,
        baseline_osv: &request.baseline_osv,
        baseline_trivy: &request.baseline_trivy,
    })?;
    let (imports, available) = match &request.comparison {
        None => (Vec::new(), false),
        Some(comparison) => {
            let (base, target) = match comparison {
                crate::security::ChangeComparison::Base(base) => {
                    (base.clone(), crate::diff::ComparisonTarget::Worktree)
                }
                crate::security::ChangeComparison::Staged => {
                    ("HEAD~1".to_owned(), crate::diff::ComparisonTarget::Index)
                }
                crate::security::ChangeComparison::Target(revision) => (
                    "HEAD~1".to_owned(),
                    crate::diff::ComparisonTarget::Revision(revision.clone()),
                ),
            };
            let options = crate::diff::ChangeOptions {
                base,
                target,
                detect_renames: false,
            };
            let entries = crate::diff::changed_source_entries(&request.path, &options)?;
            (
                crate::graph::external_packages_from_sources(&entries)?,
                true,
            )
        }
    };
    add_changed_import_evidence(&mut report, &imports, available);
    let violations = request
        .gate
        .map(|minimum| vulnerability_gate_violations(&report, minimum))
        .unwrap_or_default()
        .into_iter()
        .cloned()
        .collect();
    Ok(VulnerabilityOutcome { report, violations })
}

/// Bounded agent projection: first `top` findings plus a `truncated` flag.
pub fn agent_json(report: &VulnerabilityReport, top: usize) -> serde_json::Value {
    crate::security::agent_page(&report.findings, top)
}

/// Human-readable terminal rendering, deterministic in report order.
pub fn terminal_text(report: &VulnerabilityReport) -> String {
    if report.findings.is_empty() {
        return "No vulnerable dependencies.
"
        .to_owned();
    }
    let mut out = String::new();
    for finding in &report.findings {
        let mut markers = finding.state.as_str().to_owned();
        match finding.reachable_from_changed {
            Some(true) => markers.push_str(",changed-import"),
            Some(false) => markers.push_str(",unchanged-import"),
            None => {}
        }
        out.push_str(&format!(
            "{} {}@{} [{}] ({markers})
",
            finding.advisory_id,
            finding.package,
            finding.installed_version,
            finding.severity.as_str(),
        ));
    }
    out
}

/// Semantic identity: lowercase ecosystem, package, installed version, advisory.
fn semantic_key(finding: &VulnerabilityFinding) -> (String, String, String, String) {
    (
        finding.ecosystem.clone(),
        finding.package.clone(),
        finding.installed_version.clone(),
        finding.advisory_id.clone(),
    )
}

fn classify(
    finding: &VulnerabilityFinding,
    baseline_keys: &BTreeSet<(String, String, String, String)>,
    with_baselines: bool,
) -> FindingState {
    if !with_baselines || !baseline_keys.contains(&semantic_key(finding)) {
        FindingState::New
    } else {
        FindingState::Existing
    }
}

/// Severity descending, state (`new`, `existing`, `unknown`), ecosystem,
/// package, version, advisory.
fn compare_findings(
    left: &VulnerabilityFinding,
    right: &VulnerabilityFinding,
) -> std::cmp::Ordering {
    right
        .severity
        .cmp(&left.severity)
        .then(
            crate::security::state_rank(left.state).cmp(&crate::security::state_rank(right.state)),
        )
        .then(left.ecosystem.cmp(&right.ecosystem))
        .then(left.package.cmp(&right.package))
        .then(left.installed_version.cmp(&right.installed_version))
        .then(left.advisory_id.cmp(&right.advisory_id))
}

/// Provenance report ID: the input file name.
fn report_id(path: &Path) -> crate::Result<String> {
    Ok(path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "vulnerability input has no UTF-8 file name: {}",
                path.display()
            )
        })?
        .to_owned())
}

fn read_bounded_json(path: &Path, budget: &mut InputBudget) -> crate::Result<serde_json::Value> {
    let bytes = read_bounded(path, INPUT_BYTES_LIMIT, budget)?;
    crate::external::validate_json_depth(&bytes)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid vulnerability JSON {}: {error}", path.display()).into())
}

/// Parse one OSV-Scanner file: package identity, advisory ID, curated
/// severity label or plain numeric score, fixed range events, source path.
fn read_osv(
    path: &Path,
    budget: &mut InputBudget,
) -> crate::Result<(Vec<VulnerabilityFinding>, String)> {
    let value = read_bounded_json(path, budget)?;
    let results = value
        .get("results")
        .and_then(|results| results.as_array())
        .ok_or_else(|| format!("invalid OSV {}: missing \"results\" array", path.display()))?;
    let mut findings = Vec::new();
    for result in results {
        let manifest_path = match result.get("source").and_then(|source| source.get("path")) {
            None => None,
            Some(raw) => {
                let text = raw.as_str().ok_or_else(|| {
                    format!(
                        "invalid OSV {}: source path must be a string",
                        path.display()
                    )
                })?;
                Some(
                    strict_relative_path(text)
                        .map_err(|error| format!("invalid OSV {}: {error}", path.display()))?,
                )
            }
        };
        let packages = result
            .get("packages")
            .and_then(|packages| packages.as_array());
        let Some(packages) = packages else { continue };
        for entry in packages {
            let package = entry.get("package");
            let Some(name) = package
                .and_then(|package| package.get("name"))
                .and_then(|name| name.as_str())
            else {
                continue;
            };
            let version = package
                .and_then(|package| package.get("version"))
                .and_then(|version| version.as_str())
                .unwrap_or_default();
            let ecosystem = package
                .and_then(|package| package.get("ecosystem"))
                .and_then(|ecosystem| ecosystem.as_str())
                .unwrap_or("unknown")
                .to_ascii_lowercase();
            let vulnerabilities = entry
                .get("vulnerabilities")
                .and_then(|list| list.as_array());
            let Some(vulnerabilities) = vulnerabilities else {
                continue;
            };
            for vulnerability in vulnerabilities {
                let Some(advisory_id) = vulnerability.get("id").and_then(|id| id.as_str()) else {
                    continue;
                };
                budget.consume_rows(1)?;
                findings.push(VulnerabilityFinding {
                    ecosystem: ecosystem.clone(),
                    package: name.to_owned(),
                    installed_version: version.to_owned(),
                    advisory_id: advisory_id.to_owned(),
                    severity: osv_severity(vulnerability),
                    fixed_versions: osv_fixed_versions(vulnerability),
                    manifest_path: manifest_path.clone(),
                    state: FindingState::New,
                    reachable_from_changed: None,
                    changed_imports: Vec::new(),
                    report_ids: Vec::new(),
                });
            }
        }
    }
    Ok((findings, report_id(path)?))
}

/// Curated `database_specific.severity` label first, then the first plain
/// numeric `severity[].score`; CVSS vectors stay unknown.
fn osv_severity(vulnerability: &serde_json::Value) -> SecuritySeverity {
    if let Some(label) = vulnerability
        .get("database_specific")
        .and_then(|specific| specific.get("severity"))
        .and_then(|severity| severity.as_str())
    {
        let mapped = crate::security::severity_from_label(label);
        if mapped != SecuritySeverity::Unknown {
            return mapped;
        }
    }
    vulnerability
        .get("severity")
        .and_then(|entries| entries.as_array())
        .and_then(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("score").and_then(|score| score.as_str()))
                .filter_map(|score| score.parse::<f64>().ok().filter(|score| score.is_finite()))
                .map(crate::security::severity_from_score)
                .next()
        })
        .unwrap_or(SecuritySeverity::Unknown)
}

/// Sorted unique `fixed` versions from every affected range event.
fn osv_fixed_versions(vulnerability: &serde_json::Value) -> Vec<String> {
    let mut fixed = BTreeSet::new();
    if let Some(affected) = vulnerability
        .get("affected")
        .and_then(|affected| affected.as_array())
    {
        for scope in affected {
            if let Some(ranges) = scope.get("ranges").and_then(|ranges| ranges.as_array()) {
                for range in ranges {
                    if let Some(events) = range.get("events").and_then(|events| events.as_array()) {
                        for event in events {
                            if let Some(version) =
                                event.get("fixed").and_then(|version| version.as_str())
                            {
                                fixed.insert(version.to_owned());
                            }
                        }
                    }
                }
            }
        }
    }
    fixed.into_iter().collect()
}

/// Parse one Trivy file: target manifest, `Type` ecosystem, advisory and
/// package identity, fixed version, literal severity. Titles, descriptions,
/// and CVSS objects never cross over.
fn read_trivy(
    path: &Path,
    budget: &mut InputBudget,
) -> crate::Result<(Vec<VulnerabilityFinding>, String)> {
    let value = read_bounded_json(path, budget)?;
    let results = value
        .get("Results")
        .and_then(|results| results.as_array())
        .ok_or_else(|| {
            format!(
                "invalid Trivy {}: missing \"Results\" array",
                path.display()
            )
        })?;
    let mut findings = Vec::new();
    for result in results {
        let manifest_path = match result.get("Target").and_then(|target| target.as_str()) {
            None => None,
            Some(text) => Some(
                strict_relative_path(text)
                    .map_err(|error| format!("invalid Trivy {}: {error}", path.display()))?,
            ),
        };
        let ecosystem = result
            .get("Type")
            .and_then(|kind| kind.as_str())
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        let vulnerabilities = result
            .get("Vulnerabilities")
            .and_then(|list| list.as_array());
        let Some(vulnerabilities) = vulnerabilities else {
            continue;
        };
        for vulnerability in vulnerabilities {
            let (Some(advisory_id), Some(package)) = (
                vulnerability
                    .get("VulnerabilityID")
                    .and_then(|id| id.as_str()),
                vulnerability.get("PkgName").and_then(|name| name.as_str()),
            ) else {
                continue;
            };
            let version = vulnerability
                .get("InstalledVersion")
                .and_then(|version| version.as_str())
                .unwrap_or_default();
            let fixed_versions = vulnerability
                .get("FixedVersion")
                .and_then(|version| version.as_str())
                .filter(|version| !version.is_empty())
                .map(|version| vec![version.to_owned()])
                .unwrap_or_default();
            let severity = vulnerability
                .get("Severity")
                .and_then(|severity| severity.as_str())
                .map(crate::security::severity_from_label)
                .unwrap_or(SecuritySeverity::Unknown);
            budget.consume_rows(1)?;
            findings.push(VulnerabilityFinding {
                ecosystem: ecosystem.clone(),
                package: package.to_owned(),
                installed_version: version.to_owned(),
                advisory_id: advisory_id.to_owned(),
                severity,
                fixed_versions,
                manifest_path: manifest_path.clone(),
                state: FindingState::New,
                reachable_from_changed: None,
                changed_imports: Vec::new(),
                report_ids: Vec::new(),
            });
        }
    }
    Ok((findings, report_id(path)?))
}
