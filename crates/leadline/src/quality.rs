//! Quality ratings, remediation effort, and the named quality gate.
//!
//! Effort uses the `metric-overage` model: every unit a function exceeds a
//! configured threshold costs a fixed number of minutes, so a report's effort
//! is reproducible from the report itself and needs no rule catalog. Ratings
//! are banded A–E. `maintainability` comes from the effort-to-size ratio and
//! `security` from the severity of ingested scanner findings; a value with no
//! honest source stays `null`, and a gate condition on `null` fails closed
//! instead of passing silently.

use crate::config::{QualityGateConfig, Rating};
use model::core::{FunctionAnalysis, FunctionMetrics, Thresholds};
use serde::Serialize;
use std::collections::BTreeMap;

/// Model name carried by every effort number.
pub const EFFORT_MODEL: &str = "metric-overage";

/// Remediation minutes per cognitive-complexity point over the limit.
pub const COGNITIVE_MINUTES: u64 = 8;
/// Remediation minutes per cyclomatic-complexity point over the limit.
pub const CYCLOMATIC_MINUTES: u64 = 4;
/// Remediation minutes per nesting level over the limit.
pub const NESTING_MINUTES: u64 = 10;
/// Remediation minutes per CRAP point over the limit.
pub const CRAP_MINUTES: f64 = 12.0;

/// Development minutes one line of code is assumed to cost. The
/// maintainability rating divides effort by `loc * MINUTES_PER_LINE`, so the
/// ratio is "share of the codebase's assumed cost that is debt".
pub const MINUTES_PER_LINE: f64 = 30.0;

/// Gate scope when every analyzed function was gated.
pub const SCOPE_OVERALL: &str = "overall";
/// Gate scope when only the changed/paired function set was gated.
pub const SCOPE_CHANGED: &str = "changed";

/// Maintainability ratio at or below which the rating is the band's letter.
const MAINTAINABILITY_BANDS: [(Rating, f64); 4] = [
    (Rating::A, 0.05),
    (Rating::B, 0.10),
    (Rating::C, 0.20),
    (Rating::D, 0.50),
];

/// One function's remediation estimate in minutes.
pub fn function_effort_minutes(function: &FunctionAnalysis, thresholds: &Thresholds) -> u64 {
    overages(function, thresholds)
        .iter()
        .map(|(_, minutes)| minutes)
        .sum()
}

/// Per-metric remediation minutes, in the suppression order. Every metric
/// with no overage reports `0`.
pub fn overages(function: &FunctionAnalysis, thresholds: &Thresholds) -> [(&'static str, u64); 4] {
    let metrics: &FunctionMetrics = &function.metrics;
    let mut out = [
        ("cognitive", 0),
        ("cyclomatic", 0),
        ("max_nesting", 0),
        ("crap", 0),
    ];
    if let Some(limit) = thresholds.cognitive
        && metrics.cognitive > limit
        && !function.is_suppressed("cognitive")
    {
        out[0].1 = u64::from(metrics.cognitive - limit) * COGNITIVE_MINUTES;
    }
    if let Some(limit) = thresholds.cyclomatic
        && metrics.cyclomatic > limit
        && !function.is_suppressed("cyclomatic")
    {
        out[1].1 = u64::from(metrics.cyclomatic - limit) * CYCLOMATIC_MINUTES;
    }
    if let Some(limit) = thresholds.max_nesting
        && metrics.max_nesting > limit
        && !function.is_suppressed("max_nesting")
    {
        out[2].1 = u64::from(metrics.max_nesting - limit) * NESTING_MINUTES;
    }
    // A CRAP gate without coverage fails the function but carries no
    // estimable overage, so it contributes no minutes and stays visible as
    // the `crap_unavailable` violation reason instead of a guessed number.
    if let Some(limit) = thresholds.crap
        && let Some(crap) = metrics.crap
        && crap > limit
        && !function.is_suppressed("crap")
    {
        out[3].1 = ((crap - limit) * CRAP_MINUTES).ceil() as u64;
    }
    out
}

/// Remediation effort over one gated function set.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EffortSummary {
    pub model: &'static str,
    pub minutes: u64,
    /// Functions contributing at least one minute.
    pub functions: u64,
    /// Minutes per metric; metrics that contributed nothing are absent.
    pub by_metric: BTreeMap<&'static str, u64>,
}

impl EffortSummary {
    /// Sums per-function overages over one gated set.
    pub fn of<'a>(
        functions: impl IntoIterator<Item = &'a FunctionAnalysis>,
        thresholds: &Thresholds,
    ) -> Self {
        let mut minutes = 0u64;
        let mut count = 0u64;
        let mut by_metric: BTreeMap<&'static str, u64> = BTreeMap::new();
        for function in functions {
            let function_minutes = function_effort_minutes(function, thresholds);
            if function_minutes == 0 {
                continue;
            }
            minutes += function_minutes;
            count += 1;
            for (metric, metric_minutes) in overages(function, thresholds) {
                if metric_minutes > 0 {
                    *by_metric.entry(metric).or_insert(0) += metric_minutes;
                }
            }
        }
        Self {
            model: EFFORT_MODEL,
            minutes,
            functions: count,
            by_metric,
        }
    }
}

