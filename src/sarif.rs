//! SARIF 2.1.0 output for GitHub code scanning and other SARIF consumers.

use crate::core::{AnalysisReport, FunctionMetrics, Thresholds};
use serde_json::{Value, json};

const SARIF_SCHEMA: &str =
    "https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json";
const INFORMATION_URI: &str = "https://github.com/jbt95/leadline";

/// Shared SARIF 2.1.0 envelope: one run of the leadline driver.
fn envelope(rules: Vec<Value>, results: Vec<Value>) -> Value {
    json!({
        "version": "2.1.0",
        "$schema": SARIF_SCHEMA,
        "runs": [{
            "tool": {
                "driver": {
                    "name": "leadline",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": INFORMATION_URI,
                    "rules": rules,
                },
            },
            "results": results,
        }],
    })
}

/// One explicit per-function gate violation for SARIF projection.
///
/// Regression gates build these from real before/after pairs, so results
/// carry the actual deltas and configured limits instead of synthetic ones.
pub struct FunctionViolation<'a> {
    pub rule_id: &'static str,
    pub message: String,
    pub path: &'a str,
    pub start_line: u32,
    pub end_line: u32,
}

/// Convert an [`AnalysisReport`] to SARIF 2.1.0 with absolute thresholds.
///
/// Only functions violating at least one threshold appear as results (gate
/// semantics): a function over two limits yields two results, one per
/// violated dimension, and clean functions yield none.
pub fn analysis_to_sarif(report: &AnalysisReport, thresholds: &Thresholds) -> Value {
    gate_to_sarif(report, thresholds, &[])
}

/// Metric gate SARIF: absolute threshold violations plus explicit
/// regression violations, in one run with one rule catalog.
///
/// Results are sorted by file uri, then start line, then rule id for
/// deterministic output.
pub fn gate_to_sarif(
    report: &AnalysisReport,
    thresholds: &Thresholds,
    regressions: &[FunctionViolation<'_>],
) -> Value {
    let mut results: Vec<(&str, u32, &str, Value)> = Vec::new();
    for file in &report.files {
        for function in &file.functions {
            for (rule_id, message) in violations(&function.metrics, thresholds) {
                results.push((
                    file.path.as_str(),
                    function.start_line,
                    rule_id,
                    json!({
                        "ruleId": rule_id,
                        "level": "error",
                        "message": { "text": message },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": { "uri": file.path },
                                "region": {
                                    "startLine": function.start_line,
                                    "endLine": function.end_line,
                                },
                            },
                        }],
                    }),
                ));
            }
        }
    }
    for violation in regressions {
        results.push((
            violation.path,
            violation.start_line,
            violation.rule_id,
            json!({
                "ruleId": violation.rule_id,
                "level": "error",
                "message": { "text": violation.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": violation.path },
                        "region": {
                            "startLine": violation.start_line,
                            "endLine": violation.end_line,
                        },
                    },
                }],
            }),
        ));
    }
    results.sort_by(|left, right| {
        left.0
            .cmp(right.0)
            .then(left.1.cmp(&right.1))
            .then(left.2.cmp(right.2))
    });
    let rules = vec![
        json!({
            "id": "leadline/cognitive",
            "shortDescription": { "text": "Cognitive complexity exceeds the function limit" },
        }),
        json!({
            "id": "leadline/cyclomatic",
            "shortDescription": { "text": "Cyclomatic complexity exceeds the function limit" },
        }),
        json!({
            "id": "leadline/crap",
            "shortDescription": { "text": "CRAP score exceeds the function limit" },
        }),
        json!({
            "id": "leadline/max-nesting",
            "shortDescription": { "text": "Maximum nesting depth exceeds the function limit" },
        }),
    ];
    let results: Vec<Value> = results
        .into_iter()
        .map(|(_, _, _, result)| result)
        .collect();
    envelope(rules, results)
}

