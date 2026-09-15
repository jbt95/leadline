//! Bounded SARIF 2.1.0 security-finding adapter.
//!
//! Normalizes untrusted scanner output into one deterministic
//! [`SecurityReport`]. Scanner messages, source snippets, and absolute
//! paths never survive normalization.

use crate::external::{INPUT_BYTES_LIMIT, InputBudget, read_bounded, strict_relative_path};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Normalized finding severity. Declaration order is ascending severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecuritySeverity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

/// New/existing state against supplied baselines or SARIF `baselineState`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingState {
    New,
    Existing,
    Unknown,
}

/// Current and baseline SARIF inputs. Baselines contribute keys only.
pub struct SecurityInputs<'a> {
    pub current: &'a [PathBuf],
    pub baseline: &'a [PathBuf],
}

/// One normalized finding. Unavailable evidence stays `None` and serializes
/// as `null`; enrichment fills the `Option` analytics fields later.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SecurityFinding {
    pub tool: String,
    pub rule_id: String,
    pub severity: SecuritySeverity,
    pub path: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub fingerprint: String,
    pub state: FindingState,
    pub changed: Option<bool>,
    pub report_ids: Vec<String>,
    pub function_id: Option<String>,
    pub cognitive: Option<u32>,
    pub cyclomatic: Option<u32>,
    pub crap: Option<f64>,
    pub coverage: Option<f64>,
    pub risk_score: Option<f64>,
    pub fan_in: Option<u64>,
    pub blast_radius: Option<u64>,
    pub concentration_percent: Option<f64>,
    pub reason: Option<String>,
}

/// Deterministically ordered normalized findings.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SecurityReport {
    pub findings: Vec<SecurityFinding>,
}

/// Read current SARIF files, classify against baselines, deduplicate by
/// semantic key while merging provenance report IDs, and sort.
pub fn read_security_reports(inputs: SecurityInputs<'_>) -> crate::Result<SecurityReport> {
    let mut budget = InputBudget::new();
    let mut baseline_keys = BTreeSet::new();
    for path in inputs.baseline {
        for finding in read_one(path, &mut budget)?.0 {
            baseline_keys.insert(semantic_key(&finding));
        }
    }
    let with_baselines = !inputs.baseline.is_empty();
    let mut merged: BTreeMap<(String, String, Option<String>, String), SecurityFinding> =
        BTreeMap::new();
    for path in inputs.current {
        let (findings, report_id) = read_one(path, &mut budget)?;
        for mut finding in findings {
            finding.state = if with_baselines {
                if baseline_keys.contains(&semantic_key(&finding)) {
                    FindingState::Existing
                } else {
                    FindingState::New
                }
            } else {
                finding.state
            };
            finding.report_ids = vec![report_id.clone()];
            merged
                .entry(semantic_key(&finding))
                .and_modify(|existing| {
                    merge_report_ids(&mut existing.report_ids, &finding.report_ids)
                })
                .or_insert(finding);
        }
    }
    let mut findings: Vec<SecurityFinding> = merged.into_values().collect();
    findings.sort_by(compare_findings);
    Ok(SecurityReport { findings })
}

/// Merge provenance report IDs into one row, keeping them sorted and unique.
pub(crate) fn merge_report_ids(existing: &mut Vec<String>, incoming: &[String]) {
    for id in incoming {
        if !existing.contains(id) {
            existing.push(id.clone());
        }
    }
    existing.sort();
}

