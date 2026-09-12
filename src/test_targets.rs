//! Coverage-aware test targets: rank functions by uncovered risk.
//!
//! Pure over an already-covered [`AnalysisReport`](crate::core::AnalysisReport):
//! a contribution line is uncovered only with known zero hits, covered with
//! any positive hit count, and unknown when no record names the line.
//! Unknown lines are reported separately and never count as uncovered.
//! Coverage is line coverage only, never branch coverage.

use crate::core::{AnalysisReport, MetricContribution};
use crate::coverage::CoverageMap;
use serde::Serialize;

/// One function worth testing: identity, risk, and decision lines by state.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TestTarget {
    pub path: String,
    pub function: String,
    pub line: u32,
    pub crap: Option<f64>,
    pub coverage: Option<f64>,
    /// Contributions on lines with known zero hits.
    pub uncovered: Vec<MetricContribution>,
    /// Contributions on lines absent from the coverage record.
    pub unknown: Vec<MetricContribution>,
}

/// Rank functions holding at least one known-zero-hit contribution line.
///
/// Sorted by CRAP descending (missing CRAP last), then path, function, line.
pub fn test_targets(report: &AnalysisReport, coverage: &CoverageMap) -> Vec<TestTarget> {
    let mut targets = Vec::new();
    for file in &report.files {
        for function in &file.functions {
            let mut uncovered = Vec::new();
            let mut unknown = Vec::new();
            for contribution in &function.contributions {
                match coverage.hits(&file.path, contribution.line) {
                    Some(0) => uncovered.push(contribution.clone()),
                    None => unknown.push(contribution.clone()),
                    Some(_) => {}
                }
            }
            if uncovered.is_empty() {
                continue;
            }
            targets.push(TestTarget {
                path: file.path.clone(),
                function: function.name.clone(),
                line: function.start_line,
                crap: function.metrics.crap,
                coverage: function.metrics.coverage,
                uncovered,
                unknown,
            });
        }
    }
    targets.sort_by(|left, right| {
        cmp_crap_desc(left.crap, right.crap)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.function.cmp(&right.function))
            .then_with(|| left.line.cmp(&right.line))
    });
    targets
}

/// Descending CRAP; missing CRAP sorts last.
fn cmp_crap_desc(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(a), Some(b)) => b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}