/// Maintainability rating from effort over source size.
pub fn maintainability_rating(minutes: u64, loc: u64) -> Rating {
    let assumed_cost = loc as f64 * MINUTES_PER_LINE;
    let ratio = if assumed_cost > 0.0 {
        minutes as f64 / assumed_cost
    } else {
        0.0
    };
    for (rating, limit) in MAINTAINABILITY_BANDS {
        if ratio <= limit {
            return rating;
        }
    }
    Rating::E
}

/// Finding counts by severity, over every ingested scanner family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SeverityCounts {
    pub unknown: u64,
    pub low: u64,
    pub medium: u64,
    pub high: u64,
    pub critical: u64,
}

impl SeverityCounts {
    pub fn record(&mut self, severity: &str) {
        match severity {
            "low" => self.low += 1,
            "medium" => self.medium += 1,
            "high" => self.high += 1,
            "critical" => self.critical += 1,
            _ => self.unknown += 1,
        }
    }

    pub fn total(&self) -> u64 {
        self.unknown + self.low + self.medium + self.high + self.critical
    }
}

impl<'a> FromIterator<&'a str> for SeverityCounts {
    fn from_iter<T: IntoIterator<Item = &'a str>>(severities: T) -> Self {
        let mut counts = Self::default();
        for severity in severities {
            counts.record(severity);
        }
        counts
    }
}

/// Security rating from ingested finding severities.
///
/// Findings whose severity could not be read never decide a rating: a run
/// with only unparseable severities stays `None` (unknown) instead of
/// reporting a clean `A`.
pub fn security_rating(counts: &SeverityCounts) -> Option<Rating> {
    if counts.critical > 0 {
        Some(Rating::E)
    } else if counts.high > 0 {
        Some(Rating::D)
    } else if counts.medium > 0 {
        Some(Rating::C)
    } else if counts.low > 0 {
        Some(Rating::B)
    } else if counts.total() == 0 {
        Some(Rating::A)
    } else {
        None
    }
}

/// The ratings one run can source honestly.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Ratings {
    pub maintainability: Option<Rating>,
    /// Always `null`: `check` ingests no bug-severity source, and an unknown
    /// signal is never guessed into a letter.
    pub reliability: Option<Rating>,
    pub security: Option<Rating>,
}

/// One evaluated gate condition.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GateCondition {
    pub metric: &'static str,
    pub operator: &'static str,
    /// Configured limit: a rating letter for rating conditions, minutes for
    /// `effort_minutes`.
    pub threshold: serde_json::Value,
    /// Measured value, or `null` when it could not be computed.
    pub actual: serde_json::Value,
    pub status: GateStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Passed,
    Failed,
}

/// One run's gate verdict over one function set.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QualityGateReport {
    pub name: String,
    pub status: GateStatus,
    pub scope: &'static str,
    pub conditions: Vec<GateCondition>,
    pub ratings: Ratings,
    pub effort: EffortSummary,
    /// Functions that failed a threshold only because of suppression lines.
    pub suppressed_violations: u64,
}

/// Everything one gate evaluation reads.
pub struct GateInputs<'a> {
    pub scope: &'static str,
    pub thresholds: &'a Thresholds,
    /// Functions in the gated set; changed runs pass after-side functions only.
    pub functions: Vec<&'a FunctionAnalysis>,
    /// Severity counts over ingested scanner families, `None` when no scanner
    /// input ran.
    pub security: Option<&'a SeverityCounts>,
    pub suppressed_violations: u64,
}

/// Evaluates every configured condition; unknown values fail closed.
pub fn evaluate(config: &QualityGateConfig, inputs: GateInputs<'_>) -> QualityGateReport {
    let loc: u64 = inputs
        .functions
        .iter()
        .map(|function| u64::from(function.metrics.loc))
        .sum();
    let effort = EffortSummary::of(inputs.functions.iter().copied(), inputs.thresholds);
    let maintainability = if inputs.thresholds.is_empty() {
        None
    } else {
        Some(maintainability_rating(effort.minutes, loc))
    };
    let security = inputs.security.and_then(security_rating);
    let conditions = config
        .conditions()
        .map(|metric| match metric {
            "maintainability_rating" => {
                rating_condition(metric, config.maintainability_rating, maintainability)
            }
            "security_rating" => {
                rating_condition("security_rating", config.security_rating, security)
            }
            "effort_minutes" => {
                let limit = config.effort_minutes.unwrap_or(0);
                let passed = effort.minutes <= u64::from(limit);
                GateCondition {
                    metric: "effort_minutes",
                    operator: "<=",
                    threshold: serde_json::Value::from(limit),
                    actual: serde_json::Value::from(effort.minutes),
                    status: gate_status(passed),
                }
            }
            _ => unreachable!("config validation rejects unknown gate conditions"),
        })
        .collect::<Vec<_>>();
    let status = gate_status(
        conditions
            .iter()
            .all(|condition| condition.status == GateStatus::Passed),
    );
    QualityGateReport {
        name: config.name.clone(),
        status,
        scope: inputs.scope,
        conditions,
        ratings: Ratings {
            maintainability,
            reliability: None,
            security,
        },
        effort,
        suppressed_violations: inputs.suppressed_violations,
    }
}

