//! Explainable change-risk model (`change-risk-v1`).
//!
//! Joins static source metrics, Git history facts, and dependency impact into
//! a per-file score with explicit components. `null` means unknown, never
//! zero: missing history or coverage renormalizes the score over the known
//! components instead of zeroing them.

use crate::core::AnalysisReport;
use crate::graph::{DependencyFile, DependencyReport};
use crate::history::{FileHistory, HistoryReport, HistoryWindow};
use crate::impact::impact_counts;
use serde::Serialize;
use std::collections::BTreeMap;

pub const RISK_SCHEMA_VERSION: u32 = 1;
pub const RISK_MODEL: &str = "change-risk-v1";

pub const COMPONENT_WEIGHTS: [(&str, f64); 6] = [
    ("complexity", 25.0),
    ("crap", 20.0),
    ("churn", 20.0),
    ("impact", 20.0),
    ("ownership", 15.0),
    ("policy", 0.0),
];

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
    pub blast_radius: usize,
    pub blast_radius_percent: f64,
    pub fan_in: usize,
    pub fan_out: usize,
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
    pub analyzer_version: &'static str,
    pub metric_profile: &'static str,
    pub model: &'static str,
    pub window: &'static str,
    pub git_available: bool,
    pub head_commit: Option<String>,
    pub files_analyzed: usize,
    /// Graph files the blast percents range over; equals `files_analyzed`
    /// except when analysis covers a sub-scope (e.g. a single file).
    pub scope_files: usize,
    pub risks: Vec<RiskRow>,
    pub truncated: bool,
}

/// Builds a ranked change-risk report from analyzed files and joined facts.
///
/// Files missing from `history` keep their static dimensions and carry `None`
/// churn/ownership. Files missing from `graph` read zero fan-in/out and zero
/// impact, defensively.
pub fn build(
    analysis: &AnalysisReport,
    history: &HistoryReport,
    graph: &DependencyReport,
    window: HistoryWindow,
    limit: usize,
) -> RiskReport {
    // ponytail: one BFS per file over a single shared reverse map plus
    // one BTreeMap lookup per join; the per-file BFS work is inherent to
    // reachability
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
    let counts = impact_counts(graph);
    let mut risks: Vec<RiskRow> = analysis
        .files
        .iter()
        .map(|file| {
            let mut max_cognitive = 0_u32;
            let mut max_cyclomatic = 0_u32;
            let mut max_crap: Option<f64> = None;
            for function in &file.functions {
                max_cognitive = max_cognitive.max(function.metrics.cognitive);
                max_cyclomatic = max_cyclomatic.max(function.metrics.cyclomatic);
                if let Some(crap) = function.metrics.crap {
                    max_crap = Some(max_crap.map_or(crap, |current: f64| current.max(crap)));
                }
            }

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

            let complexity = Some(
                100.0
                    * (f64::from(max_cognitive) / 30.0)
                        .min(1.0)
                        .max((f64::from(max_cyclomatic) / 20.0).min(1.0)),
            );
            let crap = max_crap.map(|value| 100.0 * (value / 30.0).min(1.0));
            let churn = changes.map(|n| 100.0 * (n as f64 / 20.0).min(1.0));
            let impact = Some(blast_radius_percent.min(100.0));
            let ownership =
                contributors.and_then(|n| if n == 0 { None } else { Some(100.0 / n as f64) });
            let components = RiskComponents {
                complexity,
                crap,
                churn,
                impact,
                ownership,
                policy: None,
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
                    blast_radius,
                    blast_radius_percent,
                    fan_in,
                    fan_out,
                },
            }
        })
        .collect();
    risks.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.cmp(&right.path))
    });
    let truncated = risks.len() > limit;
    risks.truncate(limit);
    RiskReport {
        schema_version: RISK_SCHEMA_VERSION,
        analyzer_version: analysis.analyzer_version,
        metric_profile: analysis.metric_profile,
        model: RISK_MODEL,
        window: window.label(),
        git_available: history.available,
        head_commit: history.head_commit.clone(),
        files_analyzed: analysis.files.len(),
        scope_files: graph.files.len(),
        risks,
        truncated,
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
