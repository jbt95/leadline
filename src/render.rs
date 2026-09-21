//! CI output formats over one shared finding model.
//!
//! Every renderer is a pure function of the findings and the gate summary, so
//! identical input produces identical bytes: no timestamps, no generated
//! identifiers, no hash-map iteration order. `report --from` renders the same
//! bytes as a direct run because both go through
//! [`findings_from_check_json`] and [`render`].

use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// One finding row shared by every CI format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    pub path: String,
    pub line: u32,
    pub end_line: Option<u32>,
    /// `function`, `parse_error`, `security`, `vulnerability`, or `sql`.
    pub kind: &'static str,
    /// Normalized level: `unknown`, `low`, `medium`, `high`, or `critical`.
    pub severity: &'static str,
    pub message: String,
    /// Stable identity: a hash of `path`, `kind`, and `message`, independent
    /// of line drift, so a moved finding keeps its identity across runs.
    pub fingerprint: String,
}

/// Gate verdict plus the counts the renderers need.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GateSummary {
    pub passed: bool,
    pub files: usize,
    pub violations: usize,
}

/// Gate limits re-applied to `files[].functions[].metrics` when the finding
/// set is rebuilt from a saved check document.
///
/// `f64` so one struct covers the integer metric limits and CRAP; the
/// comparisons mirror [`crate::core::Thresholds::violation_reasons`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GateLimits {
    pub cognitive: Option<f64>,
    pub cyclomatic: Option<f64>,
    pub max_nesting: Option<f64>,
    pub crap: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CiFormat {
    CodeClimate,
    GithubAnnotations,
    GithubSummary,
    Markdown,
    Badge,
    Compact,
}

/// Format name to renderer; `gitlab-codequality` is GitLab's name for the
/// CodeClimate document. `json`, `agent-json`, and `sarif` are handled by the
/// existing projections and are not CI formats.
pub fn parse_format(name: &str) -> Option<CiFormat> {
    match name {
        "codeclimate" | "gitlab-codequality" => Some(CiFormat::CodeClimate),
        "github-annotations" => Some(CiFormat::GithubAnnotations),
        "github-summary" => Some(CiFormat::GithubSummary),
        "markdown" => Some(CiFormat::Markdown),
        "badge" => Some(CiFormat::Badge),
        "compact" => Some(CiFormat::Compact),
        _ => None,
    }
}

/// Render one report in one CI format.
///
/// Findings are ordered by `(path, line, kind, message)` first, so the bytes
/// depend on the finding set rather than on caller order. Every format ends
/// with a newline; a format with no rows renders an empty string.
pub fn render(findings: &[Finding], summary: &GateSummary, format: CiFormat) -> String {
    let mut ordered: Vec<&Finding> = findings.iter().collect();
    ordered.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.kind.cmp(right.kind))
            .then(left.message.cmp(&right.message))
    });
    match format {
        CiFormat::CodeClimate => codeclimate(&ordered),
        CiFormat::GithubAnnotations => github_annotations(&ordered),
        CiFormat::GithubSummary | CiFormat::Markdown => markdown(&ordered, summary),
        CiFormat::Badge => badge(summary),
        CiFormat::Compact => compact(&ordered),
    }
}

const MISSING_DOCUMENT: &str = "not a check report: expected `files` or `security_violations` / \
                                 `vulnerability_violations` / `sql_violations`";