fn rating_condition(
    metric: &'static str,
    limit: Option<Rating>,
    actual: Option<Rating>,
) -> GateCondition {
    let passed = match (actual, limit) {
        (Some(actual), Some(limit)) => actual <= limit,
        // A condition on an uncomputable value fails closed, the same way a
        // CRAP gate without coverage does.
        _ => false,
    };
    GateCondition {
        metric,
        operator: "<=",
        threshold: limit.map_or(serde_json::Value::Null, |limit| {
            serde_json::Value::from(limit.as_str())
        }),
        actual: actual.map_or(serde_json::Value::Null, |actual| {
            serde_json::Value::from(actual.as_str())
        }),
        status: gate_status(passed),
    }
}

fn gate_status(passed: bool) -> GateStatus {
    if passed {
        GateStatus::Passed
    } else {
        GateStatus::Failed
    }
}

/// Violation reasons silenced by in-source suppression lines across one set,
/// counted per dimension so a partly suppressed function still reports the
/// dimensions it really cleared.
pub fn suppressed_violation_count<'a>(
    functions: impl IntoIterator<Item = &'a FunctionAnalysis>,
    thresholds: &Thresholds,
) -> u64 {
    functions
        .into_iter()
        .map(|function| {
            (thresholds.violation_reasons(&function.metrics).len()
                - thresholds.violation_reasons_for(function).len()) as u64
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::core::{FunctionKind, FunctionMetrics, SuppressionMetric};

    fn function(metrics: FunctionMetrics, suppressed: Vec<SuppressionMetric>) -> FunctionAnalysis {
        FunctionAnalysis {
            name: "f".to_owned(),
            id: "f".to_owned(),
            kind: FunctionKind::Function,
            start_line: 1,
            end_line: 10,
            start_byte: 0,
            end_byte: 100,
            metrics,
            contributions: Vec::new(),
            suppressed_metrics: suppressed,
            source_fingerprint: 0,
        }
    }

    fn metrics(
        cognitive: u32,
        cyclomatic: u32,
        max_nesting: u32,
        crap: Option<f64>,
    ) -> FunctionMetrics {
        FunctionMetrics {
            loc: 20,
            logical_loc: 10,
            function_length: 20,
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
            halstead_volume: 1.0,
            halstead_difficulty: 1.0,
            halstead_effort: 1.0,
            maintainability_index: 100.0,
            coverage: None,
            crap,
        }
    }

    #[test]
    fn effort_weights_each_metric_by_its_overage() {
        let thresholds = Thresholds {
            cognitive: Some(15),
            cyclomatic: Some(10),
            max_nesting: Some(4),
            crap: Some(30.0),
        };
        // cognitive 24 -> 9 * 8 = 72; cyclomatic 15 -> 5 * 4 = 20;
        // nesting 6 -> 2 * 10 = 20; crap 32.5 -> 2.5 * 12 = 30.
        let row = function(metrics(24, 15, 6, Some(32.5)), Vec::new());
        assert_eq!(function_effort_minutes(&row, &thresholds), 142);
        let summary = EffortSummary::of([&row], &thresholds);
        assert_eq!(summary.minutes, 142);
        assert_eq!(summary.functions, 1);
        assert_eq!(summary.by_metric["cognitive"], 72);
        assert_eq!(summary.by_metric["crap"], 30);
    }

    #[test]
    fn effort_counts_only_the_overage_never_the_whole_value() {
        let thresholds = Thresholds {
            cognitive: Some(20),
            ..Thresholds::default()
        };
        let row = function(metrics(21, 1, 0, None), Vec::new());
        assert_eq!(function_effort_minutes(&row, &thresholds), 8);
        assert!(EffortSummary::of([&row], &Thresholds::default()).minutes == 0);
    }

    #[test]
    fn unknown_crap_contributes_no_minutes() {
        let thresholds = Thresholds {
            crap: Some(30.0),
            ..Thresholds::default()
        };
        let row = function(metrics(1, 1, 0, None), Vec::new());
        assert_eq!(function_effort_minutes(&row, &thresholds), 0);
    }

    #[test]
    fn suppressed_metrics_contribute_no_minutes() {
        let thresholds = Thresholds {
            cognitive: Some(1),
            cyclomatic: Some(1),
            ..Thresholds::default()
        };
        let row = function(metrics(5, 5, 0, None), vec![SuppressionMetric::Cognitive]);
        assert_eq!(function_effort_minutes(&row, &thresholds), 16);
    }

    #[test]
    fn maintainability_bands_follow_the_documented_ratios() {
        // 300 minutes over 10 lines * 30 = 300 -> ratio 1.0.
        assert_eq!(maintainability_rating(15, 10), Rating::A);
        assert_eq!(maintainability_rating(30, 10), Rating::B);
        assert_eq!(maintainability_rating(60, 10), Rating::C);
        assert_eq!(maintainability_rating(150, 10), Rating::D);
        assert_eq!(maintainability_rating(151, 10), Rating::E);
        // No source size means no assumed cost to compare against.
        assert_eq!(maintainability_rating(0, 0), Rating::A);
    }

    #[test]
    fn security_rating_uses_the_worst_ingested_severity() {
        assert_eq!(security_rating(&SeverityCounts::default()), Some(Rating::A));
        assert_eq!(
            security_rating(&SeverityCounts::from_iter(["low"])),
            Some(Rating::B)
        );
        assert_eq!(
            security_rating(&SeverityCounts::from_iter(["low", "medium"])),
            Some(Rating::C)
        );
        assert_eq!(
            security_rating(&SeverityCounts::from_iter(["medium", "high"])),
            Some(Rating::D)
        );
        assert_eq!(
            security_rating(&SeverityCounts::from_iter(["high", "critical"])),
            Some(Rating::E)
        );
        // Unreadable severities never become a passing rating.
        assert_eq!(
            security_rating(&SeverityCounts::from_iter(["unparseable"])),
            None
        );
    }

    #[test]
    fn gate_fails_closed_when_a_rating_is_unknown() {
        let config = QualityGateConfig {
            name: "team".to_owned(),
            maintainability_rating: Some(Rating::B),
            security_rating: Some(Rating::A),
            effort_minutes: None,
        };
        let thresholds = Thresholds::default();
        let report = evaluate(
            &config,
            GateInputs {
                scope: SCOPE_OVERALL,
                thresholds: &thresholds,
                functions: Vec::new(),
                security: None,
                suppressed_violations: 0,
            },
        );
        assert_eq!(report.status, GateStatus::Failed);
        assert_eq!(report.ratings.maintainability, None);
        assert_eq!(report.ratings.security, None);
        assert!(report.ratings.reliability.is_none());
        assert!(
            report
                .conditions
                .iter()
                .all(|condition| condition.actual.is_null()
                    && condition.status == GateStatus::Failed)
        );
    }

    #[test]
    fn gate_passes_on_a_clean_run_and_reports_effort() {
        let config = QualityGateConfig {
            name: "team".to_owned(),
            maintainability_rating: None,
            security_rating: Some(Rating::A),
            effort_minutes: Some(100),
        };
        let thresholds = Thresholds {
            cognitive: Some(15),
            ..Thresholds::default()
        };
        let row = function(metrics(16, 1, 0, None), Vec::new());
        let security = SeverityCounts::default();
        let report = evaluate(
            &config,
            GateInputs {
                scope: SCOPE_CHANGED,
                thresholds: &thresholds,
                functions: vec![&row],
                security: Some(&security),
                suppressed_violations: 2,
            },
        );
        assert_eq!(report.status, GateStatus::Passed);
        assert_eq!(report.scope, SCOPE_CHANGED);
        assert_eq!(report.effort.minutes, 8);
        assert_eq!(report.ratings.security, Some(Rating::A));
        assert_eq!(report.suppressed_violations, 2);
    }

    #[test]
    fn effort_condition_fails_past_its_limit() {
        let config = QualityGateConfig {
            name: "team".to_owned(),
            maintainability_rating: None,
            security_rating: None,
            effort_minutes: Some(7),
        };
        let thresholds = Thresholds {
            cognitive: Some(15),
            ..Thresholds::default()
        };
        let row = function(metrics(16, 1, 0, None), Vec::new());
        let report = evaluate(
            &config,
            GateInputs {
                scope: SCOPE_OVERALL,
                thresholds: &thresholds,
                functions: vec![&row],
                security: None,
                suppressed_violations: 0,
            },
        );
        assert_eq!(report.status, GateStatus::Failed);
        assert_eq!(report.conditions[0].metric, "effort_minutes");
        assert_eq!(report.conditions[0].actual, serde_json::Value::from(8));
    }
}
