//! Hotspot analysis: static source metrics joined with Git history.
//!
//! A hotspot is a file where complexity meets change. This module keeps the
//! underlying dimensions (complexity, CRAP, coverage, churn, contributors)
//! explicit in the report. The only derived number is a documented,
//! deterministic ordering product:
//!
//! ```text
//! hotspot score = max cognitive complexity x changes in the selected window
//! ```
//!
//! `HOTSPOT_MODEL` names that rule so later versions can evolve it without
//! changing the dimension fields. Composite risk models belong to a separate,
//! explainable layer.

use crate::core::{AnalysisReport, FileAnalysis, Language};
use crate::history::{FileHistory, HistoryReport, HistoryWindow};
use serde::Serialize;
use std::collections::BTreeMap;

pub const HOTSPOT_SCHEMA_VERSION: u32 = 1;
pub const HOTSPOT_MODEL: &str = "complexity-x-churn";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hotspot {
    pub path: String,
    pub language: Language,
    /// Sum of function LOC in the file.
    pub loc: u64,
    pub functions: usize,
    pub max_cognitive: u32,
    pub max_cyclomatic: u32,
    pub max_crap: Option<f64>,
    /// LOC-weighted mean coverage over functions that have coverage data.
    pub coverage: Option<f64>,
    pub functions_with_coverage: usize,
    pub commits: Option<u64>,
    /// Changes in the report's selected window.
    pub changes: Option<u64>,
    pub changes_30d: Option<u64>,
    pub changes_90d: Option<u64>,
    pub changes_365d: Option<u64>,
    pub lines_added: Option<u64>,
    pub lines_deleted: Option<u64>,
    pub days_since_last_change: Option<u64>,
    pub contributors: Option<u64>,
    pub recent_contributors: Option<u64>,
    /// `max_cognitive x changes`; `null` when Git history is unavailable.
    pub score: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HotspotReport {
    pub schema_version: u32,
    pub analyzer_version: &'static str,
    pub metric_profile: &'static str,
    pub model: &'static str,
    pub window: &'static str,
    pub git_available: bool,
    pub head_commit: Option<String>,
    pub files_analyzed: usize,
    pub hotspots: Vec<Hotspot>,
    pub truncated: bool,
}

/// Builds a ranked hotspot report from analyzed files and historical facts.
///
/// Files missing from `history` (unknown languages, deleted files, or a
/// repository without Git) keep their complexity dimensions and carry `null`
/// churn. Ranking falls back to complexity when history is unavailable.
pub fn build(
    analysis: &AnalysisReport,
    history: &HistoryReport,
    window: HistoryWindow,
    limit: usize,
) -> HotspotReport {
    let history_by_path: BTreeMap<&str, &FileHistory> = history
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let mut hotspots: Vec<Hotspot> = analysis
        .files
        .iter()
        .map(|file| {
            hotspot(
                file,
                history_by_path.get(file.path.as_str()).copied(),
                window,
            )
        })
        .collect();
    hotspots.sort_by(|left, right| {
        rank(right)
            .cmp(&rank(left))
            .then_with(|| left.path.cmp(&right.path))
    });
    let truncated = hotspots.len() > limit;
    hotspots.truncate(limit);
    HotspotReport {
        schema_version: HOTSPOT_SCHEMA_VERSION,
        analyzer_version: analysis.analyzer_version,
        metric_profile: analysis.metric_profile,
        model: HOTSPOT_MODEL,
        window: window.label(),
        git_available: history.available,
        head_commit: history.head_commit.clone(),
        files_analyzed: analysis.files.len(),
        hotspots,
        truncated,
    }
}

fn hotspot(file: &FileAnalysis, history: Option<&FileHistory>, window: HistoryWindow) -> Hotspot {
    let mut loc = 0_u64;
    let mut max_cognitive = 0_u32;
    let mut max_cyclomatic = 0_u32;
    let mut max_crap: Option<f64> = None;
    let mut coverage_weighted = 0.0_f64;
    let mut coverage_weight = 0_u64;
    let mut functions_with_coverage = 0_usize;

    for function in &file.functions {
        let metrics = &function.metrics;
        loc += u64::from(metrics.loc);
        max_cognitive = max_cognitive.max(metrics.cognitive);
        max_cyclomatic = max_cyclomatic.max(metrics.cyclomatic);
        if let Some(crap) = metrics.crap {
            max_crap = Some(max_crap.map_or(crap, |current: f64| current.max(crap)));
        }
        if let Some(coverage) = metrics.coverage {
            functions_with_coverage += 1;
            let weight = u64::from(metrics.loc).max(1);
            coverage_weighted += coverage * weight as f64;
            coverage_weight += weight;
        }
    }

    let coverage = (coverage_weight > 0).then(|| coverage_weighted / coverage_weight as f64);
    let score = history.map(|file| u64::from(max_cognitive).saturating_mul(window.changes(file)));
    Hotspot {
        path: file.path.clone(),
        language: file.language,
        loc,
        functions: file.functions.len(),
        max_cognitive,
        max_cyclomatic,
        max_crap,
        coverage,
        functions_with_coverage,
        commits: history.map(|file| file.commits),
        changes: history.map(|file| window.changes(file)),
        changes_30d: history.map(|file| file.changes_30d),
        changes_90d: history.map(|file| file.changes_90d),
        changes_365d: history.map(|file| file.changes_365d),
        lines_added: history.map(|file| file.lines_added),
        lines_deleted: history.map(|file| file.lines_deleted),
        days_since_last_change: history.map(|file| file.days_since_last_change),
        contributors: history.map(|file| file.contributors),
        recent_contributors: history.map(|file| file.recent_contributors),
        score,
    }
}

fn rank(hotspot: &Hotspot) -> (u64, u32, u32) {
    (
        hotspot.score.unwrap_or(0),
        hotspot.max_cognitive,
        hotspot.max_cyclomatic,
    )
}
