//! Security finding vocabulary shared by every scanner adapter.
//!
//! These are the shapes and severity rules the scanner reports, gates, and
//! adapters agree on. Reading scanner input and joining findings against the
//! project model stay with the security assembler, which sits above the project
//! model and therefore above this crate.

use serde::Serialize;

/// Normalized finding severity. Declaration order is ascending severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecuritySeverity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

/// New/existing state against supplied baselines or SARIF `baselineState`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingState {
    New,
    Existing,
    Unknown,
}

/// One normalized finding. Unavailable evidence stays `None` and serializes
/// as `null`; enrichment fills the `Option` analytics fields later.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SecurityFinding {
    pub tool: String,
    pub rule_id: String,
    pub severity: SecuritySeverity,
    pub path: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub fingerprint: String,
    pub state: FindingState,
    pub changed: Option<bool>,
    pub report_ids: Vec<String>,
    pub function_id: Option<String>,
    pub cognitive: Option<u32>,
    pub cyclomatic: Option<u32>,
    pub crap: Option<f64>,
    pub coverage: Option<f64>,
    pub risk_score: Option<f64>,
    pub fan_in: Option<u64>,
    pub blast_radius: Option<u64>,
    pub concentration_percent: Option<f64>,
    pub reason: Option<String>,
}

/// Git comparison backing `changed` attribution.
#[derive(Clone, Debug, PartialEq)]
pub enum ChangeComparison {
    Base(String),
    Staged,
    Target(String),
}

impl SecuritySeverity {
    /// Stable lowercase identifier shared by terminal output and gates.
    pub fn as_str(self) -> &'static str {
        match self {
            SecuritySeverity::Unknown => "unknown",
            SecuritySeverity::Low => "low",
            SecuritySeverity::Medium => "medium",
            SecuritySeverity::High => "high",
            SecuritySeverity::Critical => "critical",
        }
    }
}

impl FindingState {
    /// Stable lowercase identifier shared by terminal output.
    pub fn as_str(self) -> &'static str {
        match self {
            FindingState::New => "new",
            FindingState::Existing => "existing",
            FindingState::Unknown => "unknown",
        }
    }
}

/// Parse a `--fail-on-severity` gate level.
pub fn parse_gate_severity(raw: &str) -> Option<SecuritySeverity> {
    match raw {
        "low" => Some(SecuritySeverity::Low),
        "medium" => Some(SecuritySeverity::Medium),
        "high" => Some(SecuritySeverity::High),
        "critical" => Some(SecuritySeverity::Critical),
        _ => None,
    }
}

/// Merge provenance report IDs into one row, keeping them sorted and unique.
pub fn merge_report_ids(existing: &mut Vec<String>, incoming: &[String]) {
    for id in incoming {
        if !existing.contains(id) {
            existing.push(id.clone());
        }
    }
    existing.sort();
}

/// Shared bounded agent projection for every scanner report: first `top`
/// findings plus a `truncated` flag.
pub fn agent_page<T: Serialize>(findings: &[T], top: usize) -> serde_json::Value {
    serde_json::json!({
        "findings": findings.iter().take(top).collect::<Vec<_>>(),
        "truncated": findings.len() > top,
    })
}

/// Sort rank for finding state: `new`, `existing`, `unknown`.
pub fn state_rank(state: FindingState) -> u8 {
    match state {
        FindingState::New => 0,
        FindingState::Existing => 1,
        FindingState::Unknown => 2,
    }
}

/// Shared literal severity labels for scanner adapters (`moderate` is medium).
///
/// Case-insensitive; anything unrecognized stays unknown.
pub fn severity_from_label(text: &str) -> SecuritySeverity {
    match text.to_ascii_lowercase().as_str() {
        "critical" => SecuritySeverity::Critical,
        "high" | "error" => SecuritySeverity::High,
        "medium" | "moderate" | "warning" => SecuritySeverity::Medium,
        "low" | "note" => SecuritySeverity::Low,
        _ => SecuritySeverity::Unknown,
    }
}

/// Shared numeric 0-10 scores: `<4` low, `<7` medium, `<9` high, `>=9` critical.
pub fn severity_from_score(score: f64) -> SecuritySeverity {
    if !score.is_finite() {
        SecuritySeverity::Unknown
    } else if score < 4.0 {
        SecuritySeverity::Low
    } else if score < 7.0 {
        SecuritySeverity::Medium
    } else if score < 9.0 {
        SecuritySeverity::High
    } else {
        SecuritySeverity::Critical
    }
}
