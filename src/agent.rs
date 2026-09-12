use crate::core::{AnalysisReport, FunctionAnalysis, FunctionMetrics};
use crate::diff::ChangedReport;
use serde_json::{Value, json};

/// Compact agent-oriented view of a full analysis report.
///
/// Keeps only stable token-cheap fields; drops halstead/maintainability detail.
pub fn analyze_agent_json(report: &AnalysisReport) -> Value {
    let mut files: Vec<_> = report.files.iter().collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let functions: usize = report.files.iter().map(|file| file.functions.len()).sum();
    let files: Vec<Value> = files
        .into_iter()
        .map(|file| {
            let mut functions: Vec<&FunctionAnalysis> = file.functions.iter().collect();
            functions.sort_by(|left, right| {
                left.start_line
                    .cmp(&right.start_line)
                    .then(left.name.cmp(&right.name))
            });
            json!({
                "path": file.path,
                "functions": functions
                    .iter()
                    .map(|function| {
                        json!({
                            "name": function.name,
                            "line": function.start_line,
                            "cognitive": function.metrics.cognitive,
                            "cyclomatic": function.metrics.cyclomatic,
                            "crap": function.metrics.crap,
                            "coverage": function.metrics.coverage,
                        })
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "schema_version": report.schema_version,
        "metric_profile": report.metric_profile,
        "summary": {"files": report.files.len(), "functions": functions},
        "files": files,
    })
}

/// Compact agent-oriented view of a changed (before/after) report.
pub fn changed_agent_json(report: &ChangedReport) -> Value {
    let mut regressions: Vec<Entry> = Vec::new();
    let mut improvements: Vec<Entry> = Vec::new();
    for change in &report.functions {
        let entry = Entry::new(
            &change.path,
            &change.name,
            change.before.as_ref(),
            change.after.as_ref(),
        );
        if entry.regressed {
            regressions.push(entry);
        } else if entry.improved {
            improvements.push(entry);
        }
    }
    regressions.sort_by(|left, right| {
        right
            .crap_delta
            .total_cmp(&left.crap_delta)
            .then(left.path.cmp(&right.path))
            .then(left.name.cmp(&right.name))
    });
    improvements.sort_by(|left, right| left.path.cmp(&right.path).then(left.name.cmp(&right.name)));
    json!({
        "schema_version": report.schema_version,
        "base": report.base,
        "summary": {
            "changed_functions": report.functions.len(),
            "regressions": regressions.len(),
            "improvements": improvements.len(),
        },
        "regressions": regressions.into_iter().map(|entry| entry.value).collect::<Vec<_>>(),
        "improvements": improvements.into_iter().map(|entry| entry.value).collect::<Vec<_>>(),
    })
}

struct Entry {
    value: Value,
    regressed: bool,
    improved: bool,
    crap_delta: f64,
    path: String,
    name: String,
}

impl Entry {
    fn new(
        path: &str,
        name: &str,
        before: Option<&FunctionAnalysis>,
        after: Option<&FunctionAnalysis>,
    ) -> Self {
        let (regressed, improved, crap_delta) = classify(before, after);
        let value = json!({
            "path": path,
            "line": after.or(before).map(|function| function.start_line),
            "function": name,
            "before": before.map(|f| metric_set(&f.metrics)),
            "after": after.map(|f| metric_set(&f.metrics)),
            "delta": delta(before, after),
        });
        Self {
            value,
            regressed,
            improved,
            crap_delta,
            path: path.to_owned(),
            name: name.to_owned(),
        }
    }
}

fn metric_set(metrics: &FunctionMetrics) -> Value {
    json!({
        "cognitive": metrics.cognitive,
        "cyclomatic": metrics.cyclomatic,
        "crap": metrics.crap,
    })
}

fn delta(before: Option<&FunctionAnalysis>, after: Option<&FunctionAnalysis>) -> Value {
    match (before, after) {
        (Some(before), Some(after)) => json!({
            "cognitive": after.metrics.cognitive as i64 - before.metrics.cognitive as i64,
            "cyclomatic": after.metrics.cyclomatic as i64 - before.metrics.cyclomatic as i64,
            "crap": match (before.metrics.crap, after.metrics.crap) {
                (Some(before), Some(after)) => json!(after - before),
                _ => Value::Null,
            },
        }),
        _ => json!({"cognitive": null, "cyclomatic": null, "crap": null}),
    }
}

/// (regressed, improved, crap_delta). Null crap counts as no-change on that axis.
fn classify(
    before: Option<&FunctionAnalysis>,
    after: Option<&FunctionAnalysis>,
) -> (bool, bool, f64) {
    let (Some(before), Some(after)) = (before, after) else {
        return (false, false, 0.0);
    };
    let up = after.metrics.cognitive > before.metrics.cognitive
        || after.metrics.cyclomatic > before.metrics.cyclomatic
        || crap_delta(before, after).is_some_and(|delta| delta > 0.0);
    let down = after.metrics.cognitive < before.metrics.cognitive
        || after.metrics.cyclomatic < before.metrics.cyclomatic
        || crap_delta(before, after).is_some_and(|delta| delta < 0.0);
    (up, down && !up, crap_delta(before, after).unwrap_or(0.0))
}

fn crap_delta(before: &FunctionAnalysis, after: &FunctionAnalysis) -> Option<f64> {
    match (before.metrics.crap, after.metrics.crap) {
        (Some(before), Some(after)) => Some(after - before),
        _ => None,
    }
}
