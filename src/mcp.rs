//! Local stdio MCP-style server (hand-rolled JSON-RPC 2.0, no SDK).
//!
//! Read-only by construction: the only filesystem reads are source-file
//! analysis input, an optional coverage file, and the `git` reads already
//! performed inside [`crate::diff::analyze_changes`]. No writes, no shell,
//! no network.
//!
//! Wire format: newline-delimited JSON-RPC 2.0 over stdin/stdout.
//! Requests look like `{jsonrpc:"2.0", id, method, params}`; responses are
//! `{jsonrpc:"2.0", id, result}` or `{jsonrpc:"2.0", id, error:{code,message}}`.
//! Messages without an `id` are notifications and produce no response.

use crate::agent::{SortKey, changed_causes};
use crate::config::RegressionLimits;
use crate::core::{
    FunctionAnalysis, FunctionMetrics, METRIC_PROFILE, OUTPUT_SCHEMA_VERSION, Thresholds,
};
use std::collections::BTreeSet;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};

/// Maximum entries returned in any result array before truncation kicks in.
const MAX_ENTRIES: usize = 200;

/// Usage guidance returned by `initialize`. Hosts may inject this into the
/// system prompt, so it doubles as the server's self-advertisement.
const SERVER_INSTRUCTIONS: &str = "leadline reports deterministic function-level complexity metrics (Java, JavaScript, TypeScript, TSX) without executing code or touching the network. Use analyze_changed after substantial edits to spot regressions, repo_summary for a first look at unfamiliar code, analyze_function with explain:true to see which lines drive complexity, check to gate thresholds or regressions, and test_targets to rank uncovered decision lines. Metrics are evidence, not objectives: do not refactor solely to lower a number.";

/// The seven tools this server exposes. Fixed set; keep in sync with
/// [`tools_list`] and [`dispatch_tool`].
const TOOL_NAMES: [&str; 7] = [
    "analyze",
    "analyze_changed",
    "analyze_function",
    "check",
    "explain_metric",
    "repo_summary",
    "test_targets",
];

/// Serve JSON-RPC requests from stdin, writing responses to stdout.
///
/// One request per line. Batch arrays are accepted when a whole line parses
/// as a JSON array. Blank lines are ignored. Responses are flushed after
/// every line: live clients keep stdin open while waiting, so buffering
/// until EOF would deadlock them into a request timeout.
pub fn serve() -> crate::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_request(&line) {
            use std::io::Write as _;
            writeln!(out, "{response}")?;
            out.flush()?;
        }
    }
    use std::io::Write as _;
    out.flush()?;
    Ok(())
}

/// Handle one raw input line, returning the response line if any.
///
/// Pure function (no I/O beyond analysis reads) so tests can drive it
/// directly without subprocesses. Returns `None` for notifications
/// (requests without an `id`) and for blank input.
pub fn handle_request(raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => return Some(parse_error_response()),
    };
    if let serde_json::Value::Array(batch) = value {
        let mut responses = Vec::new();
        for item in &batch {
            if let Some(response) = handle_single(item) {
                responses.push(response);
            }
        }
        if responses.is_empty() {
            return None;
        }
        return Some(serde_json::Value::Array(responses).to_string());
    }
    handle_single(&value).map(|response| response.to_string())
}

fn handle_single(request: &serde_json::Value) -> Option<serde_json::Value> {
    let object = match request.as_object() {
        Some(object) => object,
        None => {
            return Some(error_response(
                serde_json::Value::Null,
                -32600,
                "Invalid Request",
            ));
        }
    };
    // No `id` => notification => no response.
    if !object.contains_key("id") {
        return None;
    }
    let id = object.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let jsonrpc = object.get("jsonrpc").and_then(serde_json::Value::as_str);
    let method = object.get("method").and_then(serde_json::Value::as_str);
    if jsonrpc != Some("2.0") || method.is_none() {
        return Some(error_response(id, -32600, "Invalid Request"));
    }
    let method = method.unwrap_or_default().to_owned();
    // `initialized` and `notifications/*` are acknowledgements.
    if method == "initialized" || method.starts_with("notifications/") {
        return Some(success_response(id, serde_json::json!({})));
    }
    let params = object
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    match method.as_str() {
        "initialize" => Some(success_response(id, initialize_result(&params))),
        "ping" => Some(success_response(id, serde_json::json!({}))),
        "tools/list" => Some(success_response(id, tools_list_result())),
        "tools/call" => match dispatch_tools_call(&params) {
            Ok(result) => Some(success_response(id, tool_result(result))),
            Err((code, message)) => Some(error_response(id, code, &message)),
        },
        name if TOOL_NAMES.contains(&name) => match dispatch_tool(name, &params) {
            Ok(result) => Some(success_response(id, tool_result(result))),
            Err((code, message)) => Some(error_response(id, code, &message)),
        },
        _ => Some(error_response(id, -32601, "Method not found")),
    }
}