/// Rebuild the finding set from a saved `check --json` document.
///
/// Function findings come from `files[].functions[].metrics` re-gated with
/// `limits`, parse errors from `files[].parse_errors[]`, and scanner findings
/// from `security_violations`, `vulnerability_violations`, and
/// `sql_violations` when the scan families contributed. A document that is
/// neither an analysis report nor a scanner payload is an error; the caller
/// maps that to exit `2`.
///
/// The result is sorted by `(path, line, kind, message)` and every row carries
/// its stable fingerprint.
pub fn findings_from_check_json(
    document: &Value,
    limits: &GateLimits,
) -> Result<Vec<Finding>, String> {
    let object = document
        .as_object()
        .ok_or_else(|| MISSING_DOCUMENT.to_owned())?;
    let files = match object.get("files") {
        Some(Value::Array(files)) => Some(files.as_slice()),
        Some(_) => return Err("check report `files` must be an array".to_owned()),
        None => None,
    };
    let mut scanner_rows: Vec<(&'static str, &Vec<Value>)> = Vec::new();
    for (key, kind) in [
        ("security_violations", "security"),
        ("vulnerability_violations", "vulnerability"),
        ("sql_violations", "sql"),
    ] {
        if let Some(rows) = optional_array(object, key)? {
            scanner_rows.push((kind, rows));
        }
    }
    if files.is_none() && scanner_rows.is_empty() {
        return Err(MISSING_DOCUMENT.to_owned());
    }
    let mut findings = Vec::new();
    for file in files.into_iter().flatten() {
        let file = file
            .as_object()
            .ok_or_else(|| "check report file entry must be an object".to_owned())?;
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "check report file entry needs a `path`".to_owned())?;
        if let Some(functions) = optional_array(file, "functions")? {
            for function in functions {
                push_function(&mut findings, path, function, limits)?;
            }
        }
        if let Some(errors) = optional_array(file, "parse_errors")? {
            for error in errors {
                let row = error
                    .as_object()
                    .ok_or_else(|| format!("parse error entry of `{path}` must be an object"))?;
                findings.push(Finding {
                    path: first_string(row, &["path", "file"])
                        .unwrap_or(path)
                        .to_owned(),
                    line: row_line(row),
                    end_line: row_end_line(row),
                    kind: "parse_error",
                    severity: "low",
                    message: finding_message(row, "parse_error"),
                    fingerprint: String::new(),
                });
            }
        }
    }
    for (kind, rows) in scanner_rows {
        for row in rows {
            let row = row
                .as_object()
                .ok_or_else(|| format!("`{kind}` violation entry must be an object"))?;
            findings.push(Finding {
                path: row_path(row),
                line: row_line(row),
                end_line: row_end_line(row),
                kind,
                severity: row_severity(row),
                message: finding_message(row, kind),
                fingerprint: String::new(),
            });
        }
    }
    findings.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.kind.cmp(right.kind))
            .then(left.message.cmp(&right.message))
    });
    for finding in &mut findings {
        finding.fingerprint = fingerprint(finding);
    }
    Ok(findings)
}

/// Gate verdict for a saved check document.
///
/// `violations` and `passed` follow the finding set rebuilt with `limits`;
/// `files` is the analyzed file count, or the distinct finding paths when the
/// document is a scanner payload without an analysis report.
pub fn summary_from_check_json(
    document: &Value,
    limits: &GateLimits,
) -> Result<GateSummary, String> {
    let findings = findings_from_check_json(document, limits)?;
    let files = match document.get("files").and_then(Value::as_array) {
        Some(files) => files.len(),
        None => findings
            .iter()
            .filter(|finding| !finding.path.is_empty())
            .map(|finding| finding.path.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
    };
    Ok(GateSummary {
        passed: findings.is_empty(),
        files,
        violations: findings.len(),
    })
}

/// Stable identity of one finding: the first 16 hex characters of the
/// `blake3` hash over `path`, `kind`, and `message`, separated by NUL.
fn fingerprint(finding: &Finding) -> String {
    let source = format!(
        "{}\u{0}{}\u{0}{}",
        finding.path, finding.kind, finding.message
    );
    let mut digest = blake3::hash(source.as_bytes()).to_hex().to_string();
    digest.truncate(16);
    digest
}

/// One finding per function failing at least one limit, in gate order.
///
/// Gate violations are not scanner severities, so the level is the fixed
/// `medium` that the CI formats render as a warning: the limit, not a
/// scanner, decided the row.
fn push_function(
    findings: &mut Vec<Finding>,
    path: &str,
    function: &Value,
    limits: &GateLimits,
) -> Result<(), String> {
    let row = function
        .as_object()
        .ok_or_else(|| format!("function entry of `{path}` must be an object"))?;
    let metrics = match row.get("metrics") {
        Some(Value::Object(metrics)) => metrics,
        Some(_) => {
            return Err(format!(
                "function entry of `{path}` has non-object `metrics`"
            ));
        }
        None => return Ok(()),
    };
    let reasons = violation_reasons(metrics, limits);
    if reasons.is_empty() {
        return Ok(());
    }
    findings.push(Finding {
        path: path.to_owned(),
        line: row_line(row),
        end_line: row_end_line(row),
        kind: "function",
        severity: "medium",
        message: format!("exceeds {}", reasons.join(", ")),
        fingerprint: String::new(),
    });
    Ok(())
}

/// Failed limit names for one metrics object, mirroring
/// [`crate::core::Thresholds::violation_reasons`] order: cognitive,
/// cyclomatic, max_nesting, then CRAP.
///
/// A metric the document does not carry cannot be compared and does not fire,
/// except CRAP: a set CRAP limit fails closed when the document has no CRAP
/// value, which is the same `crap_unavailable` reason the gate reports.
fn violation_reasons(metrics: &Map<String, Value>, limits: &GateLimits) -> Vec<&'static str> {
    let metric = |key: &str| metrics.get(key).and_then(Value::as_f64);
    let mut reasons = Vec::new();
    if limits
        .cognitive
        .is_some_and(|limit| metric("cognitive").is_some_and(|actual| actual > limit))
    {
        reasons.push("cognitive");
    }
    if limits
        .cyclomatic
        .is_some_and(|limit| metric("cyclomatic").is_some_and(|actual| actual > limit))
    {
        reasons.push("cyclomatic");
    }
    if limits
        .max_nesting
        .is_some_and(|limit| metric("max_nesting").is_some_and(|actual| actual > limit))
    {
        reasons.push("max_nesting");
    }
    if let Some(limit) = limits.crap {
        let crap = metric("crap");
        if crap.is_none_or(|actual| actual > limit) {
            reasons.push(if crap.is_some() {
                "crap"
            } else {
                "crap_unavailable"
            });
        }
    }
    reasons
}

