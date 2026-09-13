//! Mutation-report ingestion: PIT `mutations.xml` and Stryker JSON.
//!
//! Both providers normalize into one row shape with provider-native identity
//! preserved as provenance. Nothing here executes tools, resolves packages,
//! or guesses function identity by name alone.

use crate::Result;
use crate::core::AnalysisReport;
use crate::external::{
    INPUT_BYTES_LIMIT, InputBudget, read_bounded, strict_relative_path, validate_json_depth,
    validate_xml_preamble_and_depth,
};
use crate::source_snapshot::SourceEntry;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const MUTATION_SCHEMA_VERSION: u32 = 1;
pub const MUTATION_MODEL: &str = "mutation";

/// One external mutation report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MutationInput {
    Pit(PathBuf),
    Stryker(PathBuf),
}

/// Normalized mutation status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationStatus {
    Killed,
    TimedOut,
    Survived,
    NoCoverage,
    Ignored,
    CompileError,
    Error,
    Unknown,
}

impl MutationStatus {
    /// Maps provider status spellings; unknown spellings stay `unknown`.
    pub fn parse(raw: &str) -> MutationStatus {
        let normalized: String = raw
            .chars()
            .filter(|character| !matches!(character, '_' | '-'))
            .flat_map(char::to_lowercase)
            .collect();
        match normalized.as_str() {
            "killed" => MutationStatus::Killed,
            "timedout" | "timeout" => MutationStatus::TimedOut,
            "survived" => MutationStatus::Survived,
            "nocoverage" => MutationStatus::NoCoverage,
            "ignored" | "pending" => MutationStatus::Ignored,
            "nonviable" | "compileerror" => MutationStatus::CompileError,
            "runerror" | "runtimeerror" | "memoryerror" => MutationStatus::Error,
            _ => MutationStatus::Unknown,
        }
    }

    fn from_killed(detected: bool) -> MutationStatus {
        if detected {
            MutationStatus::Killed
        } else {
            MutationStatus::Survived
        }
    }
}

/// One normalized mutant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MutationRow {
    pub provider: &'static str,
    /// Reporter inputs this exact mutant came from, sorted and deduplicated.
    pub report_ids: Vec<String>,
    pub native_id: Option<String>,
    pub status: MutationStatus,
    pub raw_status: String,
    pub operator: String,
    pub description: Option<String>,
    /// Analysis-root-relative path when resolved.
    pub path: Option<String>,
    /// 1-based inclusive start line and exclusive end line.
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub start_column: Option<u32>,
    pub end_column: Option<u32>,
    pub function_id: Option<String>,
    /// Stable reason when the mutant could not be attributed.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MutationSummary {
    pub total: u64,
    pub killed: u64,
    pub timed_out: u64,
    pub survived: u64,
    pub no_coverage: u64,
    pub ignored: u64,
    pub compile_error: u64,
    pub error: u64,
    pub unknown: u64,
    pub unresolved: u64,
    /// `100 * (killed + timed_out) / scored`, `null` for an empty denominator.
    pub score: Option<f64>,
    pub scored_mutants: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MutationReport {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub model: &'static str,
    pub summary: MutationSummary,
    pub mutants: Vec<MutationRow>,
}

/// Parses and merges every input against the analyzed sources.
pub fn ingest(
    inputs: &[MutationInput],
    entries: &[SourceEntry],
    analysis: &AnalysisReport,
    budget: &mut InputBudget,
) -> Result<MutationReport> {
    let mut merged: BTreeMap<String, MutationRow> = BTreeMap::new();
    for input in inputs {
        let path = match input {
            MutationInput::Pit(path) | MutationInput::Stryker(path) => path,
        };
        let bytes = read_bounded(path, INPUT_BYTES_LIMIT, budget)?;
        let report_id = blake3::hash(&bytes).to_hex().to_string();
        let rows = match input {
            MutationInput::Pit(_) => parse_pit(&bytes, budget)?,
            MutationInput::Stryker(_) => parse_stryker(&bytes, budget)?,
        };
        for mut row in rows {
            row.report_ids = vec![report_id.clone()];
            resolve_row(&mut row, entries, analysis);
            let key = semantic_key(&row);
            match merged.get_mut(&key) {
                Some(existing) => {
                    existing.report_ids.extend(row.report_ids);
                    existing.report_ids.sort();
                    existing.report_ids.dedup();
                }
                None => {
                    merged.insert(key, row);
                }
            }
        }
    }

    let mut mutants: Vec<MutationRow> = merged.into_values().collect();
    mutants.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.start_line.cmp(&right.start_line))
            .then(left.start_column.cmp(&right.start_column))
            .then(left.provider.cmp(right.provider))
            .then(left.operator.cmp(&right.operator))
            .then(left.status.cmp(&right.status))
            .then(left.native_id.cmp(&right.native_id))
            .then(left.description.cmp(&right.description))
    });

    let summary = summarize(&mutants);
    Ok(MutationReport {
        schema_version: MUTATION_SCHEMA_VERSION,
        metric_profile: crate::core::METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        model: MUTATION_MODEL,
        summary,
        mutants,
    })
}