/// Join findings against project functions, file risk, and changed paths.
///
/// One indexed pass: functions and risk rows are grouped by path once, then
/// each located finding takes the smallest containing span (ties break on
/// the lexical function ID). Unattributed findings keep null analytics and
/// gain a machine-readable `reason`. `changed` is set only when a
/// comparison was requested; pathless findings always stay null.
pub fn enrich(
    report: &mut SecurityReport,
    project: &crate::project::Project,
    changed_paths: Option<&BTreeSet<String>>,
) {
    let mut functions: BTreeMap<&str, Vec<&crate::project::ProjectFunction>> = BTreeMap::new();
    for function in &project.functions {
        functions
            .entry(function.path.as_str())
            .or_default()
            .push(function);
    }
    let mut risk: BTreeMap<&str, &crate::project::ProjectRiskRow> = BTreeMap::new();
    for row in &project.risk.rows {
        risk.insert(row.path.as_str(), row);
    }
    for finding in &mut report.findings {
        let mut attributed = false;
        if let (Some(path), Some(line)) = (finding.path.as_deref(), finding.start_line) {
            if let Some(candidates) = functions.get(path) {
                let best = crate::core::innermost_containing(
                    candidates,
                    line,
                    |function| (function.start_line, function.end_line),
                    |function| function.id.as_str(),
                );
                if let Some(function) = best {
                    finding.function_id = Some(function.id.clone());
                    finding.cognitive = Some(function.cognitive);
                    finding.cyclomatic = Some(function.cyclomatic);
                    finding.crap = function.crap;
                    finding.coverage = function.coverage;
                    attributed = true;
                }
            }
            if let Some(row) = risk.get(path) {
                finding.risk_score = Some(row.score);
                finding.fan_in = Some(row.fan_in as u64);
                finding.blast_radius = Some(row.blast_radius as u64);
                finding.concentration_percent = row.concentration_percent;
            }
        }
        // Changed state is file-level Git state: a located path counts even
        // when the scanner reported no region, or `--changed-only` would
        // silently skip file-level findings in changed files.
        finding.changed = finding
            .path
            .as_deref()
            .and_then(|path| changed_paths.map(|changed| changed.contains(path)));
        if !attributed {
            finding.reason = Some(
                match finding.path.as_deref() {
                    None => "no-path",
                    Some(path) if !functions.contains_key(path) && !risk.contains_key(path) => {
                        "path-not-in-project"
                    }
                    _ => "no-function-at-location",
                }
                .to_owned(),
            );
        }
    }
}

/// Default agent-JSON row cap shared by every scanner report.
pub const AGENT_DEFAULT_TOP: usize = 50;

/// Gate predicate: minimum severity with optional new/changed narrowing.
///
/// `--new-only` and `--changed-only` narrow the gate, never the report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SecurityGate {
    pub minimum: SecuritySeverity,
    pub new_only: bool,
    pub changed_only: bool,
}

/// Findings at or above the gate minimum, honoring new/changed narrowing.
///
/// Unknown severities sit below every threshold and never violate.
pub fn gate_violations<'a>(
    report: &'a SecurityReport,
    gate: &SecurityGate,
) -> Vec<&'a SecurityFinding> {
    report
        .findings
        .iter()
        .filter(|finding| {
            finding.severity >= gate.minimum
                && (!gate.new_only || finding.state == FindingState::New)
                && (!gate.changed_only || finding.changed == Some(true))
        })
        .collect()
}

/// Bounded agent projection: first `top` findings plus a `truncated` flag.
pub fn agent_json(report: &SecurityReport, top: usize) -> serde_json::Value {
    agent_page(&report.findings, top)
}

/// Shared bounded agent projection for every scanner report: first `top`
/// findings plus a `truncated` flag.
pub fn agent_page<T: Serialize>(findings: &[T], top: usize) -> serde_json::Value {
    serde_json::json!({
        "findings": findings.iter().take(top).collect::<Vec<_>>(),
        "truncated": findings.len() > top,
    })
}

/// Human-readable terminal rendering, deterministic in report order.
pub fn terminal_text(report: &SecurityReport) -> String {
    if report.findings.is_empty() {
        return "No security findings.
"
        .to_owned();
    }
    let mut out = String::new();
    for finding in &report.findings {
        let location = match (&finding.path, finding.start_line) {
            (Some(path), Some(line)) => format!("{path}:{line}"),
            (Some(path), None) => path.clone(),
            (None, _) => "?".to_owned(),
        };
        let mut markers = finding.state.as_str().to_owned();
        if finding.changed == Some(true) {
            markers.push_str(",changed");
        }
        let context = finding
            .function_id
            .as_deref()
            .or(finding.reason.as_deref())
            .unwrap_or("?");
        out.push_str(&format!(
            "{location} [{}] {}/{} ({markers}) {context}
",
            finding.severity.as_str(),
            finding.tool,
            finding.rule_id,
        ));
    }
    out
}