/// Convert a PostgreSQL plan comparison report to SARIF 2.1.0.
///
/// One result per gate violation. Rule IDs are `postgresql-plan/{kind}` and
/// artifact URIs are `plans/{query_id}.json`. Messages carry only fixed text
/// with plan metrics; they never include SQL, plan predicates, or file paths
/// beyond the query artifact.
pub fn pg_plan_to_sarif(report: &crate::pg_plan::PgPlanReport) -> Value {
    let rules: Vec<Value> = [
        (
            "index_to_sequential_scan",
            "Plan regressed from an index scan to a sequential scan",
        ),
        (
            "cost_increase",
            "Plan total cost increased past the configured limit",
        ),
        (
            "row_growth",
            "Plan row estimate grew past the configured ratio",
        ),
        (
            "estimate_error",
            "Planner estimate error exceeds the configured ratio",
        ),
        (
            "added_sort",
            "Plan added sort nodes beside a numeric regression",
        ),
        (
            "join_strategy_change",
            "Plan join strategy changed beside a numeric regression",
        ),
    ]
    .iter()
    .map(|(kind, text)| {
        json!({
            "id": format!("postgresql-plan/{kind}"),
            "shortDescription": { "text": text },
        })
    })
    .collect();
    let mut results = Vec::new();
    for violation in &report.violations {
        let kind = violation.kind.as_str();
        results.push(json!({
            "ruleId": format!("postgresql-plan/{kind}"),
            "level": "error",
            "message": { "text": pg_plan_message(violation) },
            "locations": [{
                "physicalLocation": {
                    "artifactLocation": { "uri": format!("plans/{}.json", violation.query_id) },
                },
            }],
        }));
    }
    envelope(rules, results)
}

/// Fixed SARIF message from metrics only; no SQL or plan predicates.
fn pg_plan_message(violation: &crate::pg_plan::QueryPlanChange) -> String {
    use crate::pg_plan::PlanRegressionKind;
    let id = violation.query_id.as_str();
    match violation.kind {
        PlanRegressionKind::IndexToSequentialScan => format!(
            "query '{id}' regressed from index scan to sequential scan on table '{}'",
            violation.relation.as_deref().unwrap_or("?")
        ),
        PlanRegressionKind::CostIncrease => match (violation.baseline, violation.current) {
            (Some(base), Some(now)) => format!(
                "query '{id}' total cost {base} -> {now} ({})",
                violation.detail.as_deref().unwrap_or("?")
            ),
            _ => format!("query '{id}' total cost increased"),
        },
        PlanRegressionKind::RowGrowth => match (violation.baseline, violation.current) {
            (Some(base), Some(now)) => format!(
                "query '{id}' plan rows {base} -> {now} ({})",
                violation.detail.as_deref().unwrap_or("?")
            ),
            _ => format!("query '{id}' plan rows grew"),
        },
        PlanRegressionKind::EstimateError => match violation.current {
            Some(ratio) => format!("query '{id}' planner estimate error {ratio:.1}x"),
            None => format!("query '{id}' planner estimate error is unbounded"),
        },
        PlanRegressionKind::AddedSort => format!(
            "query '{id}' added sort nodes ({})",
            violation.detail.as_deref().unwrap_or("?")
        ),
        PlanRegressionKind::JoinStrategyChange => format!(
            "query '{id}' join strategy changed ({})",
            violation.detail.as_deref().unwrap_or("?")
        ),
    }
}

