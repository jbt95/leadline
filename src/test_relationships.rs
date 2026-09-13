//! Explicit test-to-code relationships.
//!
//! Relationships are supplied by a versioned mapping file; nothing is guessed
//! from file names or directory layout. Unresolved rows stay visible with a
//! stable reason instead of failing the whole report.

use crate::Result;
use crate::core::AnalysisReport;
use crate::external::{
    INPUT_BYTES_LIMIT, InputBudget, read_bounded, strict_relative_path, validate_json_depth,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub const TEST_RELATIONSHIP_SCHEMA_VERSION: u32 = 1;

/// One normalized relationship row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TestRelationship {
    pub test_path: Option<String>,
    pub target_path: Option<String>,
    pub target_function: Option<String>,
    /// Function id when the optional name resolved uniquely.
    pub target_function_id: Option<String>,
    pub resolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TestRelationshipReport {
    pub schema_version: u32,
    pub analyzer_version: &'static str,
    pub relationships: Vec<TestRelationship>,
    pub unresolved: u64,
}

/// Parses every mapping file and resolves rows against the analysis.
pub fn ingest(
    paths: &[PathBuf],
    analysis: &AnalysisReport,
    budget: &mut InputBudget,
) -> Result<TestRelationshipReport> {
    let mut merged: BTreeSet<String> = BTreeSet::new();
    let mut relationships = Vec::new();
    for path in paths {
        let bytes = read_bounded(path, INPUT_BYTES_LIMIT, budget)?;
        validate_json_depth(&bytes)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("test map {}: {error}", path.display()))?;
        let schema = value
            .get("schema_version")
            .and_then(|schema| schema.as_u64())
            .ok_or_else(|| format!("test map {}: missing schema_version", path.display()))?;
        if schema != u64::from(TEST_RELATIONSHIP_SCHEMA_VERSION) {
            return Err(format!(
                "test map {}: unsupported schema_version {schema}",
                path.display()
            )
            .into());
        }
        let rows = value
            .get("relationships")
            .and_then(|rows| rows.as_array())
            .ok_or_else(|| format!("test map {}: missing relationships array", path.display()))?;
        budget.consume_rows(rows.len() as u64)?;
        for row in rows {
            let relationship = normalize_row(row, analysis);
            let key = serde_json::to_string(&relationship).unwrap_or_default();
            if merged.insert(key) {
                relationships.push(relationship);
            }
        }
    }
    relationships.sort_by(|left, right| {
        left.target_path
            .cmp(&right.target_path)
            .then(left.target_function.cmp(&right.target_function))
            .then(left.test_path.cmp(&right.test_path))
    });
    let unresolved = relationships
        .iter()
        .filter(|relationship| !relationship.resolved)
        .count() as u64;
    Ok(TestRelationshipReport {
        schema_version: TEST_RELATIONSHIP_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        relationships,
        unresolved,
    })
}

fn normalize_row(row: &serde_json::Value, analysis: &AnalysisReport) -> TestRelationship {
    let test_path = row.get("test_path").and_then(|path| path.as_str());
    let target_path = row.get("target_path").and_then(|path| path.as_str());
    let target_function = row
        .get("target_function")
        .and_then(|function| function.as_str())
        .map(str::to_owned);

    let resolved_test = test_path.and_then(|path| resolve_path(path, analysis));
    let resolved_target = target_path.and_then(|path| resolve_path(path, analysis));
    let (target_function_id, function_reason) = match (&resolved_target, &target_function) {
        (Some(path), Some(name)) => match analysis.files.iter().find(|file| &file.path == path) {
            Some(file) => {
                let mut matches = file
                    .functions
                    .iter()
                    .filter(|function| function.name == *name);
                match (matches.next(), matches.next()) {
                    (Some(function), None) => (Some(function.id.clone()), None),
                    (Some(_), Some(_)) => (None, Some("ambiguous function name".to_owned())),
                    (None, _) => (None, Some("unknown function name".to_owned())),
                }
            }
            None => (None, Some("unknown target file".to_owned())),
        },
        (Some(_), None) => (None, None),
        (None, _) => (None, Some("unknown target file".to_owned())),
    };

    let reason = if resolved_test.is_none() {
        Some("unknown test file".to_owned())
    } else if resolved_target.is_none() {
        Some("unknown target file".to_owned())
    } else {
        function_reason
    };
    let resolved = resolved_test.is_some() && resolved_target.is_some() && reason.is_none();
    TestRelationship {
        test_path: resolved_test.or_else(|| test_path.map(str::to_owned)),
        target_path: resolved_target.or_else(|| target_path.map(str::to_owned)),
        target_function,
        target_function_id,
        resolved,
        reason,
    }
}

fn resolve_path(path: &str, analysis: &AnalysisReport) -> Option<String> {
    let normalized = strict_relative_path(path).ok()?;
    if analysis.files.iter().any(|file| file.path == normalized) {
        return Some(normalized);
    }
    let base = normalized.rsplit('/').next().unwrap_or(&normalized);
    let mut matches = analysis
        .files
        .iter()
        .map(|file| file.path.as_str())
        .filter(|candidate| candidate.rsplit('/').next() == Some(base));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first.to_owned())
}