impl SecuritySeverity {
    /// Stable lowercase identifier shared by terminal output and gates.
    pub fn as_str(self) -> &'static str {
        match self {
            SecuritySeverity::Unknown => "unknown",
            SecuritySeverity::Low => "low",
            SecuritySeverity::Medium => "medium",
            SecuritySeverity::High => "high",
            SecuritySeverity::Critical => "critical",
        }
    }
}

impl FindingState {
    /// Stable lowercase identifier shared by terminal output.
    pub fn as_str(self) -> &'static str {
        match self {
            FindingState::New => "new",
            FindingState::Existing => "existing",
            FindingState::Unknown => "unknown",
        }
    }
}

/// Parse a `--fail-on-severity` gate level.
pub fn parse_gate_severity(raw: &str) -> Option<SecuritySeverity> {
    match raw {
        "low" => Some(SecuritySeverity::Low),
        "medium" => Some(SecuritySeverity::Medium),
        "high" => Some(SecuritySeverity::High),
        "critical" => Some(SecuritySeverity::Critical),
        _ => None,
    }
}

/// Git comparison backing `changed` attribution.
#[derive(Clone, Debug, PartialEq)]
pub enum ChangeComparison {
    Base(String),
    Staged,
    Target(String),
}

/// One assembled security request shared by the CLI, `check`, and MCP.
///
/// `gate` of `None` keeps the command informational: findings still render
/// but nothing violates.
pub struct SecurityRequest {
    pub path: PathBuf,
    pub sarif: Vec<PathBuf>,
    pub baseline_sarif: Vec<PathBuf>,
    pub comparison: Option<ChangeComparison>,
    pub gate: Option<SecurityGate>,
}

/// Assembled report plus owned gate violations.
pub struct SecurityOutcome {
    pub report: SecurityReport,
    pub violations: Vec<SecurityFinding>,
}

/// Assembly failure tagged with the CLI exit kind it maps to.
#[derive(Debug)]
pub enum SecurityError {
    /// Scanner input failure: the CLI exits `4`.
    Input(String),
    /// Project build or Git comparison failure: the CLI exits `3`.
    Incomplete(String),
}

impl SecurityError {
    /// The underlying failure message.
    pub fn message(&self) -> &str {
        match self {
            SecurityError::Input(message) | SecurityError::Incomplete(message) => message,
        }
    }
}

impl std::fmt::Display for SecurityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for SecurityError {}

/// Read SARIF inputs, build the project once, join enrichment, and apply
/// the gate: the single assembly path for every `security` surface.
pub fn assemble(request: &SecurityRequest) -> Result<SecurityOutcome, SecurityError> {
    let mut report = read_security_reports(SecurityInputs {
        current: &request.sarif,
        baseline: &request.baseline_sarif,
    })
    .map_err(|error| SecurityError::Input(error.to_string()))?;
    let project = crate::analytics::build(&crate::analytics::ProjectRequest {
        path: request.path.clone(),
        target: crate::source_snapshot::SnapshotTarget::Worktree,
        window: crate::history::HistoryWindow::Days90,
        mutation_inputs: Vec::new(),
        test_maps: Vec::new(),
        ownership_mode: crate::ownership::OwnershipMode::AggregateOnly,
        snapshots_path: None,
        coverage: None,
    })
    .map_err(|error| SecurityError::Incomplete(error.to_string()))?;
    let changed = match &request.comparison {
        None => None,
        Some(comparison) => {
            let (base, target) = match comparison {
                ChangeComparison::Base(base) => {
                    (base.clone(), crate::diff::ComparisonTarget::Worktree)
                }
                ChangeComparison::Staged => {
                    ("HEAD~1".to_owned(), crate::diff::ComparisonTarget::Index)
                }
                ChangeComparison::Target(revision) => (
                    "HEAD~1".to_owned(),
                    crate::diff::ComparisonTarget::Revision(revision.clone()),
                ),
            };
            let options = crate::diff::ChangeOptions {
                base,
                target,
                detect_renames: false,
            };
            Some(
                crate::diff::changed_paths(&request.path, &options)
                    .map_err(|error| SecurityError::Incomplete(error.to_string()))?,
            )
        }
    };
    enrich(&mut report, &project, changed.as_ref());
    let violations = request
        .gate
        .map(|gate| gate_violations(&report, &gate))
        .unwrap_or_default()
        .into_iter()
        .cloned()
        .collect();
    Ok(SecurityOutcome { report, violations })
}