/// `None` when the key is absent or null; `Err` when it holds another type.
fn optional_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a Vec<Value>>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(rows)) => Ok(Some(rows)),
        Some(_) => Err(format!("`{key}` must be an array")),
    }
}

/// First non-empty string among `keys`.
fn first_string<'a>(row: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        row.get(*key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
    })
}

/// Finding path: `path`, else `file`, else `manifest_path`, else empty.
fn row_path(row: &Map<String, Value>) -> String {
    first_string(row, &["path", "file", "manifest_path"])
        .unwrap_or_default()
        .to_owned()
}

/// 1-based line: `line`, else `start_line`, else 1.
fn row_line(row: &Map<String, Value>) -> u32 {
    first_line(row, &["line", "start_line"]).unwrap_or(1)
}

/// End line when the row carries one.
fn row_end_line(row: &Map<String, Value>) -> Option<u32> {
    first_line(row, &["end_line"])
}

fn first_line(row: &Map<String, Value>, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        row.get(*key)
            .and_then(Value::as_u64)
            .and_then(|line| u32::try_from(line).ok())
    })
}

/// Normalized severity; any other level is `unknown`.
fn row_severity(row: &Map<String, Value>) -> &'static str {
    match row.get("severity").and_then(Value::as_str) {
        Some("critical") => "critical",
        Some("high") => "high",
        Some("medium") => "medium",
        Some("low") => "low",
        _ => "unknown",
    }
}

/// Message for one scanner or parse-error row: the row's own text when it
/// carries any, else a fixed fallback. Scanner source text never crosses over
/// beyond the fields leadline already normalized.
fn finding_message(row: &Map<String, Value>, kind: &str) -> String {
    let keys: &[&str] = match kind {
        "parse_error" => &["message", "kind"],
        "vulnerability" => &["message", "title", "advisory_id", "id"],
        "sql" => &["message", "title", "remediation", "rule_id", "rule"],
        _ => &["message", "title", "rule_id", "rule", "id"],
    };
    if let Some(text) = first_string(row, keys) {
        return text.to_owned();
    }
    match kind {
        "parse_error" => "parse error".to_owned(),
        "vulnerability" => match row.get("package").and_then(Value::as_str) {
            Some(package) => format!("vulnerable dependency {package}"),
            None => "vulnerable dependency".to_owned(),
        },
        "sql" => "postgresql risk".to_owned(),
        _ => "security finding".to_owned(),
    }
}

/// GitLab Code Quality / CodeClimate JSON array.
fn codeclimate(findings: &[&Finding]) -> String {
    let issues: Vec<Value> = findings
        .iter()
        .map(|finding| {
            let mut lines = serde_json::json!({ "begin": finding.line });
            if let Some(end) = finding.end_line.filter(|end| *end >= finding.line) {
                lines["end"] = Value::from(end);
            }
            serde_json::json!({
                "type": "issue",
                "check_name": finding.kind,
                "description": finding.message,
                "categories": ["Bug Risk"],
                "severity": codeclimate_severity(finding.severity),
                "location": { "path": finding.path, "lines": lines },
                "fingerprint": finding.fingerprint,
            })
        })
        .collect();
    let mut output = Value::Array(issues).to_string();
    output.push('\n');
    output
}

