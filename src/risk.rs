//! Policy- and concentration-aware change risk (`change-risk-v2`).
//!
//! The single risk model: static source metrics, Git history facts,
//! ownership concentration, dependency impact, and architecture policy
//! join into a per-file score with explicit components. `null` means
//! unknown, never zero: missing history or coverage renormalizes the
//! score over the known components instead of zeroing them.

use crate::config::Severity;
use crate::core::AnalysisReport;
use crate::graph::{DependencyFile, DependencyReport};
use crate::history::{FileHistory, HistoryReport, HistoryWindow};
use crate::impact::impact_counts;
use crate::ownership::OwnershipReport;
use crate::policy::{PolicyReport, PolicyStatus};
use serde::Serialize;
use std::collections::BTreeMap;

pub const RISK_SCHEMA_VERSION: u32 = 1;
pub const RISK_MODEL: &str = "change-risk-v2";

pub const COMPONENT_WEIGHTS: [(&str, f64); 6] = [
    ("complexity", 20.0),
    ("crap", 15.0),
    ("churn", 20.0),
    ("impact", 20.0),
    ("ownership", 10.0),
    ("policy", 15.0),
];

const POLICY_INFO: f64 = 30.0;
const POLICY_WARNING: f64 = 60.0;
const POLICY_ERROR: f64 = 100.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RiskComponents {
    pub complexity: Option<f64>,
    pub crap: Option<f64>,
    pub churn: Option<f64>,
    pub impact: Option<f64>,
    pub ownership: Option<f64>,
    pub policy: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RiskRaw {
    pub max_cognitive: u32,
    pub max_cyclomatic: u32,
    pub max_crap: Option<f64>,
    pub changes: Option<u64>,
    pub contributors: Option<u64>,
    pub concentration_percent: Option<f64>,
    pub blast_radius: usize,
    pub blast_radius_percent: f64,
    pub fan_in: usize,
    pub fan_out: usize,
    pub policy_severity: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RiskRow {
    pub path: String,
    pub score: f64,
    pub components: RiskComponents,
    pub raw: RiskRaw,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RiskReport {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub model: &'static str,
    pub window: &'static str,
    pub git_available: bool,
    pub head_commit: Option<String>,
    pub files_analyzed: usize,
    pub scope_files: usize,
    pub risks: Vec<RiskRow>,
}

/// Builds a complete v2 ranking (no truncation; consumers project budgets).
pub fn build(
    analysis: &AnalysisReport,
    history: &HistoryReport,
    graph: &DependencyReport,
    ownership: &OwnershipReport,
    policy: &PolicyReport,
    window: HistoryWindow,
) -> RiskReport {
    let history_rows: BTreeMap<&str, &FileHistory> = history
        .files
        .iter()
        .map(|row| (row.path.as_str(), row))
        .collect();
    let graph_rows: BTreeMap<&str, &DependencyFile> = graph
        .files
        .iter()
        .map(|row| (row.path.as_str(), row))
        .collect();
    let ownership_rows: BTreeMap<&str, Option<f64>> = ownership
        .files
        .iter()
        .map(|row| (row.path.as_str(), row.concentration_percent))
        .collect();
    let counts = impact_counts(graph);
    let policies = current_policy_severities(policy);

    let mut risks: Vec<RiskRow> = analysis
        .files
        .iter()
        .map(|file| {
            let history_row = history_rows.get(file.path.as_str()).copied();
            let changes = history_row.map(|row| window.changes(row));
            let contributors = history_row.map(|row| row.contributors);
            let (fan_in, fan_out) = graph_rows
                .get(file.path.as_str())
                .map(|row| (row.fan_in, row.fan_out))
                .unwrap_or((0, 0));
            let (blast_radius, blast_radius_percent) = counts
                .get(file.path.as_str())
                .map(|counts| (counts.blast_radius, counts.blast_radius_percent))
                .unwrap_or((0, 0.0));
            let max_cognitive = file
                .functions
                .iter()
                .map(|function| function.metrics.cognitive)
                .max()
                .unwrap_or(0);
            let max_cyclomatic = file
                .functions
                .iter()
                .map(|function| function.metrics.cyclomatic)
                .max()
                .unwrap_or(0);
            let max_crap = file
                .functions
                .iter()
                .filter_map(|function| function.metrics.crap)
                .reduce(f64::max);
            let severity = policies.get(file.path.as_str()).copied();
            let policy_value = severity.map(severity_value).unwrap_or(0.0);

            let components = RiskComponents {
                complexity: Some(
                    100.0
                        * (max_cognitive as f64 / 30.0)
                            .min(1.0)
                            .max((max_cyclomatic as f64 / 20.0).min(1.0)),
                ),
                crap: max_crap.map(|value| 100.0 * (value / 30.0).min(1.0)),
                churn: changes.map(|value| 100.0 * (value as f64 / 20.0).min(1.0)),
                impact: Some(blast_radius_percent.min(100.0)),
                ownership: ownership_rows.get(file.path.as_str()).copied().flatten(),
                policy: Some(policy_value),
            };
            let score = weighted_score(&components);
            RiskRow {
                path: file.path.clone(),
                score,
                components,
                raw: RiskRaw {
                    max_cognitive,
                    max_cyclomatic,
                    max_crap,
                    changes,
                    contributors,
                    concentration_percent: ownership_rows
                        .get(file.path.as_str())
                        .copied()
                        .flatten(),
                    blast_radius,
                    blast_radius_percent,
                    fan_in,
                    fan_out,
                    policy_severity: severity.map(severity_name),
                },
            }
        })
        .collect();
    risks.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });

    RiskReport {
        schema_version: RISK_SCHEMA_VERSION,
        metric_profile: analysis.metric_profile,
        analyzer_version: analysis.analyzer_version,
        model: RISK_MODEL,
        window: window.label(),
        git_available: history.available,
        head_commit: history.head_commit.clone(),
        files_analyzed: analysis.files.len(),
        scope_files: graph.files.len(),
        risks,
    }
}

/// Highest current (non-resolved) policy severity per source file.
fn current_policy_severities(policy: &PolicyReport) -> BTreeMap<&str, Severity> {
    let mut severities: BTreeMap<&str, Severity> = BTreeMap::new();
    for violation in &policy.violations {
        if matches!(violation.status, Some(PolicyStatus::Resolved)) {
            continue;
        }
        severities
            .entry(violation.source.as_str())
            .and_modify(|severity| *severity = (*severity).max(violation.severity))
            .or_insert(violation.severity);
    }
    severities
}

fn severity_value(severity: Severity) -> f64 {
    match severity {
        Severity::Info => POLICY_INFO,
        Severity::Warning => POLICY_WARNING,
        Severity::Error => POLICY_ERROR,
    }
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

fn weighted_score(components: &RiskComponents) -> f64 {
    let mut weighted = 0.0_f64;
    let mut weights = 0.0_f64;
    for &(name, weight) in COMPONENT_WEIGHTS.iter() {
        let value = match name {
            "complexity" => components.complexity,
            "crap" => components.crap,
            "churn" => components.churn,
            "impact" => components.impact,
            "ownership" => components.ownership,
            "policy" => components.policy,
            _ => None,
        };
        if let Some(value) = value {
            weighted += weight * value;
            weights += weight;
        }
    }
    if weights == 0.0 {
        0.0
    } else {
        weighted / weights
    }
}