/// Semantic identity: tool, rule, normalized path, and stable scanner
/// fingerprint when present; the fallback hash already binds the span.
fn semantic_key(finding: &SecurityFinding) -> (String, String, Option<String>, String) {
    (
        finding.tool.clone(),
        finding.rule_id.clone(),
        finding.path.clone(),
        finding.fingerprint.clone(),
    )
}

/// Severity descending, state (`new`, `existing`, `unknown`), path, line,
/// tool, rule, fingerprint. Pathless findings sort last.
fn compare_findings(left: &SecurityFinding, right: &SecurityFinding) -> std::cmp::Ordering {
    right
        .severity
        .cmp(&left.severity)
        .then(state_rank(left.state).cmp(&state_rank(right.state)))
        .then(compare_optional(&left.path, &right.path))
        .then(
            left.start_line
                .unwrap_or(u32::MAX)
                .cmp(&right.start_line.unwrap_or(u32::MAX)),
        )
        .then(left.tool.cmp(&right.tool))
        .then(left.rule_id.cmp(&right.rule_id))
        .then(left.fingerprint.cmp(&right.fingerprint))
}

/// Sort rank for finding state: `new`, `existing`, `unknown`.
pub(crate) fn state_rank(state: FindingState) -> u8 {
    match state {
        FindingState::New => 0,
        FindingState::Existing => 1,
        FindingState::Unknown => 2,
    }
}

fn compare_optional(left: &Option<String>, right: &Option<String>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(a), Some(b)) => a.cmp(b),
        (None, None) => std::cmp::Ordering::Equal,
        (None, _) => std::cmp::Ordering::Greater,
        (_, None) => std::cmp::Ordering::Less,
    }
}

/// Parse one SARIF file into raw findings plus its provenance report ID.
fn read_one(
    path: &Path,
    budget: &mut InputBudget,
) -> crate::Result<(Vec<SecurityFinding>, String)> {
    let report_id = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("SARIF input has no UTF-8 file name: {}", path.display()))?
        .to_owned();
    let bytes = read_bounded(path, INPUT_BYTES_LIMIT, budget)?;
    crate::external::validate_json_depth(&bytes)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid SARIF JSON {}: {error}", path.display()))?;
    let runs = value
        .get("runs")
        .and_then(|runs| runs.as_array())
        .ok_or_else(|| format!("invalid SARIF {}: missing \"runs\" array", path.display()))?;
    let mut findings = Vec::new();
    for run in runs {
        let driver = run.get("tool").and_then(|tool| tool.get("driver"));
        let tool = driver
            .and_then(|driver| driver.get("name"))
            .and_then(|name| name.as_str())
            .unwrap_or("unknown")
            .to_owned();
        let mut rule_severity = BTreeMap::new();
        if let Some(rules) = driver
            .and_then(|driver| driver.get("rules"))
            .and_then(|rules| rules.as_array())
        {
            for rule in rules {
                if let Some(id) = rule.get("id").and_then(|id| id.as_str()) {
                    rule_severity.insert(
                        id.to_owned(),
                        parse_severity(
                            rule.get("properties")
                                .and_then(|props| props.get("security-severity")),
                        ),
                    );
                }
            }
        }
        let results = run.get("results").and_then(|results| results.as_array());
        let Some(results) = results else { continue };
        for result in results {
            // Charge per finding as it is parsed: the byte limit alone lets a
            // compact report expand into millions of owned findings.
            budget.consume_rows(1)?;
            findings.push(parse_result(result, &tool, &rule_severity, path)?);
        }
    }
    Ok((findings, report_id))
}