/// Wrap a tool payload in the MCP `CallToolResult` shape: a text content
/// block for hosts that render text, plus `structuredContent` so code-mode
/// style hosts can hand agents native JSON. Clients that receive neither
/// surface a null result, so this envelope is required, not cosmetic.
fn tool_result(payload: serde_json::Value) -> serde_json::Value {
    let text = match &payload {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    serde_json::json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": payload,
    })
}

fn dispatch_tools_call(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let object = params
        .as_object()
        .ok_or((-32602, "tools/call requires {name, arguments}".to_owned()))?;
    let name = object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or((-32602, "tools/call requires a string name".to_owned()))?;
    if !TOOL_NAMES.contains(&name) {
        return Err((-32602, format!("unknown tool '{name}'")));
    }
    let arguments = object
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    dispatch_tool(name, &arguments)
}

fn dispatch_tool(
    name: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, (i64, String)> {
    let params = if params.is_null() {
        &serde_json::Value::Object(serde_json::Map::new())
    } else {
        params
    };
    match name {
        "analyze" => tool_analyze(params),
        "analyze_changed" => tool_analyze_changed(params),
        "analyze_function" => tool_analyze_function(params),
        "check" => tool_check(params),
        "explain_metric" => tool_explain_metric(params),
        "repo_summary" => tool_repo_summary(params),
        "test_targets" => tool_test_targets(params),
        _ => Err((-32602, format!("unknown tool '{name}'"))),
    }
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

fn metric_specs() -> Result<serde_json::Value, (i64, String)> {
    serde_json::to_value(crate::core::MetricSpecs::default())
        .map_err(|error| (-32603, format!("internal error: {error}")))
}

/// Every tool result carries the version envelope so agents can pin behavior.
fn envelope(
    mut fields: serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, (i64, String)> {
    fields.insert(
        "schema_version".to_owned(),
        serde_json::Value::from(OUTPUT_SCHEMA_VERSION),
    );
    fields.insert(
        "analyzer_version".to_owned(),
        serde_json::Value::from(env!("CARGO_PKG_VERSION")),
    );
    fields.insert("metric_specs".to_owned(), metric_specs()?);
    Ok(serde_json::Value::Object(fields))
}

fn envelope_fields() -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::new()
}

// ---------------------------------------------------------------------------
// Compact rows
// ---------------------------------------------------------------------------

/// Trimmed agent-facing function row: identity + spans + the metrics an
/// agent acts on. Intentionally mirrors the agent-json field set.
fn compact_function(path: &str, function: &FunctionAnalysis) -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("name".to_owned(), serde_json::json!(function.name));
    fields.insert("line".to_owned(), serde_json::json!(function.start_line));
    fields.insert(
        "start_line".to_owned(),
        serde_json::json!(function.start_line),
    );
    fields.insert("end_line".to_owned(), serde_json::json!(function.end_line));
    fields.extend(metric_fields(&function.metrics));
    serde_json::Value::Object(fields)
}

fn compact_metrics(value: &FunctionAnalysis) -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    fields.insert("line".to_owned(), serde_json::json!(value.start_line));
    fields.extend(metric_fields(&value.metrics));
    serde_json::Value::Object(fields)
}

/// The shared metric field set both compact rows carry.
fn metric_fields(metrics: &FunctionMetrics) -> serde_json::Map<String, serde_json::Value> {
    let mut fields = serde_json::Map::new();
    fields.insert("cognitive".to_owned(), serde_json::json!(metrics.cognitive));
    fields.insert(
        "cyclomatic".to_owned(),
        serde_json::json!(metrics.cyclomatic),
    );
    fields.insert("crap".to_owned(), round1_opt(metrics.crap));
    fields.insert("coverage".to_owned(), round1_opt(metrics.coverage));
    fields
}

fn round1_opt(value: Option<f64>) -> serde_json::Value {
    match value {
        Some(number) if number.is_finite() => {
            serde_json::json!((number * 10.0).round() / 10.0)
        }
        _ => serde_json::Value::Null,
    }
}

/// Cap a result array at the default bound, reporting truncation explicitly
/// instead of silently dropping entries.
fn cap(entries: &mut Vec<serde_json::Value>) -> (Vec<serde_json::Value>, bool, usize) {
    cap_with_limit(entries, MAX_ENTRIES)
}

/// Cap a result array at `limit`. Returns `(kept, truncated, total)`.
fn cap_with_limit(
    entries: &mut Vec<serde_json::Value>,
    limit: usize,
) -> (Vec<serde_json::Value>, bool, usize) {
    let total = entries.len();
    if total > limit {
        (entries[..limit].to_vec(), true, total)
    } else {
        (std::mem::take(entries), false, total)
    }
}

/// Output budget for list-shaped tools, mirroring the CLI agent-json flags.
#[derive(Clone, Copy, Debug, Default)]
struct ToolBudget {
    top: Option<usize>,
    sort_by: Option<SortKey>,
    min_crap: Option<f64>,
    min_delta: Option<f64>,
}

impl ToolBudget {
    fn limit(&self) -> usize {
        self.top.unwrap_or(MAX_ENTRIES)
    }
}