/// Convert normalized security findings to SARIF 2.1.0.
///
/// One result per finding with rule ID `{tool}/{rule_id}` and the fixed
/// message `"{tool} finding {rule_id}"`. Scanner messages, snippets, and
/// source text never cross over. Pathless findings carry no locations.
/// Callers pass the full report or only gate violations.
pub fn security_findings_to_sarif(findings: &[crate::security::SecurityFinding]) -> Value {
    let mut seen = std::collections::BTreeSet::new();
    let mut rules = Vec::new();
    for finding in findings {
        let id = format!("{}/{}", finding.tool, finding.rule_id);
        if seen.insert(id.clone()) {
            rules.push(json!({
                "id": id,
                "shortDescription": { "text": format!("{} finding {}", finding.tool, finding.rule_id) },
            }));
        }
    }
    let mut results = Vec::new();
    for finding in findings {
        let mut result = json!({
            "ruleId": format!("{}/{}", finding.tool, finding.rule_id),
            "level": severity_level(finding.severity),
            "message": { "text": format!("{} finding {}", finding.tool, finding.rule_id) },
        });
        if let Some(path) = finding.path.as_deref() {
            let mut location = json!({
                "physicalLocation": {
                    "artifactLocation": { "uri": path },
                },
            });
            if finding.start_line.is_some() || finding.end_line.is_some() {
                location["physicalLocation"]["region"] = json!({});
                if let Some(line) = finding.start_line {
                    location["physicalLocation"]["region"]["startLine"] = json!(line);
                }
                if let Some(line) = finding.end_line {
                    location["physicalLocation"]["region"]["endLine"] = json!(line);
                }
            }
            result["locations"] = json!([location]);
        }
        results.push(result);
    }
    envelope(rules, results)
}

/// SARIF level from normalized severity, shared by every scanner projection.
fn severity_level(severity: crate::security::SecuritySeverity) -> &'static str {
    match severity {
        crate::security::SecuritySeverity::Critical | crate::security::SecuritySeverity::High => {
            "error"
        }
        crate::security::SecuritySeverity::Medium => "warning",
        crate::security::SecuritySeverity::Low | crate::security::SecuritySeverity::Unknown => {
            "note"
        }
    }
}

/// Convert normalized vulnerability findings to SARIF 2.1.0.
///
/// One result per finding with rule ID
/// `vulnerability/{ecosystem}/{advisory_id}` and the fixed message
/// `"vulnerable dependency {package}"`. Installed versions, titles,
/// descriptions, and URLs never cross over. Findings without a manifest
/// path carry no locations. Callers pass the full report or only gate
/// violations.
pub fn vulnerability_findings_to_sarif(
    findings: &[crate::vulnerabilities::VulnerabilityFinding],
) -> Value {
    let mut seen = std::collections::BTreeSet::new();
    let mut rules = Vec::new();
    for finding in findings {
        let id = format!(
            "vulnerability/{}/{}",
            finding.ecosystem, finding.advisory_id
        );
        if seen.insert(id.clone()) {
            rules.push(json!({
                "id": id,
                "shortDescription": { "text": "Vulnerable dependency advisory" },
            }));
        }
    }
    let mut results = Vec::new();
    for finding in findings {
        let mut result = json!({
            "ruleId": format!("vulnerability/{}/{}", finding.ecosystem, finding.advisory_id),
            "level": severity_level(finding.severity),
            "message": { "text": format!("vulnerable dependency {}", finding.package) },
        });
        if let Some(manifest) = finding.manifest_path.as_deref() {
            result["locations"] = json!([{
                "physicalLocation": {
                    "artifactLocation": { "uri": manifest },
                },
            }]);
        }
        results.push(result);
    }
    envelope(rules, results)
}

/// Convert static PostgreSQL risk findings to SARIF 2.1.0.
///
/// Rule IDs equal the SQL rule IDs; messages carry the fixed remediation
/// strings only, never SQL text, literals, or identifiers. Callers pass the
/// full report or only gate violations.
pub fn sql_findings_to_sarif(findings: &[crate::sql::SqlFinding]) -> Value {
    let mut seen = std::collections::BTreeSet::new();
    let mut rules = Vec::new();
    for finding in findings {
        if seen.insert(finding.rule_id.clone()) {
            rules.push(json!({
                "id": finding.rule_id,
                "shortDescription": { "text": finding.remediation },
            }));
        }
    }
    let results: Vec<Value> = findings
        .iter()
        .map(|finding| {
            json!({
                "ruleId": finding.rule_id,
                "level": severity_level(finding.severity),
                "message": { "text": finding.remediation },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": finding.path },
                        "region": {
                            "startLine": finding.start_line,
                            "endLine": finding.end_line,
                        },
                    },
                }],
            })
        })
        .collect();
    envelope(rules, results)
}

