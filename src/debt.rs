//! Full-state debt and risk comparison.
//!
//! Function debt classifies threshold transitions per dimension; entity
//! absence is a known non-violation boundary while a present function with a
//! missing metric is unknown and never becomes a finding. Risk changes compare
//! complete per-side scores and expose component deltas.

use crate::config::Thresholds;
use crate::core::{FileAnalysis, FunctionAnalysis};
use crate::risk::RiskReport;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const DEBT_SCHEMA_VERSION: u32 = 1;
pub const RISK_CHANGE_MODEL: &str = "change-risk-diff";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DebtStatus {
    New,
    Existing,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DebtFinding {
    pub path: String,
    pub name: String,
    pub dimension: &'static str,
    pub threshold: f64,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub status: DebtStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskChangeStatus {
    Added,
    Increased,
    Decreased,
    Removed,
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComponentDelta {
    pub component: &'static str,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub delta: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RiskChange {
    pub path: String,
    pub status: RiskChangeStatus,
    pub before_score: Option<f64>,
    pub after_score: Option<f64>,
    pub delta: Option<f64>,
    pub components: Vec<ComponentDelta>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DebtSummary {
    pub new: u64,
    pub existing: u64,
    pub resolved: u64,
    /// One count per configured dimension/function pair whose classification
    /// is blocked by a missing value.
    pub unknown: u64,
    pub risk_increased: u64,
    pub risk_decreased: u64,
    pub risk_added: u64,
    pub risk_removed: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DebtReport {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub model: &'static str,
    pub base: String,
    pub base_commit: Option<String>,
    pub target_commit: Option<String>,
    pub summary: DebtSummary,
    pub findings: Vec<DebtFinding>,
    pub risk_changes: Vec<RiskChange>,
}

/// One side's risk rows reduced to comparable path/score/components.
#[derive(Clone, Debug, PartialEq)]
pub struct RiskSideEntry {
    pub path: String,
    pub score: Option<f64>,
    pub components: BTreeMap<&'static str, Option<f64>>,
}

pub const RISK_COMPONENTS: [&str; 6] = [
    "complexity",
    "crap",
    "churn",
    "impact",
    "ownership",
    "policy",
];

/// Projects a risk report into comparable entries.
pub fn risk_entries(report: &RiskReport) -> Vec<RiskSideEntry> {
    report
        .risks
        .iter()
        .map(|row| {
            let components = [
                ("complexity", row.components.complexity),
                ("crap", row.components.crap),
                ("churn", row.components.churn),
                ("impact", row.components.impact),
                ("ownership", row.components.ownership),
                ("policy", row.components.policy),
            ]
            .into_iter()
            .collect();
            RiskSideEntry {
                path: row.path.clone(),
                score: Some(row.score),
                components,
            }
        })
        .collect()
}

/// Pairs complete function sets by `(path, name, same-name source order)`.
///
/// Unlike changed-code pairing, unchanged fingerprints are kept: `existing`
/// debt is only visible when clean pairs are present.
pub fn pair_complete<'a>(
    before: &'a [&'a crate::core::FileAnalysis],
    after: &'a [&'a crate::core::FileAnalysis],
) -> Vec<(
    String,
    Option<&'a FunctionAnalysis>,
    Option<&'a FunctionAnalysis>,
)> {
    fn group<'a>(
        files: &[&'a FileAnalysis],
    ) -> BTreeMap<String, BTreeMap<String, Vec<&'a FunctionAnalysis>>> {
        let mut by_path: BTreeMap<String, BTreeMap<String, Vec<&FunctionAnalysis>>> =
            BTreeMap::new();
        for file in files {
            for function in &file.functions {
                by_path
                    .entry(file.path.clone())
                    .or_default()
                    .entry(function.name.clone())
                    .or_default()
                    .push(function);
            }
        }
        by_path
    }
    let before_paths = group(before);
    let after_paths = group(after);
    let paths: BTreeSet<&String> = before_paths.keys().chain(after_paths.keys()).collect();
    let mut pairs = Vec::new();
    for path in paths {
        let empty = BTreeMap::new();
        let before_names = before_paths.get(path).unwrap_or(&empty);
        let after_names = after_paths.get(path).unwrap_or(&empty);
        let names: BTreeSet<&String> = before_names.keys().chain(after_names.keys()).collect();
        for name in names {
            let before_group = before_names.get(name).map(Vec::as_slice).unwrap_or(&[]);
            let after_group = after_names.get(name).map(Vec::as_slice).unwrap_or(&[]);
            let count = before_group.len().max(after_group.len());
            for index in 0..count {
                pairs.push((
                    path.clone(),
                    before_group.get(index).copied(),
                    after_group.get(index).copied(),
                ));
            }
        }
    }
    pairs
}

/// Classifies threshold-state transitions over complete paired functions.
pub fn classify(
    before_files: &[&crate::core::FileAnalysis],
    after_files: &[&crate::core::FileAnalysis],
    thresholds: &Thresholds,
) -> (Vec<DebtFinding>, u64) {
    let mut findings = Vec::new();
    let mut unknown = 0_u64;
    for (path, before, after) in pair_complete(before_files, after_files) {
        type Dimension = (&'static str, Option<f64>, Option<f64>, Option<f64>);
        let dimensions: [Dimension; 4] = [
            (
                "cognitive",
                thresholds.cognitive.map(f64::from),
                before.map(|function| f64::from(function.metrics.cognitive)),
                after.map(|function| f64::from(function.metrics.cognitive)),
            ),
            (
                "cyclomatic",
                thresholds.cyclomatic.map(f64::from),
                before.map(|function| f64::from(function.metrics.cyclomatic)),
                after.map(|function| f64::from(function.metrics.cyclomatic)),
            ),
            (
                "crap",
                thresholds.crap,
                before.and_then(|function| function.metrics.crap),
                after.and_then(|function| function.metrics.crap),
            ),
            (
                "max_nesting",
                thresholds.max_nesting.map(f64::from),
                before.map(|function| f64::from(function.metrics.max_nesting)),
                after.map(|function| f64::from(function.metrics.max_nesting)),
            ),
        ];
        for (dimension, limit, before_value, after_value) in dimensions {
            let Some(limit) = limit else { continue };
            let name = before
                .map(|function| function.name.clone())
                .or_else(|| after.map(|function| function.name.clone()))
                .unwrap_or_default();
            // Entity absence is a known non-violation; a present function
            // with a missing metric stays unknown.
            let before_state = match before {
                None => Some(false),
                Some(_) => state(before_value, limit),
            };
            let after_state = match after {
                None => Some(false),
                Some(_) => state(after_value, limit),
            };
            match (before_state, after_state) {
                (Some(false), Some(true)) => findings.push(DebtFinding {
                    path: path.clone(),
                    name,
                    dimension,
                    threshold: limit,
                    before: before_value,
                    after: after_value,
                    status: DebtStatus::New,
                }),
                (Some(true), Some(true)) => findings.push(DebtFinding {
                    path: path.clone(),
                    name,
                    dimension,
                    threshold: limit,
                    before: before_value,
                    after: after_value,
                    status: DebtStatus::Existing,
                }),
                (Some(true), Some(false)) => findings.push(DebtFinding {
                    path: path.clone(),
                    name,
                    dimension,
                    threshold: limit,
                    before: before_value,
                    after: after_value,
                    status: DebtStatus::Resolved,
                }),
                (Some(false), Some(false)) => {}
                _ => unknown += 1,
            }
        }
    }
    findings.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.name.cmp(&right.name))
            .then(left.dimension.cmp(right.dimension))
    });
    (findings, unknown)
}