fn semantic_key(row: &MutationRow) -> String {
    let mut copy = row.clone();
    copy.report_ids.clear();
    serde_json::to_string(&copy).unwrap_or_default()
}

fn summarize(mutants: &[MutationRow]) -> MutationSummary {
    let mut summary = MutationSummary {
        total: mutants.len() as u64,
        killed: 0,
        timed_out: 0,
        survived: 0,
        no_coverage: 0,
        ignored: 0,
        compile_error: 0,
        error: 0,
        unknown: 0,
        unresolved: 0,
        score: None,
        scored_mutants: 0,
    };
    for row in mutants {
        match row.status {
            MutationStatus::Killed => summary.killed += 1,
            MutationStatus::TimedOut => summary.timed_out += 1,
            MutationStatus::Survived => summary.survived += 1,
            MutationStatus::NoCoverage => summary.no_coverage += 1,
            MutationStatus::Ignored => summary.ignored += 1,
            MutationStatus::CompileError => summary.compile_error += 1,
            MutationStatus::Error => summary.error += 1,
            MutationStatus::Unknown => summary.unknown += 1,
        }
        if row.path.is_none() {
            summary.unresolved += 1;
        }
    }
    let scored = summary.killed + summary.timed_out + summary.survived + summary.no_coverage;
    summary.scored_mutants = scored;
    if scored > 0 {
        summary.score = Some((summary.killed + summary.timed_out) as f64 / scored as f64 * 100.0);
    }
    summary
}

fn resolve_row(row: &mut MutationRow, entries: &[SourceEntry], analysis: &AnalysisReport) {
    let Some(raw_path) = row.path.as_deref() else {
        row.reason
            .get_or_insert_with(|| "missing source path".to_owned());
        return;
    };
    let candidate = strict_relative_path(raw_path)
        .ok()
        .and_then(|normalized| resolve_path(entries, analysis, &normalized));
    let Some(path) = candidate else {
        row.path = None;
        row.reason = Some("source path does not match any analyzed file".to_owned());
        return;
    };
    row.path = Some(path.clone());
    let Some(file) = analysis.files.iter().find(|file| file.path == path) else {
        row.reason = Some("source path does not match any analyzed file".to_owned());
        return;
    };
    let start = row.start_line.unwrap_or(1);
    let end = row.end_line.unwrap_or(start);
    match innermost_function(file, start, end) {
        FunctionMatch::Unique(id) => {
            row.function_id = Some(id);
            row.reason = None;
        }
        FunctionMatch::None => row.reason = Some("no containing function".to_owned()),
        FunctionMatch::Ambiguous => {
            row.reason = Some("ambiguous containing function".to_owned());
        }
    }
}

fn resolve_path(
    entries: &[SourceEntry],
    analysis: &AnalysisReport,
    normalized: &str,
) -> Option<String> {
    if analysis.files.iter().any(|file| file.path == normalized) {
        return Some(normalized.to_owned());
    }
    let base = normalized.rsplit('/').next().unwrap_or(normalized);
    let mut matches = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .filter(|path| path.rsplit('/').next() == Some(base));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first.to_owned())
}

enum FunctionMatch {
    Unique(String),
    None,
    Ambiguous,
}

fn innermost_function(file: &crate::core::FileAnalysis, start: u32, end: u32) -> FunctionMatch {
    let mut best: Option<&crate::core::FunctionAnalysis> = None;
    let mut ambiguous = false;
    for function in &file.functions {
        if function.start_line <= start && function.end_line >= end {
            match best {
                None => best = Some(function),
                Some(current) => {
                    let current_span = current.end_line - current.start_line;
                    let candidate_span = function.end_line - function.start_line;
                    if candidate_span < current_span {
                        best = Some(function);
                        ambiguous = false;
                    } else if candidate_span == current_span {
                        ambiguous = true;
                    }
                }
            }
        }
    }
    match best {
        Some(function) if !ambiguous => FunctionMatch::Unique(function.id.clone()),
        Some(_) => FunctionMatch::Ambiguous,
        None => FunctionMatch::None,
    }
}