/// Normalize one SARIF result. Messages are dropped; only the severity
/// signals, location, fingerprints, and baseline state are kept.
fn parse_result(
    result: &serde_json::Value,
    tool: &str,
    rule_severity: &BTreeMap<String, SecuritySeverity>,
    path: &Path,
) -> crate::Result<SecurityFinding> {
    let rule_id = result
        .get("ruleId")
        .and_then(|id| id.as_str())
        .or_else(|| {
            result
                .get("rule")
                .and_then(|rule| rule.get("id"))
                .and_then(|id| id.as_str())
        })
        .unwrap_or("unknown")
        .to_owned();
    let severity = parse_severity(
        result
            .get("properties")
            .and_then(|props| props.get("security-severity")),
    );
    let severity = if severity != SecuritySeverity::Unknown {
        severity
    } else if let Some(mapped) = rule_severity.get(&rule_id).copied() {
        if mapped != SecuritySeverity::Unknown {
            mapped
        } else {
            level_severity(result.get("level").and_then(|level| level.as_str()))
        }
    } else {
        level_severity(result.get("level").and_then(|level| level.as_str()))
    };
    let (normalized_path, start_line, end_line) = match location(result) {
        Some((uri, start, end)) => (Some(normalize_uri(uri, path)?), start, end),
        None => (None, None, None),
    };
    let fingerprint = fingerprint(
        result,
        tool,
        &rule_id,
        normalized_path.as_deref(),
        start_line,
        end_line,
    );
    let state = match result.get("baselineState").and_then(|state| state.as_str()) {
        Some("new") => FindingState::New,
        Some("unchanged") | Some("updated") => FindingState::Existing,
        _ => FindingState::Unknown,
    };
    Ok(SecurityFinding {
        tool: tool.to_owned(),
        rule_id,
        severity,
        path: normalized_path,
        start_line,
        end_line,
        fingerprint,
        state,
        changed: None,
        report_ids: Vec::new(),
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
    })
}

/// First physical location URI plus optional 1-based region lines.
fn location(result: &serde_json::Value) -> Option<(&str, Option<u32>, Option<u32>)> {
    let physical = result
        .get("locations")
        .and_then(|locations| locations.as_array())
        .and_then(|locations| locations.first())
        .and_then(|location| location.get("physicalLocation"))?;
    let uri = physical
        .get("artifactLocation")
        .and_then(|location| location.get("uri"))
        .and_then(|uri| uri.as_str())?;
    let region = physical.get("region");
    let line = |key: &str| {
        region
            .and_then(|region| region.get(key))
            .and_then(|line| line.as_u64())
            .and_then(|line| u32::try_from(line).ok())
            .filter(|line| *line > 0)
    };
    Some((uri, line("startLine"), line("endLine")))
}

/// Normalize one artifact URI to an analysis-root-relative path.
///
/// Backslashes become separators (Windows scanners) and `file:` URIs shed
/// their scheme; anything else with a URI scheme is rejected. Percent escapes
/// are decoded, so `src/a%20b.ts` names `src/a b.ts`; decoding happens before
/// the path checks, so an encoded traversal is still rejected.
fn normalize_uri(uri: &str, report: &Path) -> crate::Result<String> {
    let slashed = uri.replace('\\', "/");
    let (without_scheme, file_authority) = match slashed.strip_prefix("file://") {
        Some(rest) => (rest, Some(rest.split('/').next().unwrap_or(""))),
        None => (slashed.strip_prefix("file:").unwrap_or(&slashed), None),
    };
    if let Some(authority) = file_authority
        && !authority.is_empty()
    {
        return Err(format!(
            "invalid SARIF {}: unsupported file URI authority in {uri:?}",
            report.display()
        )
        .into());
    }
    if has_scheme(without_scheme) {
        return Err(format!(
            "invalid SARIF {}: unsupported artifact URI scheme in {uri:?}",
            report.display()
        )
        .into());
    }
    let decoded = percent_decode(without_scheme).ok_or_else(|| {
        format!(
            "invalid SARIF {}: malformed percent escape in {uri:?}",
            report.display()
        )
    })?;
    let decoded = decoded.replace('\\', "/");
    strict_relative_path(&decoded)
        .map_err(|error| format!("invalid SARIF {}: {}", report.display(), error).into())
}

