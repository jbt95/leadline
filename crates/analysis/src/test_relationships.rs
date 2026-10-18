//! Explicit test-to-code relationships.
//!
//! Relationships are supplied by a versioned mapping file; nothing is guessed
//! from file names or directory layout. Unresolved rows stay visible with a
//! stable reason instead of failing the whole report. A row names its test by
//! the namespaced `test_id` a runner accepts and its function by the stable
//! `symbol_key` from [`model::symbol`], so a map recorded in one checkout can be
//! read against another without line numbers or bare names.

use model::Result;
use model::core::AnalysisReport;
use parse::external::{
    INPUT_BYTES_LIMIT, InputBudget, read_bounded, strict_relative_path, validate_json_depth,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const TEST_RELATIONSHIP_SCHEMA_VERSION: u32 = 2;

/// One normalized relationship row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TestRelationship {
    /// Namespaced `path::name` test identifier, supplied by the map.
    pub test_id: String,
    pub test_path: Option<String>,
    /// Authoritative target: the stable key of the exercised function.
    pub symbol_key: String,
    /// Readable path of the target, also used by the directory escalation.
    pub target_path: Option<String>,
    /// Whether the row's test and symbol key both resolve in this analysis.
    pub resolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TestRelationshipReport {
    pub schema_version: u32,
    pub analyzer_version: &'static str,
    /// Producer that wrote the map.
    pub recorder: String,
    /// Commit the recorded suite ran against.
    pub recorded_at: String,
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
    let mut provenance: Option<(String, String)> = None;
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
                "test map {}: unsupported schema_version {schema}, expected {}",
                path.display(),
                TEST_RELATIONSHIP_SCHEMA_VERSION
            )
            .into());
        }
        let recorder = required_string(&value, "recorder", path, None)?.to_owned();
        let recorded_at = required_string(&value, "recorded_at", path, None)?.to_owned();
        match &provenance {
            Some((known_recorder, known_recorded_at))
                if *known_recorder != recorder || *known_recorded_at != recorded_at =>
            {
                return Err(format!(
                    "test map {}: recorded by {recorder} at {recorded_at}, but an earlier map \
                     was recorded by {known_recorder} at {known_recorded_at}",
                    path.display()
                )
                .into());
            }
            Some(_) => {}
            None => provenance = Some((recorder, recorded_at)),
        }
        let rows = value
            .get("relationships")
            .and_then(|rows| rows.as_array())
            .ok_or_else(|| format!("test map {}: missing relationships array", path.display()))?;
        budget.consume_rows(rows.len() as u64)?;
        for (index, row) in rows.iter().enumerate() {
            let relationship = normalize_row(row, analysis, path, index)?;
            let key = serde_json::to_string(&relationship).unwrap_or_default();
            if merged.insert(key) {
                relationships.push(relationship);
            }
        }
    }
    relationships.sort_by(|left, right| {
        left.target_path
            .cmp(&right.target_path)
            .then(left.symbol_key.cmp(&right.symbol_key))
            .then(left.test_id.cmp(&right.test_id))
    });
    let unresolved = relationships
        .iter()
        .filter(|relationship| !relationship.resolved)
        .count() as u64;
    let (recorder, recorded_at) = provenance.unwrap_or_default();
    Ok(TestRelationshipReport {
        schema_version: TEST_RELATIONSHIP_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        recorder,
        recorded_at,
        relationships,
        unresolved,
    })
}

fn normalize_row(
    row: &serde_json::Value,
    analysis: &AnalysisReport,
    map: &Path,
    index: usize,
) -> Result<TestRelationship> {
    let test_id = required_string(row, "test_id", map, Some(index))?.to_owned();
    let symbol_key = required_string(row, "symbol_key", map, Some(index))?.to_owned();
    let test_path = row.get("test_path").and_then(|path| path.as_str());
    let target_path = row.get("target_path").and_then(|path| path.as_str());

    let resolved_test = test_path.and_then(|path| resolve_path(path, analysis));
    let resolved_target = target_path.and_then(|path| resolve_path(path, analysis));
    let symbol_reason = match_symbol(&symbol_key, analysis);

    let reason = if resolved_test.is_none() {
        Some("unknown test file".to_owned())
    } else {
        symbol_reason.map(str::to_owned)
    };
    let resolved = reason.is_none();
    Ok(TestRelationship {
        test_id,
        test_path: resolved_test.or_else(|| test_path.map(str::to_owned)),
        symbol_key,
        target_path: resolved_target.or_else(|| target_path.map(str::to_owned)),
        resolved,
        reason,
    })
}

/// Reads a required string field, naming the map and row it came from.
fn required_string<'a>(
    value: &'a serde_json::Value,
    field: &str,
    map: &Path,
    row: Option<usize>,
) -> Result<&'a str> {
    value
        .get(field)
        .and_then(|field| field.as_str())
        .ok_or_else(|| {
            let location = match row {
                Some(index) => format!("test map {}: relationship {index}", map.display()),
                None => format!("test map {}", map.display()),
            };
            format!("{location}: missing {field}").into()
        })
}

/// Stable reason when the key matches no current symbol, or more than one.
///
/// A key that resolves to nothing keeps its row: ingest cannot know which
/// checkout the map will later be read against.
fn match_symbol(symbol_key: &str, analysis: &AnalysisReport) -> Option<&'static str> {
    let mut matches = analysis
        .files
        .iter()
        .flat_map(|file| file.functions.iter())
        .filter(|function| function.symbol_key == symbol_key);
    match (matches.next(), matches.next()) {
        (Some(_), None) => None,
        (Some(_), Some(_)) => Some("ambiguous symbol key"),
        (None, _) => Some("unknown symbol key"),
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