fn parse_tool_budget(
    params: &serde_json::Value,
    allow_min_delta: bool,
) -> Result<ToolBudget, (i64, String)> {
    let top = match params.get("top") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            let count = value
                .as_u64()
                .ok_or((-32602, "top must be a positive integer".to_owned()))?;
            let count =
                usize::try_from(count).map_err(|_| (-32602, "top must fit in usize".to_owned()))?;
            if count == 0 {
                return Err((-32602, "top must be at least 1".to_owned()));
            }
            Some(count)
        }
    };
    let sort_by = match opt_str(params, "sort_by") {
        None => None,
        Some(value) => Some(SortKey::parse(value).ok_or((
            -32602,
            format!("unknown sort_by '{value}': expected 'crap', 'cognitive', or 'cyclomatic'"),
        ))?),
    };
    let min_crap = parse_optional_f64(params, "min_crap")?;
    let min_delta = if allow_min_delta {
        parse_optional_f64(params, "min_delta")?
    } else {
        None
    };
    Ok(ToolBudget {
        top,
        sort_by,
        min_crap,
        min_delta,
    })
}

fn parse_optional_f64(params: &serde_json::Value, key: &str) -> Result<Option<f64>, (i64, String)> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let number = value
                .as_f64()
                .ok_or((-32602, format!("{key} must be a number")))?;
            if !number.is_finite() {
                return Err((-32602, format!("{key} must be finite")));
            }
            Ok(Some(number))
        }
    }
}

fn row_u32(row: &serde_json::Value, key: &str) -> u32 {
    row.get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32
}

fn row_f64(row: &serde_json::Value, key: &str) -> Option<f64> {
    row.get(key).and_then(serde_json::Value::as_f64)
}

/// Descending order by metric; missing values sort last.
fn cmp_desc_f64(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (Some(left), Some(right)) => right.total_cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn row_position(row: &serde_json::Value) -> (&str, &str, u32) {
    (
        opt_str(row, "path").unwrap_or_default(),
        opt_str(row, "name").unwrap_or_default(),
        row_u32(row, "line"),
    )
}

/// Sort compact `analyze` rows: primary metric descending (missing CRAP
/// last), then path/name/line for determinism.
fn sort_analyze_rows(rows: &mut [serde_json::Value], key: SortKey) {
    rows.sort_by(|left, right| {
        let primary = match key {
            SortKey::Crap => cmp_desc_f64(row_f64(left, "crap"), row_f64(right, "crap")),
            SortKey::Cognitive => row_u32(right, "cognitive").cmp(&row_u32(left, "cognitive")),
            SortKey::Cyclomatic => row_u32(right, "cyclomatic").cmp(&row_u32(left, "cyclomatic")),
        };
        primary.then_with(|| row_position(left).cmp(&row_position(right)))
    });
}

/// The current-side metrics of a changed row: after when present, else before.
fn changed_current(row: &serde_json::Value) -> &serde_json::Value {
    row.get("after")
        .filter(|value| !value.is_null())
        .or_else(|| row.get("before"))
        .unwrap_or(row)
}

/// Max absolute delta across cognitive, cyclomatic, and CRAP; 0.0 when the
/// row pairs no numeric delta (added or removed functions).
fn changed_max_delta(row: &serde_json::Value) -> f64 {
    let delta = row.get("delta");
    ["cognitive", "cyclomatic", "crap"]
        .iter()
        .filter_map(|key| {
            delta
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_f64)
        })
        .map(f64::abs)
        .fold(0.0, f64::max)
}