/// Decode `%XX` escapes; a literal percent sign must be written `%25`.
fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_digit(*bytes.get(index + 1)?)?;
            let low = hex_digit(*bytes.get(index + 2)?)?;
            decoded.push(high << 4 | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// `true` for `scheme:rest` shapes, including Windows drive prefixes.
fn has_scheme(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(colon) = bytes.iter().position(|byte| *byte == b':') else {
        return false;
    };
    let (scheme, _) = value.split_at(colon);
    !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Lexicographically first `partialFingerprints` value, then first
/// `fingerprints` value (an object property bag per SARIF 2.1.0, with an
/// array tolerated from older producers), else a `blake3` hash binding tool,
/// rule, path, and span with separators.
fn fingerprint(
    result: &serde_json::Value,
    tool: &str,
    rule_id: &str,
    path: Option<&str>,
    start_line: Option<u32>,
    end_line: Option<u32>,
) -> String {
    if let Some(prints) = result
        .get("partialFingerprints")
        .and_then(|prints| prints.as_object())
        && let Some(first) = prints.values().filter_map(|value| value.as_str()).min()
    {
        return first.to_owned();
    }
    if let Some(prints) = result.get("fingerprints") {
        let object = prints
            .as_object()
            .and_then(|prints| prints.values().filter_map(|value| value.as_str()).min());
        let array = prints
            .as_array()
            .and_then(|prints| prints.first())
            .and_then(|print| print.as_str());
        if let Some(first) = object.or(array) {
            return first.to_owned();
        }
    }
    blake3::hash(
        format!(
            "{tool}\u{1f}{rule_id}\u{1f}{}\u{1f}{start_line:?}-{end_line:?}",
            path.unwrap_or("")
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}

/// `properties.security-severity`: numbers use the 0-10 thresholds,
/// case-insensitive labels map directly (`moderate` is medium).
fn parse_severity(value: Option<&serde_json::Value>) -> SecuritySeverity {
    let Some(value) = value else {
        return SecuritySeverity::Unknown;
    };
    if let Some(number) = value.as_f64() {
        return severity_from_score(number);
    }
    let Some(text) = value.as_str() else {
        return SecuritySeverity::Unknown;
    };
    if let Ok(number) = text.parse::<f64>() {
        if number.is_finite() {
            return severity_from_score(number);
        }
        return SecuritySeverity::Unknown;
    }
    severity_from_label(text)
}

/// Shared literal severity labels for scanner adapters (`moderate` is medium).
///
/// Case-insensitive; anything unrecognized stays unknown.
pub(crate) fn severity_from_label(text: &str) -> SecuritySeverity {
    match text.to_ascii_lowercase().as_str() {
        "critical" => SecuritySeverity::Critical,
        "high" | "error" => SecuritySeverity::High,
        "medium" | "moderate" | "warning" => SecuritySeverity::Medium,
        "low" | "note" => SecuritySeverity::Low,
        _ => SecuritySeverity::Unknown,
    }
}

/// Shared numeric 0-10 scores: `<4` low, `<7` medium, `<9` high, `>=9` critical.
pub(crate) fn severity_from_score(score: f64) -> SecuritySeverity {
    if !score.is_finite() {
        SecuritySeverity::Unknown
    } else if score < 4.0 {
        SecuritySeverity::Low
    } else if score < 7.0 {
        SecuritySeverity::Medium
    } else if score < 9.0 {
        SecuritySeverity::High
    } else {
        SecuritySeverity::Critical
    }
}

/// SARIF `level` without a severity property: `note`, `warning`, and
/// `error` map to `low`, `medium`, and `high`.
fn level_severity(level: Option<&str>) -> SecuritySeverity {
    match level {
        Some("error") => SecuritySeverity::High,
        Some("warning") => SecuritySeverity::Medium,
        Some("note") => SecuritySeverity::Low,
        _ => SecuritySeverity::Unknown,
    }
}