/// One `(rule id, message)` pair per violated threshold dimension.
fn violations(metrics: &FunctionMetrics, thresholds: &Thresholds) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if thresholds
        .cognitive
        .is_some_and(|limit| metrics.cognitive > limit)
    {
        out.push((
            "leadline/cognitive",
            format!(
                "cognitive complexity {} exceeds limit {}",
                metrics.cognitive,
                thresholds.cognitive.unwrap()
            ),
        ));
    }
    if thresholds
        .cyclomatic
        .is_some_and(|limit| metrics.cyclomatic > limit)
    {
        out.push((
            "leadline/cyclomatic",
            format!(
                "cyclomatic complexity {} exceeds limit {}",
                metrics.cyclomatic,
                thresholds.cyclomatic.unwrap()
            ),
        ));
    }
    if thresholds
        .crap
        .is_some_and(|limit| metrics.crap.is_none_or(|value| value > limit))
    {
        let actual = metrics
            .crap
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        out.push((
            "leadline/crap",
            format!(
                "CRAP score {actual} exceeds limit {}",
                thresholds.crap.unwrap()
            ),
        ));
    }
    if thresholds
        .max_nesting
        .is_some_and(|limit| metrics.max_nesting > limit)
    {
        out.push((
            "leadline/max-nesting",
            format!(
                "max nesting {} exceeds limit {}",
                metrics.max_nesting,
                thresholds.max_nesting.unwrap()
            ),
        ));
    }
    out
}