fn state(value: Option<f64>, limit: f64) -> Option<bool> {
    value.map(|value| value > limit)
}

/// Classifies per-path score changes, applying rename mapping to the base side.
pub fn compare_risks(
    before: &[RiskSideEntry],
    after: &[RiskSideEntry],
    renames: &BTreeMap<String, String>,
) -> Vec<RiskChange> {
    let mut before_rows: BTreeMap<String, &RiskSideEntry> = BTreeMap::new();
    for entry in before {
        let path = renames
            .get(&entry.path)
            .cloned()
            .unwrap_or_else(|| entry.path.clone());
        before_rows.insert(path, entry);
    }
    let after_rows: BTreeMap<&str, &RiskSideEntry> = after
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    let paths: BTreeSet<&str> = before_rows
        .keys()
        .map(String::as_str)
        .chain(after_rows.keys().copied())
        .collect();
    let mut changes = Vec::new();
    for path in paths {
        let before_row = before_rows.get(path).copied();
        let after_row = after_rows.get(path).copied();
        let status = match (before_row, after_row) {
            (None, Some(_)) => RiskChangeStatus::Added,
            (Some(_), None) => RiskChangeStatus::Removed,
            (Some(before_row), Some(after_row)) => match (before_row.score, after_row.score) {
                (Some(before_score), Some(after_score)) => {
                    if after_score > before_score {
                        RiskChangeStatus::Increased
                    } else if after_score < before_score {
                        RiskChangeStatus::Decreased
                    } else {
                        RiskChangeStatus::Unchanged
                    }
                }
                _ => RiskChangeStatus::Unchanged,
            },
            (None, None) => continue,
        };
        let components = RISK_COMPONENTS
            .iter()
            .map(|component| {
                let before_value = before_row
                    .and_then(|row| row.components.get(component))
                    .copied()
                    .flatten();
                let after_value = after_row
                    .and_then(|row| row.components.get(component))
                    .copied()
                    .flatten();
                ComponentDelta {
                    component,
                    before: before_value,
                    after: after_value,
                    delta: match (before_value, after_value) {
                        (Some(before_value), Some(after_value)) => Some(after_value - before_value),
                        _ => None,
                    },
                }
            })
            .collect();
        let before_score = before_row.and_then(|row| row.score);
        let after_score = after_row.and_then(|row| row.score);
        changes.push(RiskChange {
            path: path.to_owned(),
            status,
            before_score,
            after_score,
            delta: match (before_score, after_score) {
                (Some(before_score), Some(after_score)) => Some(after_score - before_score),
                _ => None,
            },
            components,
        });
    }
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    changes
}

/// Summarizes findings and risk changes.
pub fn summarize(
    findings: &[DebtFinding],
    unknown: u64,
    risk_changes: &[RiskChange],
) -> DebtSummary {
    let mut summary = DebtSummary {
        unknown,
        ..DebtSummary::default()
    };
    for finding in findings {
        match finding.status {
            DebtStatus::New => summary.new += 1,
            DebtStatus::Existing => summary.existing += 1,
            DebtStatus::Resolved => summary.resolved += 1,
        }
    }
    for change in risk_changes {
        match change.status {
            RiskChangeStatus::Increased => summary.risk_increased += 1,
            RiskChangeStatus::Decreased => summary.risk_decreased += 1,
            RiskChangeStatus::Added => summary.risk_added += 1,
            RiskChangeStatus::Removed => summary.risk_removed += 1,
            RiskChangeStatus::Unchanged => {}
        }
    }
    summary
}
