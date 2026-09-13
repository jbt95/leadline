//! SARIF 2.1.0 output for GitHub code scanning and other SARIF consumers.

use crate::core::{AnalysisReport, FunctionMetrics, Thresholds};
use serde_json::{Value, json};

const SARIF_SCHEMA: &str =
    "https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json";
const INFORMATION_URI: &str = "https://github.com/jbt95/leadline";

/// Convert an [`AnalysisReport`] to SARIF 2.1.0.
///
/// Only functions violating at least one threshold appear as results (gate
/// semantics): a function over two limits yields two results, one per
/// violated dimension, and clean functions yield none. Results are sorted
/// by file uri, then start line, then rule id for deterministic output.
pub fn analysis_to_sarif(report: &AnalysisReport, thresholds: &Thresholds) -> Value {
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
    results.sort_by(|left, right| {
        left.0
            .cmp(right.0)
            .then(left.1.cmp(&right.1))
            .then(left.2.cmp(right.2))
    });
    json!({
        "version": "2.1.0",
        "$schema": SARIF_SCHEMA,
        "runs": [{
            "tool": {
                "driver": {
                    "name": "leadline",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": INFORMATION_URI,
                    "rules": [
                        {
                            "id": "leadline/cognitive",
                            "shortDescription": { "text": "Cognitive complexity exceeds the function limit" },
                        },
                        {
                            "id": "leadline/cyclomatic",
                            "shortDescription": { "text": "Cyclomatic complexity exceeds the function limit" },
                        },
                        {
                            "id": "leadline/crap",
                            "shortDescription": { "text": "CRAP score exceeds the function limit" },
                        },
                        {
                            "id": "leadline/max-nesting",
                            "shortDescription": { "text": "Maximum nesting depth exceeds the function limit" },
                        },
                    ],
                },
            },
            "results": results.into_iter().map(|(_, _, _, result)| result).collect::<Vec<_>>(),
        }],
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{FileAnalysis, FunctionAnalysis, FunctionKind, Language, MetricSpecs};

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
}