/// Delta violations for one paired function, with the real limits.
///
/// Messages carry the before/after values and the configured allowed delta,
/// never a synthetic limit. Rule IDs stay dimension-level so consumers group
/// absolute and regression results per metric.
pub fn regression_violations<'a>(
    before: &crate::core::FunctionAnalysis,
    after: &crate::core::FunctionAnalysis,
    limits: &crate::config::RegressionLimits,
    path: &'a str,
) -> Vec<FunctionViolation<'a>> {
    let dimensions = crate::diff::regression_dimensions(before, after, limits);
    let mut out = Vec::new();
    let location = |rule_id: &'static str, message: String| FunctionViolation {
        rule_id,
        message,
        path,
        start_line: after.start_line,
        end_line: after.end_line,
    };
    if dimensions[0] {
        out.push(location(
            "leadline/cognitive",
            format!(
                "cognitive complexity {} -> {} exceeds allowed delta {}",
                before.metrics.cognitive, after.metrics.cognitive, limits.cognitive
            ),
        ));
    }
    if dimensions[1] {
        out.push(location(
            "leadline/cyclomatic",
            format!(
                "cyclomatic complexity {} -> {} exceeds allowed delta {}",
                before.metrics.cyclomatic, after.metrics.cyclomatic, limits.cyclomatic
            ),
        ));
    }
    if dimensions[2]
        && let (Some(before_score), Some(after_score)) = (before.metrics.crap, after.metrics.crap)
    {
        out.push(location(
            "leadline/crap",
            format!(
                "CRAP score {before_score} -> {after_score} exceeds allowed delta {}",
                limits.crap
            ),
        ));
    }
    if dimensions[3] {
        out.push(location(
            "leadline/max-nesting",
            format!(
                "max nesting {} -> {} exceeds allowed delta {}",
                before.metrics.max_nesting, after.metrics.max_nesting, limits.max_nesting
            ),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{FileAnalysis, FunctionAnalysis, FunctionKind, Language, MetricSpecs};
    use crate::pg_plan::{PlanRegressionKind, QueryPlanChange};

    fn metrics(
        cognitive: u32,
        cyclomatic: u32,
        crap: Option<f64>,
        max_nesting: u32,
    ) -> FunctionMetrics {
        FunctionMetrics {
            loc: 10,
            logical_loc: 5,
            function_length: 10,
            parameters: 1,
            max_nesting,
            cyclomatic,
            cognitive,
            halstead_n1: 1,
            halstead_n2: 1,
            halstead_total_operators: 1,
            halstead_total_operands: 1,
            halstead_vocabulary: 2,
            halstead_length: 2,
            halstead_volume: 2.0,
            halstead_difficulty: 1.0,
            halstead_effort: 2.0,
            maintainability_index: 80.0,
            coverage: Some(0.5),
            crap,
        }
    }

    fn function(name: &str, start_line: u32, metrics: FunctionMetrics) -> FunctionAnalysis {
        FunctionAnalysis {
            name: name.to_owned(),
            id: format!("{name}:function:{start_line}:{}", start_line + 5),
            kind: FunctionKind::Function,
            start_line,
            end_line: start_line + 5,
            start_byte: u64::from(start_line),
            end_byte: u64::from(start_line + 5),
            metrics,
            source_fingerprint: 0,
            contributions: Vec::new(),
        }
    }

    fn report(files: Vec<(&str, Vec<FunctionAnalysis>)>) -> AnalysisReport {
        AnalysisReport {
            schema_version: 1,
            metric_profile: "default",
            analyzer_version: "0.1.0",
            metric_specs: MetricSpecs::default(),
            files: files
                .into_iter()
                .map(|(path, functions)| FileAnalysis {
                    path: path.to_owned(),
                    language: Language::TypeScript,
                    functions,
                    parse_errors: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn top_level_keys_present() {
        let sarif = analysis_to_sarif(&report(vec![]), &Thresholds::default());
        assert_eq!(sarif["version"], json!("2.1.0"));
        assert!(sarif["$schema"].is_string());
        let driver = &sarif["runs"][0]["tool"]["driver"];
        assert_eq!(driver["name"], json!("leadline"));
        assert!(driver["version"].is_string());
        assert!(driver["informationUri"].is_string());
        assert_eq!(driver["rules"].as_array().unwrap().len(), 4);
        assert!(sarif["runs"][0]["results"].is_array());
    }

    #[test]
    fn one_violation_yields_one_result() {
        let report = report(vec![(
            "src/a.ts",
            vec![function("over", 10, metrics(20, 2, Some(1.0), 1))],
        )]);
        let thresholds = Thresholds {
            cognitive: Some(15),
            ..Thresholds::default()
        };
        let sarif = analysis_to_sarif(&report, &thresholds);
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["ruleId"], json!("leadline/cognitive"));
        assert_eq!(results[0]["level"], json!("error"));
        assert_eq!(
            results[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            json!("src/a.ts")
        );
        let region = &results[0]["locations"][0]["physicalLocation"]["region"];
        assert_eq!(region["startLine"], json!(10));
        assert_eq!(region["endLine"], json!(15));
    }

    #[test]
    fn clean_report_yields_empty_results() {
        let report = report(vec![(
            "src/a.ts",
            vec![function("fine", 1, metrics(5, 2, Some(1.0), 1))],
        )]);
        let thresholds = Thresholds {
            cognitive: Some(15),
            cyclomatic: Some(10),
            crap: Some(30.0),
            max_nesting: Some(4),
        };
        let sarif = analysis_to_sarif(&report, &thresholds);
        assert_eq!(sarif["runs"][0]["results"], json!([]));
    }

    #[test]
    fn results_sorted_deterministically() {
        let report = report(vec![
            (
                "src/b.ts",
                vec![function("b", 3, metrics(20, 20, Some(99.0), 9))],
            ),
            (
                "src/a.ts",
                vec![
                    function("a2", 30, metrics(20, 1, Some(1.0), 1)),
                    function("a1", 5, metrics(20, 1, Some(1.0), 1)),
                ],
            ),
        ]);
        let thresholds = Thresholds {
            cognitive: Some(15),
            cyclomatic: Some(10),
            crap: Some(30.0),
            max_nesting: Some(4),
        };
        let sarif = analysis_to_sarif(&report, &thresholds);
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        let keys: Vec<(&str, u64, &str)> = results
            .iter()
            .map(|result| {
                (
                    result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
                        .as_str()
                        .unwrap(),
                    result["locations"][0]["physicalLocation"]["region"]["startLine"]
                        .as_u64()
                        .unwrap(),
                    result["ruleId"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                ("src/a.ts", 5, "leadline/cognitive"),
                ("src/a.ts", 30, "leadline/cognitive"),
                ("src/b.ts", 3, "leadline/cognitive"),
                ("src/b.ts", 3, "leadline/crap"),
                ("src/b.ts", 3, "leadline/cyclomatic"),
                ("src/b.ts", 3, "leadline/max-nesting"),
            ]
        );
    }

    fn plan_change(kind: PlanRegressionKind) -> QueryPlanChange {
        QueryPlanChange {
            query_id: "q1".to_owned(),
            kind,
            relation: Some("users".to_owned()),
            detail: Some("detail".to_owned()),
            baseline: Some(100.0),
            current: Some(150.0),
            violates_gate: true,
        }
    }

    /// One fixed message per regression kind. The text is the whole contract
    /// for a SARIF consumer, so every kind needs a case: a silently wrong arm
    /// reports a regression the plan never had.
    #[test]
    fn pg_plan_messages_cover_every_kind() {
        use PlanRegressionKind::{
            AddedSort, CostIncrease, EstimateError, IndexToSequentialScan, JoinStrategyChange,
            RowGrowth,
        };
        for (kind, expected) in [
            (
                IndexToSequentialScan,
                "query 'q1' regressed from index scan to sequential scan on table 'users'",
            ),
            (CostIncrease, "query 'q1' total cost 100 -> 150 (detail)"),
            (RowGrowth, "query 'q1' plan rows 100 -> 150 (detail)"),
            (EstimateError, "query 'q1' planner estimate error 150.0x"),
            (AddedSort, "query 'q1' added sort nodes (detail)"),
            (
                JoinStrategyChange,
                "query 'q1' join strategy changed (detail)",
            ),
        ] {
            assert_eq!(pg_plan_message(&plan_change(kind)), expected, "{kind:?}");
        }
    }

    /// A missing relation, detail, or cost must render the `?` placeholder or
    /// the short form rather than an empty gap.
    #[test]
    fn pg_plan_messages_substitute_placeholders() {
        use PlanRegressionKind::{
            AddedSort, CostIncrease, EstimateError, IndexToSequentialScan, JoinStrategyChange,
            RowGrowth,
        };
        let bare = |kind| {
            let mut change = plan_change(kind);
            change.relation = None;
            change.detail = None;
            change.baseline = None;
            change.current = None;
            change
        };

        for (kind, expected) in [
            (
                IndexToSequentialScan,
                "query 'q1' regressed from index scan to sequential scan on table '?'",
            ),
            (CostIncrease, "query 'q1' total cost increased"),
            (RowGrowth, "query 'q1' plan rows grew"),
            (
                EstimateError,
                "query 'q1' planner estimate error is unbounded",
            ),
            (AddedSort, "query 'q1' added sort nodes (?)"),
            (JoinStrategyChange, "query 'q1' join strategy changed (?)"),
        ] {
            assert_eq!(pg_plan_message(&bare(kind)), expected, "{kind:?}");
        }
    }
}