/// CodeClimate severity vocabulary; `critical` and `high` both mean the most
/// severe CI levels, and anything unrecognized is informational.
fn codeclimate_severity(severity: &str) -> &'static str {
    match severity {
        "critical" => "blocker",
        "high" => "critical",
        "medium" => "major",
        "low" => "minor",
        _ => "info",
    }
}

/// GitHub workflow commands, one annotation per finding.
fn github_annotations(findings: &[&Finding]) -> String {
    let mut output = String::new();
    for finding in findings {
        let level = match finding.severity {
            "critical" | "high" => "error",
            _ => "warning",
        };
        let _ = writeln!(
            output,
            "::{level} file={},line={},title={}::{}",
            escape_property(&finding.path),
            finding.line,
            escape_property(finding.kind),
            escape_data(&finding.message)
        );
    }
    output
}

/// Workflow-command escaping for message data: `%`, CR, and LF.
fn escape_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Property escaping adds the `,` and `:` separators.
fn escape_property(text: &str) -> String {
    escape_data(text).replace(',', "%2C").replace(':', "%3A")
}

/// Markdown tables; `github-summary` is this document.
fn markdown(findings: &[&Finding], summary: &GateSummary) -> String {
    let verdict = if summary.passed { "passing" } else { "failing" };
    let mut output = String::from("## leadline check\n\n");
    let _ = writeln!(
        output,
        "**{verdict}** - {} {} in {} {}\n",
        summary.violations,
        plural(summary.violations, "violation", "violations"),
        summary.files,
        plural(summary.files, "file", "files"),
    );
    if findings.is_empty() {
        output.push_str("No findings.\n");
        return output;
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for finding in findings {
        *counts.entry(finding.kind).or_default() += 1;
    }
    output.push_str("| Kind | Count |\n| --- | --- |\n");
    for (kind, count) in &counts {
        let _ = writeln!(output, "| {} | {count} |", cell(kind));
    }
    output.push_str(
        "\n| Path | Line | Kind | Severity | Message |\n| --- | --- | --- | --- | --- |\n",
    );
    for finding in findings {
        let _ = writeln!(
            output,
            "| {} | {} | {} | {} | {} |",
            cell(&finding.path),
            finding.line,
            cell(finding.kind),
            cell(finding.severity),
            cell(&finding.message)
        );
    }
    output
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// One markdown table cell: pipes are escaped and newlines flattened so a
/// message cannot break the table.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

/// Shields.io-style SVG: the verdict and the violation count, nothing else.
///
/// Widths come from a fixed per-character advance, so the bytes never depend
/// on fonts, timestamps, or generated identifiers.
fn badge(summary: &GateSummary) -> String {
    let (color, verdict) = if summary.passed {
        ("#4c1", "passing")
    } else {
        ("#e05d44", "failing")
    };
    let label = "leadline";
    let message = format!("{verdict} {}", summary.violations);
    let label_width = text_width(label);
    let message_width = text_width(&message);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"20\" role=\"img\" \
         aria-label=\"{label}: {message}\">\n\
         <title>{label}: {message}</title>\n\
         <g shape-rendering=\"crispEdges\">\n\
         <rect width=\"{label_width}\" height=\"20\" fill=\"#555\"/>\n\
         <rect x=\"{label_width}\" width=\"{message_width}\" height=\"20\" fill=\"{color}\"/>\n\
         </g>\n\
         <g fill=\"#fff\" text-anchor=\"middle\" font-family=\"Verdana,Geneva,DejaVu Sans,sans-serif\" \
         font-size=\"11\">\n\
         <text x=\"{}\" y=\"14\">{label}</text>\n\
         <text x=\"{}\" y=\"14\">{message}</text>\n\
         </g>\n\
         </svg>\n",
        label_width + message_width,
        label_width / 2,
        label_width + message_width / 2,
    )
}

const CHARACTER_WIDTH: usize = 7;
const BADGE_PADDING: usize = 10;

fn text_width(text: &str) -> usize {
    text.chars().count() * CHARACTER_WIDTH + BADGE_PADDING
}

/// One grep-friendly line per finding.
fn compact(findings: &[&Finding]) -> String {
    let mut output = String::new();
    for finding in findings {
        let _ = writeln!(
            output,
            "{}:{} {} {} {}",
            finding.path,
            finding.line,
            finding.kind,
            finding.severity,
            flatten(&finding.message)
        );
    }
    output
}

/// Newlines flattened so one finding stays on one line.
fn flatten(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}