fn parse_pit(bytes: &[u8], budget: &mut InputBudget) -> Result<Vec<MutationRow>> {
    validate_xml_preamble_and_depth(bytes)?;
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut rows = Vec::new();
    let mut current: Option<BTreeMap<String, String>> = None;
    let mut status = String::new();
    let mut detected = false;
    let mut text_target: Option<&'static str> = None;
    let mut buffer = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                buffer.clear();
                match event.name().as_ref() {
                    "mutation" => {
                        current = Some(BTreeMap::new());
                        status.clear();
                        detected = false;
                        for attribute in event.attributes().flatten() {
                            let value = attribute.value.into_owned();
                            match attribute.key.as_ref() {
                                "status" => status = value,
                                "detected" => detected = value == "true",
                                _ => {}
                            }
                        }
                    }
                    "sourceFile" => text_target = Some("sourceFile"),
                    "lineNumber" => text_target = Some("lineNumber"),
                    "mutator" => text_target = Some("mutator"),
                    "description" => text_target = Some("description"),
                    "mutatedClass" => text_target = Some("mutatedClass"),
                    "mutatedMethod" => text_target = Some("mutatedMethod"),
                    "methodDescription" => text_target = Some("methodDescription"),
                    "index" => text_target = Some("index"),
                    "block" => text_target = Some("block"),
                    _ => {}
                }
            }
            Ok(Event::Text(event)) => {
                if let Some(target) = text_target.take() {
                    let text = quick_xml::escape::unescape(event.into_inner().as_ref())
                        .map_err(|error| format!("PIT report: {error}"))?
                        .into_owned();
                    if let Some(fields) = current.as_mut() {
                        fields.insert(target.to_owned(), text);
                    }
                }
            }
            Ok(Event::End(event)) => {
                if event.name().as_ref() == "mutation" {
                    budget.consume_rows(1)?;
                    let fields = current.take().unwrap_or_default();
                    let raw_status = status.clone();
                    let status = if raw_status.is_empty() {
                        MutationStatus::from_killed(detected)
                    } else {
                        MutationStatus::parse(&raw_status)
                    };
                    rows.push(MutationRow {
                        provider: "pit",
                        report_ids: Vec::new(),
                        native_id: fields.get("index").map(|index| format!("index-{index}")),
                        status,
                        raw_status,
                        operator: fields.get("mutator").cloned().unwrap_or_default(),
                        description: fields.get("description").cloned(),
                        path: fields.get("sourceFile").cloned(),
                        start_line: fields.get("lineNumber").and_then(|line| line.parse().ok()),
                        end_line: fields
                            .get("lineNumber")
                            .and_then(|line| line.parse::<u32>().ok())
                            .map(|line| line + 1),
                        start_column: Some(1),
                        end_column: Some(1),
                        function_id: None,
                        reason: None,
                    });
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("PIT report: {error}").into()),
        }
    }
    Ok(rows)
}

fn parse_stryker(bytes: &[u8], budget: &mut InputBudget) -> Result<Vec<MutationRow>> {
    validate_json_depth(bytes)?;
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("Stryker report: {error}"))?;
    let schema = match value.get("schemaVersion") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Number(number)) => number.to_string(),
        _ => return Err("Stryker report: missing schemaVersion".into()),
    };
    let major = schema.split('.').next().unwrap_or(&schema);
    if major != "1" {
        return Err(format!("Stryker report: unsupported schemaVersion {schema:?}").into());
    }
    let files = value
        .get("files")
        .and_then(|files| files.as_object())
        .ok_or("Stryker report: missing files object")?;
    let mut rows = Vec::new();
    for (path, file) in files {
        let mutants = file
            .get("mutants")
            .and_then(|mutants| mutants.as_array())
            .ok_or("Stryker report: file is missing mutants")?;
        for mutant in mutants {
            budget.consume_rows(1)?;
            let raw_status = mutant
                .get("status")
                .and_then(|status| status.as_str())
                .unwrap_or("Unknown")
                .to_owned();
            let location = mutant.get("location");
            let coordinate = |field: &str, axis: &str| {
                location
                    .and_then(|location| location.get(field))
                    .and_then(|point| point.get(axis))
                    .and_then(|value| value.as_u64())
                    .map(|value| value as u32)
            };
            let start_line = coordinate("start", "line");
            let start_column = coordinate("start", "column").map(|column| column + 1);
            let end_line = coordinate("end", "line");
            let end_column = coordinate("end", "column").map(|column| column + 1);
            rows.push(MutationRow {
                provider: "stryker",
                report_ids: Vec::new(),
                native_id: mutant
                    .get("id")
                    .and_then(|id| id.as_str())
                    .map(str::to_owned),
                status: MutationStatus::parse(&raw_status),
                raw_status,
                operator: mutant
                    .get("mutatorName")
                    .and_then(|operator| operator.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                description: mutant
                    .get("description")
                    .and_then(|description| description.as_str())
                    .map(str::to_owned)
                    .or_else(|| {
                        mutant
                            .get("replacement")
                            .and_then(|replacement| replacement.as_str())
                            .map(str::to_owned)
                    }),
                path: Some(path.clone()),
                start_line,
                end_line,
                start_column,
                end_column,
                function_id: None,
                reason: None,
            });
        }
    }
    Ok(rows)
}