/// Sort changed rows by current-side metric descending.
fn sort_changed_rows(rows: &mut [serde_json::Value], key: SortKey) {
    rows.sort_by(|left, right| {
        let left_metric = changed_current(left);
        let right_metric = changed_current(right);
        let primary = match key {
            SortKey::Crap => {
                cmp_desc_f64(row_f64(left_metric, "crap"), row_f64(right_metric, "crap"))
            }
            SortKey::Cognitive => {
                row_u32(right_metric, "cognitive").cmp(&row_u32(left_metric, "cognitive"))
            }
            SortKey::Cyclomatic => {
                row_u32(right_metric, "cyclomatic").cmp(&row_u32(left_metric, "cyclomatic"))
            }
        };
        primary.then_with(|| row_position(left).cmp(&row_position(right)))
    });
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

fn tool_analyze(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let path = opt_str(params, "path").unwrap_or(".");
    let budget = parse_tool_budget(params, false)?;
    let coverage_path = opt_str(params, "coverage");
    let coverage = coverage_path
        .map(load_coverage_file)
        .transpose()
        .map_err(|message| (-32602, message))?;
    let report = crate::analyze_path(Path::new(path), coverage.as_ref())
        .map_err(|error| (-32602, error.to_string()))?;
    let mut rows = Vec::new();
    for file in &report.files {
        for function in &file.functions {
            rows.push(compact_function(&file.path, function));
        }
    }
    if let Some(floor) = budget.min_crap {
        rows.retain(|row| row_f64(row, "crap").is_some_and(|score| score >= floor));
    }
    if let Some(key) = budget.sort_by {
        sort_analyze_rows(&mut rows, key);
    }
    let (functions, truncated, total) = cap_with_limit(&mut rows, budget.limit());
    let mut fields = envelope_fields();
    fields.insert("tool".to_owned(), serde_json::json!("analyze"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("functions".to_owned(), serde_json::Value::Array(functions));
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    envelope(fields)
}

fn tool_analyze_changed(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let base = opt_str(params, "base").unwrap_or("HEAD~1");
    let path = opt_str(params, "path").unwrap_or(".");
    let target = match opt_str(params, "target") {
        None | Some("worktree") => crate::diff::ComparisonTarget::Worktree,
        Some("index") => crate::diff::ComparisonTarget::Index,
        Some(revision) => crate::diff::ComparisonTarget::Revision(revision.to_owned()),
    };
    let detect_renames = params
        .get("renames")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let explain = params
        .get("explain")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let budget = parse_tool_budget(params, true)?;
    let options = crate::diff::ChangeOptions {
        base: base.to_owned(),
        target,
        detect_renames,
    };
    let report = crate::diff::analyze_changes(Path::new(path), &options)
        .map_err(|error| (-32602, error.to_string()))?;
    let mut rows = Vec::new();
    for change in &report.functions {
        let line = change
            .after
            .as_ref()
            .or(change.before.as_ref())
            .map_or(0, |function| function.start_line);
        let (start_line, end_line) = change
            .after
            .as_ref()
            .or(change.before.as_ref())
            .map_or((line, line), |function| {
                (function.start_line, function.end_line)
            });
        let mut row = serde_json::json!({
            "path": change.path,
            "name": change.name,
            "line": line,
            "start_line": start_line,
            "end_line": end_line,
            "before": change.before.as_ref().map(compact_metrics),
            "after": change.after.as_ref().map(compact_metrics),
            "delta": change_delta(change.before.as_ref(), change.after.as_ref()),
        });
        if explain
            && let Some(causes) = changed_causes(change.before.as_ref(), change.after.as_ref())
        {
            row["causes"] = causes;
        }
        rows.push(row);
    }
    if let Some(floor) = budget.min_crap {
        rows.retain(|row| {
            row_f64(changed_current(row), "crap").is_some_and(|score| score >= floor)
        });
    }
    if let Some(floor) = budget.min_delta {
        rows.retain(|row| changed_max_delta(row) >= floor);
    }
    if let Some(key) = budget.sort_by {
        sort_changed_rows(&mut rows, key);
    }
    let (changes, truncated, total) = cap_with_limit(&mut rows, budget.limit());
    let mut fields = envelope_fields();
    fields.insert("tool".to_owned(), serde_json::json!("analyze_changed"));
    fields.insert("base".to_owned(), serde_json::json!(report.base));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("changes".to_owned(), serde_json::Value::Array(changes));
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    envelope(fields)
}

fn change_delta(
    before: Option<&FunctionAnalysis>,
    after: Option<&FunctionAnalysis>,
) -> serde_json::Value {
    match (before, after) {
        (Some(left), Some(right)) => {
            let crap = match (left.metrics.crap, right.metrics.crap) {
                (Some(a), Some(b)) if a.is_finite() && b.is_finite() => {
                    serde_json::json!(((b - a) * 10.0).round() / 10.0)
                }
                _ => serde_json::Value::Null,
            };
            serde_json::json!({
                "cognitive": right.metrics.cognitive as i64 - left.metrics.cognitive as i64,
                "cyclomatic": right.metrics.cyclomatic as i64 - left.metrics.cyclomatic as i64,
                "crap": crap,
            })
        }
        _ => serde_json::Value::Null,
    }
}

fn tool_analyze_function(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let path = req_str(params, "path")?;
    let function = req_str(params, "function")?;
    let file = PathBuf::from(path);
    let root = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut analyzed =
        crate::analyze_file(&file, &root, None).map_err(|error| (-32602, error.to_string()))?;
    analyzed.path = crate::normalize_path(&file);
    analyzed
        .functions
        .retain(|candidate| candidate.name == function);
    if analyzed.functions.is_empty() {
        return Err((
            -32602,
            format!("function '{function}' was not found in {path}"),
        ));
    }
    let mut rows: Vec<serde_json::Value> = analyzed
        .functions
        .iter()
        .map(|item| compact_function(&analyzed.path, item))
        .collect();
    let explain = params
        .get("explain")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if explain {
        for (row, function) in rows.iter_mut().zip(&analyzed.functions) {
            row["contributions"] =
                serde_json::to_value(&function.contributions).unwrap_or(serde_json::Value::Null);
        }
    }
    let total = rows.len();
    let (functions, truncated, _) = cap(&mut rows);
    let mut fields = envelope_fields();
    fields.insert("tool".to_owned(), serde_json::json!("analyze_function"));
    fields.insert("path".to_owned(), serde_json::json!(analyzed.path));
    fields.insert("function".to_owned(), serde_json::json!(function));
    fields.insert("functions".to_owned(), serde_json::Value::Array(functions));
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    envelope(fields)
}

fn tool_check(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let path = opt_str(params, "path").unwrap_or(".");
    let regression_limits = parse_regression_limits(params)?;
    let base = opt_str(params, "base");
    let baseline_path = opt_str(params, "baseline");
    if base.is_some() && baseline_path.is_some() {
        return Err((-32602, "base and baseline are exclusive".to_owned()));
    }
    if regression_limits.is_some() && base.is_none() && baseline_path.is_none() {
        return Err((
            -32602,
            "check regressions requires a base revision or baseline file".to_owned(),
        ));
    }
    let thresholds = parse_thresholds(params, regression_limits.is_some())?;
    let coverage = opt_str(params, "coverage")
        .map(load_coverage_file)
        .transpose()
        .map_err(|message| (-32602, message))?;
    let mut rows = Vec::new();
    if let Some(base) = base {
        let mut report = crate::diff::analyze_changed(Path::new(path), base)
            .map_err(|error| (-32602, error.to_string()))?;
        if let Some(coverage) = coverage.as_ref() {
            for change in &mut report.functions {
                if let Some(before) = change.before.as_mut() {
                    coverage.apply_function(&change.path, before);
                }
                if let Some(after) = change.after.as_mut() {
                    coverage.apply_function(&change.path, after);
                }
            }
        }
        for change in &report.functions {
            let Some(after) = change.after.as_ref() else {
                continue;
            };
            let absolute = thresholds.violates(&after.metrics);
            let delta = regression_limits.as_ref().is_some_and(|limits| {
                change
                    .before
                    .as_ref()
                    .is_some_and(|before| crate::diff::regression_violates(before, after, limits))
            });
            if absolute || delta {
                rows.push(compact_function(&change.path, after));
            }
        }
    } else if let Some(baseline_path) = baseline_path {
        let baseline = crate::baseline::Baseline::read(Path::new(baseline_path))
            .map_err(|error| (-32602, error.to_string()))?;
        let report = crate::analyze_path(Path::new(path), coverage.as_ref())
            .map_err(|error| (-32602, error.to_string()))?;
        let regressed: BTreeSet<(String, String)> = match &regression_limits {
            Some(limits) => baseline
                .compare(&report, limits)
                .iter()
                .map(|finding| (finding.path.clone(), finding.id.clone()))
                .collect(),
            None => BTreeSet::new(),
        };
        for file in &report.files {
            for function in &file.functions {
                let absolute = thresholds.violates(&function.metrics);
                let delta = regression_limits.is_some()
                    && regressed.contains(&(file.path.clone(), function.id.clone()));
                if absolute || delta {
                    rows.push(compact_function(&file.path, function));
                }
            }
        }
    } else {
        let report = crate::analyze_path(Path::new(path), coverage.as_ref())
            .map_err(|error| (-32602, error.to_string()))?;
        for file in &report.files {
            for function in &file.functions {
                if thresholds.violates(&function.metrics) {
                    rows.push(compact_function(&file.path, function));
                }
            }
        }
    }
    let passed = rows.is_empty();
    let (violations, truncated, total) = cap(&mut rows);
    let mut fields = envelope_fields();
    fields.insert("tool".to_owned(), serde_json::json!("check"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    if let Some(base) = base {
        fields.insert("base".to_owned(), serde_json::json!(base));
    }
    if let Some(baseline_path) = baseline_path {
        fields.insert("baseline".to_owned(), serde_json::json!(baseline_path));
    }
    fields.insert(
        "thresholds".to_owned(),
        serde_json::json!({
            "cognitive": thresholds.cognitive,
            "cyclomatic": thresholds.cyclomatic,
            "crap": thresholds.crap,
            "max_nesting": thresholds.max_nesting,
        }),
    );
    fields.insert(
        "regressions".to_owned(),
        serde_json::json!(regression_limits.as_ref().map(|limits| {
            serde_json::json!({
                "cognitive": limits.cognitive,
                "cyclomatic": limits.cyclomatic,
                "crap": limits.crap,
                "max_nesting": limits.max_nesting,
            })
        })),
    );
    fields.insert("passed".to_owned(), serde_json::Value::from(passed));
    fields.insert(
        "violations".to_owned(),
        serde_json::Value::Array(violations),
    );
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    envelope(fields)
}

fn parse_regression_limits(
    params: &serde_json::Value,
) -> Result<Option<RegressionLimits>, (i64, String)> {
    let Some(value) = params.get("regressions") else {
        return Ok(None);
    };
    if value.is_null() || value == &serde_json::Value::Bool(false) {
        return Ok(None);
    }
    if value == &serde_json::Value::Bool(true) {
        return Ok(Some(RegressionLimits::default()));
    }
    let object = value
        .as_object()
        .ok_or((-32602, "regressions must be true or an object".to_owned()))?;
    let mut limits = RegressionLimits::default();
    for key in ["cognitive", "cyclomatic", "max_nesting"] {
        if let Some(value) = object.get(key) {
            let limit = value.as_u64().ok_or((
                -32602,
                format!("regressions.{key} must be a non-negative integer"),
            ))?;
            let limit = u32::try_from(limit)
                .map_err(|_| (-32602, format!("regressions.{key} must fit in u32")))?;
            match key {
                "cognitive" => limits.cognitive = limit,
                "cyclomatic" => limits.cyclomatic = limit,
                _ => limits.max_nesting = limit,
            }
        }
    }
    if let Some(value) = object.get("crap") {
        let limit = value.as_f64().ok_or((
            -32602,
            "regressions.crap must be a non-negative number".to_owned(),
        ))?;
        if !limit.is_finite() || limit < 0.0 {
            return Err((
                -32602,
                "regressions.crap must be a non-negative number".to_owned(),
            ));
        }
        limits.crap = limit;
    }
    Ok(Some(limits))
}

fn parse_thresholds(
    params: &serde_json::Value,
    allow_empty: bool,
) -> Result<Thresholds, (i64, String)> {
    let object = match params.get("thresholds") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_object()
                .ok_or((-32602, "thresholds must be an object".to_owned()))?,
        ),
    };
    let mut thresholds = Thresholds {
        cognitive: None,
        cyclomatic: None,
        crap: None,
        max_nesting: None,
    };
    if let Some(object) = object {
        for key in ["cognitive", "cyclomatic", "crap", "max_nesting"] {
            if let Some(value) = object.get(key) {
                if value.is_null() {
                    continue;
                }
                match key {
                    "crap" => {
                        let limit = value.as_f64().ok_or((
                            -32602,
                            "thresholds.crap must be a non-negative number".to_owned(),
                        ))?;
                        if !limit.is_finite() || limit < 0.0 {
                            return Err((
                                -32602,
                                "thresholds.crap must be a non-negative number".to_owned(),
                            ));
                        }
                        thresholds.crap = Some(limit);
                    }
                    _ => {
                        let limit = value.as_u64().ok_or((
                            -32602,
                            format!("thresholds.{key} must be a non-negative integer"),
                        ))?;
                        let limit = u32::try_from(limit)
                            .map_err(|_| (-32602, format!("thresholds.{key} must fit in u32")))?;
                        match key {
                            "cognitive" => thresholds.cognitive = Some(limit),
                            "cyclomatic" => thresholds.cyclomatic = Some(limit),
                            _ => thresholds.max_nesting = Some(limit),
                        }
                    }
                }
            }
        }
    }
    if thresholds.is_empty() && !allow_empty {
        return Err((-32602, "check requires at least one threshold".to_owned()));
    }
    Ok(thresholds)
}

fn tool_explain_metric(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let metric = req_str(params, "metric")?;
    let name = metric.to_ascii_lowercase();
    let definition = match name.as_str() {
        "cyclomatic" => {
            "Cyclomatic complexity (default): every function starts at 1. Add 1 for each `if`, loop, `catch`, non-default switch case, ternary expression, `&&`, `||`, and JavaScript/TypeScript `??`. `else`, `finally`, default cases, functions, and lambdas add nothing. Nested functions are scored independently."
        }
        "cognitive" => {
            "Cognitive complexity (default): add `1 + current nesting` for `if`, loops, `catch`, `switch`, and ternary expressions. An `else if` adds 1 and continues the chain; a final `else` adds 1; a labeled `break`/`continue` adds 1. For logical expressions the first `&&`/`||`/`??` adds 1 and each operator change adds 1. Lambdas, arrows, and nested functions are scored independently and do not raise the enclosing score. Recursion is not scored."
        }
        "halstead" => {
            "Halstead metrics (default): leaf operator tokens and control keywords are operators; identifiers, literals, `this`, `super`, `true`, `false`, and `null` are operands. Distinct values are source byte slices within one function. Vocabulary is n1 + n2, length is N1 + N2, volume is length * log2(vocabulary), difficulty is (n1 / 2) * (N2 / n2), effort is difficulty * volume. Zero denominators produce zero, never NaN."
        }
        "maintainability" => {
            "Maintainability index (default): max(0, (171 - 5.2 ln(volume) - 0.23 cyclomatic - 16.2 ln(LOC)) * 100 / 171). Volume and LOC use a floor of 1, the result is capped at 100, and comment weighting is not used. Higher is better."
        }
        "crap" => {
            "CRAP (default): c^2 * (1 - p)^3 + c, where c is cyclomatic complexity and p is normalized function coverage (0.0..1.0). CRAP is null when no coverage line overlaps the function range, never a fake 0%. Lower is better."
        }
        _ => {
            return Err((
                -32602,
                format!(
                    "unknown metric '{metric}': expected cyclomatic|cognitive|halstead|maintainability|crap"
                ),
            ));
        }
    };
    let mut fields = envelope_fields();
    fields.insert("tool".to_owned(), serde_json::json!("explain_metric"));
    fields.insert("metric".to_owned(), serde_json::json!(name));
    fields.insert("spec".to_owned(), serde_json::json!(METRIC_PROFILE));
    fields.insert("definition".to_owned(), serde_json::Value::from(definition));
    envelope(fields)
}

fn tool_repo_summary(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let path = opt_str(params, "path").unwrap_or(".");
    let top = match params.get("top") {
        None | Some(serde_json::Value::Null) => 5,
        Some(value) => {
            let count = value
                .as_u64()
                .ok_or((-32602, "top must be a positive integer".to_owned()))?;
            let count =
                usize::try_from(count).map_err(|_| (-32602, "top must fit in usize".to_owned()))?;
            if count == 0 {
                return Err((-32602, "top must be at least 1".to_owned()));
            }
            count.min(50)
        }
    };
    let report =
        crate::analyze_path(Path::new(path), None).map_err(|error| (-32602, error.to_string()))?;
    let files = report.files.len();
    let mut functions = 0usize;
    let mut parse_errors = 0usize;
    let mut rows = Vec::new();
    for file in &report.files {
        parse_errors += file.parse_errors.len();
        for function in &file.functions {
            functions += 1;
            rows.push(compact_function(&file.path, function));
        }
    }
    let top_rows = |key: SortKey| {
        let mut selected = rows.clone();
        sort_analyze_rows(&mut selected, key);
        selected.truncate(top);
        selected
    };
    let mut fields = envelope_fields();
    fields.insert("tool".to_owned(), serde_json::json!("repo_summary"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert(
        "totals".to_owned(),
        serde_json::json!({
            "files": files,
            "functions": functions,
            "parse_errors": parse_errors,
        }),
    );
    fields.insert(
        "top".to_owned(),
        serde_json::json!({
            "crap": top_rows(SortKey::Crap),
            "cognitive": top_rows(SortKey::Cognitive),
            "cyclomatic": top_rows(SortKey::Cyclomatic),
        }),
    );
    fields.insert(
        "truncated".to_owned(),
        serde_json::Value::from(functions > top),
    );
    envelope(fields)
}

fn tool_test_targets(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let path = opt_str(params, "path").unwrap_or(".");
    let coverage_path = opt_str(params, "coverage").ok_or((
        -32602,
        "test_targets requires a coverage file path".to_owned(),
    ))?;
    let top = match params.get("top") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            let count = value
                .as_u64()
                .ok_or((-32602, "top must be a positive integer".to_owned()))?;
            let count =
                usize::try_from(count).map_err(|_| (-32602, "top must fit in usize".to_owned()))?;
            if count == 0 {
                return Err((-32602, "top must be at least 1".to_owned()));
            }
            Some(count)
        }
    };
    let coverage = load_coverage_file(coverage_path).map_err(|message| (-32602, message))?;
    let report = crate::analyze_path(Path::new(path), Some(&coverage))
        .map_err(|error| (-32602, error.to_string()))?;
    let ranked = crate::test_targets::test_targets(&report, &coverage);
    let total = ranked.len();
    let limit = top.unwrap_or(MAX_ENTRIES).min(total);
    let truncated = total > limit;
    let mut rows = Vec::with_capacity(limit);
    for target in &ranked[..limit] {
        rows.push(serde_json::json!({
            "path": target.path,
            "function": target.function,
            "line": target.line,
            "crap": round1_opt(target.crap),
            "coverage": round1_opt(target.coverage),
            "uncovered": target.uncovered,
            "unknown": target.unknown,
        }));
    }
    let mut fields = envelope_fields();
    fields.insert("tool".to_owned(), serde_json::json!("test_targets"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("targets".to_owned(), serde_json::Value::Array(rows));
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    envelope(fields)
}

// ---------------------------------------------------------------------------
// Coverage
// ---------------------------------------------------------------------------

/// Load one coverage file, auto-detecting the format by extension:
/// `.xml` is JaCoCo XML, anything else (including `.info`) is LCOV.
fn load_coverage_file(path: &str) -> Result<crate::coverage::CoverageMap, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read coverage '{path}': {error}"))?;
    if Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
    {
        crate::coverage::CoverageMap::from_jacoco_xml(&text)
    } else {
        crate::coverage::CoverageMap::from_lcov(&text)
    }
    .map_err(|error| format!("cannot parse coverage '{path}': {error}"))
}

// ---------------------------------------------------------------------------
// Protocol helpers
// ---------------------------------------------------------------------------

/// Builds the initialize result, echoing the client's requested protocol
/// version per the spec's negotiation rules. Everything this server
/// implements (initialize, tools/list, tools/call, ping, notifications) is
/// version-stable, so echoing is honest; `2024-11-05` remains the fallback
/// for clients that send no version.
fn initialize_result(params: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": params
            .get("protocolVersion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("2024-11-05"),
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "leadline",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": SERVER_INSTRUCTIONS,
    })
}

fn tools_list_result() -> serde_json::Value {
    serde_json::json!({
        "tools": [
            {
                "name": "analyze",
                "description": "Measure function complexity across a path. Result paths are relative to the `path` argument (rejoin with it before passing one to analyze_function). Use for hotspot lists (sort_by/top) or specific metrics; use analyze_changed after edits.",
                "annotations": { "title": "Analyze complexity", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "coverage": { "type": ["string", "null"], "description": "Coverage file path (.info for LCOV, .xml for JaCoCo)." },
                        "top": { "type": "integer", "minimum": 1, "description": "Keep at most this many rows." },
                        "sort_by": { "type": "string", "enum": ["crap", "cognitive", "cyclomatic"], "description": "Sort rows by metric descending before capping." },
                        "min_crap": { "type": "number", "description": "Drop functions whose CRAP is below this floor; unknown CRAP is dropped." },
                    },
                },
            },
            {
                "name": "analyze_changed",
                "description": "Use after editing code to see which functions regressed. Compares against a git base with before/after deltas.",
                "annotations": { "title": "Analyze changed functions", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "base": { "type": "string", "default": "HEAD~1" },
                        "path": { "type": "string", "default": "." },
                        "target": { "type": "string", "default": "worktree", "description": "'worktree', 'index', or a revision to compare against base." },
                        "renames": { "type": "boolean", "default": false, "description": "Detect Git file renames and pair old-path content with new-path content." },
                        "explain": { "type": "boolean", "default": false, "description": "Include multiset-added contribution causes on regression rows." },
                        "top": { "type": "integer", "minimum": 1, "description": "Keep at most this many changes." },
                        "sort_by": { "type": "string", "enum": ["crap", "cognitive", "cyclomatic"], "description": "Sort changes by current-side metric descending." },
                        "min_crap": { "type": "number", "description": "Drop changes whose current-side CRAP is below this floor." },
                        "min_delta": { "type": "number", "description": "Drop changes whose max absolute delta is below this floor." },
                    },
                },
            },
            {
                "name": "analyze_function",
                "description": "Inspect one named function. Use explain:true to see the exact lines driving its complexity.",
                "annotations": { "title": "Analyze one function", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "function": { "type": "string" },
                        "explain": { "type": "boolean", "default": false, "description": "Include per-decision contribution lines (rule, line, nesting, increments)." },
                    },
                    "required": ["path", "function"],
                },
            },
            {
                "name": "check",
                "description": "Quality gate for thresholds or regressions. Use in CI or before committing; pass coverage so CRAP gates are meaningful.",
                "annotations": { "title": "Run quality gate", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "base": { "type": "string", "description": "Git base revision for changed-function gates." },
                        "baseline": { "type": "string", "description": "Baseline snapshot file for regression gates without Git. Exclusive with base; read-only, never written by this server." },
                        "coverage": { "type": "string", "description": "Coverage file path (.info for LCOV, .xml for JaCoCo) applied before CRAP gates." },
                        "thresholds": {
                            "type": "object",
                            "properties": {
                                "cognitive": { "type": "integer" },
                                "cyclomatic": { "type": "integer" },
                                "crap": { "type": "number" },
                                "max_nesting": { "type": "integer" },
                            },
                        },
                        "regressions": {
                            "description": "true for zero-tolerance deltas, or an object of allowed non-negative deltas.",
                            "type": ["boolean", "object"],
                            "properties": {
                                "cognitive": { "type": "integer", "minimum": 0 },
                                "cyclomatic": { "type": "integer", "minimum": 0 },
                                "crap": { "type": "number", "minimum": 0 },
                                "max_nesting": { "type": "integer", "minimum": 0 },
                            },
                        },
                    },
                },
            },
            {
                "name": "explain_metric",
                "description": "Look up how a metric is defined (default) before interpreting its numbers.",
                "annotations": { "title": "Explain metric", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "metric": { "type": "string", "enum": ["cyclomatic", "cognitive", "halstead", "maintainability", "crap"] },
                    },
                    "required": ["metric"],
                },
            },
            {
                "name": "repo_summary",
                "description": "Use as a first look at unfamiliar code: totals plus the top functions by CRAP, cognitive, and cyclomatic complexity. Result paths are relative to the `path` argument.",
                "annotations": { "title": "Repository summary", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 50, "default": 5, "description": "Functions kept per metric list." },
                    },
                },
            },
            {
                "name": "test_targets",
                "description": "Use to decide what to test: ranks functions holding uncovered decision lines by CRAP. Requires coverage.",
                "annotations": { "title": "Rank test targets", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "coverage": { "type": "string", "description": "Required coverage file path (.info for LCOV, .xml for JaCoCo). Line coverage required; branch records (BRDA, mb/cb) improve ratios when present." },
                        "top": { "type": "integer", "minimum": 1, "description": "Keep at most this many targets." },
                    },
                    "required": ["coverage"],
                },
            },
        ],
    })
}

fn success_response(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

fn parse_error_response() -> String {
    error_response(serde_json::Value::Null, -32700, "Parse error").to_string()
}

fn opt_str<'a>(params: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(|value| {
        if value.is_null() {
            None
        } else {
            value.as_str()
        }
    })
}

fn req_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, (i64, String)> {
    opt_str(params, key).ok_or((-32602, format!("missing required string param '{key}'")))
}
