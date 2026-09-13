//! Token-level clone detection (`tokens-v1`) and drift.
//!
//! Identifier, string, and number leaves normalize to categories; keywords and
//! punctuation stay exact. Languages are separate partitions. The detector is
//! bounded: it never exceeds the token or exact-comparison ceilings and marks
//! the report incomplete instead of dropping candidates silently.

use crate::config::DuplicationConfig;
use crate::core::Language;
use crate::parser::normalized_tokens;
use crate::source_snapshot::SourceEntry;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const DUPLICATION_SCHEMA_VERSION: u32 = 1;
pub const DUPLICATION_PROFILE: &str = "tokens-v1";
pub const DUPLICATION_DRIFT_MODEL: &str = "duplication-drift-v1";
pub const TOKEN_CEILING: usize = 10_000_000;
pub const COMPARISON_CEILING: usize = 10_000_000;

const CONTEXT_TOKENS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceStatus {
    Added,
    Existing,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupStatus {
    New,
    Existing,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CloneOccurrence {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub token_count: usize,
    pub duplicated_lines: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<OccurrenceStatus>,
    /// Stable drift identity; never serialized.
    #[serde(skip)]
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CloneGroup {
    pub id: String,
    pub language: Language,
    pub token_count: usize,
    pub occurrences: Vec<CloneOccurrence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<GroupStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DuplicationDiagnostic {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DuplicationReport {
    pub schema_version: u32,
    pub analyzer_version: &'static str,
    pub profile: &'static str,
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub total_tokens: usize,
    pub comparisons: usize,
    pub duplicated_lines: u64,
    pub groups: Vec<CloneGroup>,
    pub diagnostics: Vec<DuplicationDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DuplicationDriftReport {
    pub schema_version: u32,
    pub analyzer_version: &'static str,
    pub model: &'static str,
    pub complete: bool,
    pub added_occurrences: u64,
    pub removed_occurrences: u64,
    pub groups: Vec<CloneGroup>,
}

struct TokenFile {
    path: String,
    language: Language,
    texts: Vec<String>,
    lines: Vec<u32>,
    ids: Vec<u32>,
}

/// Detects clones across the analyzed sources.
pub fn detect(entries: &[SourceEntry], config: &DuplicationConfig) -> DuplicationReport {
    detect_with_limits(entries, config, TOKEN_CEILING, COMPARISON_CEILING)
}

/// Test seam: exercises ceilings without building multi-million-token corpora.
pub fn detect_with_limits(
    entries: &[SourceEntry],
    config: &DuplicationConfig,
    token_ceiling: usize,
    comparison_ceiling: usize,
) -> DuplicationReport {
    let excluded = ExcludeMatcher::new(&config.excludes);
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    let mut total_tokens = 0usize;
    for entry in entries {
        if excluded.matches(&entry.path) {
            continue;
        }
        let tokenized = match normalized_tokens(&entry.path, &entry.bytes) {
            Ok(tokenized) => tokenized,
            Err(_) => continue,
        };
        if tokenized.parse_errors > 0 {
            diagnostics.push(DuplicationDiagnostic {
                path: entry.path.clone(),
                reason: "parse errors exclude the file from clone candidates".to_owned(),
            });
            continue;
        }
        total_tokens += tokenized.tokens.len();
        files.push((entry.path.clone(), tokenized));
    }

    let mut report = DuplicationReport {
        schema_version: DUPLICATION_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        profile: DUPLICATION_PROFILE,
        complete: true,
        reason: None,
        total_tokens,
        comparisons: 0,
        duplicated_lines: 0,
        groups: Vec::new(),
        diagnostics,
    };
    if total_tokens > token_ceiling {
        report.complete = false;
        report.reason = Some("token_ceiling_exceeded".to_owned());
        return report;
    }

    let mut token_ids: BTreeMap<String, u32> = BTreeMap::new();
    let mut tokens = Vec::new();
    for (path, tokenized) in files {
        let mut ids = Vec::with_capacity(tokenized.tokens.len());
        for token in &tokenized.tokens {
            let next = token_ids.len() as u32;
            ids.push(*token_ids.entry(token.text.clone()).or_insert(next));
        }
        tokens.push(TokenFile {
            path,
            language: tokenized.language,
            texts: tokenized.tokens.iter().map(|t| t.text.clone()).collect(),
            lines: tokenized.tokens.iter().map(|t| t.line).collect(),
            ids,
        });
    }

    let min_tokens = config.min_tokens;
    let mut buckets: BTreeMap<u64, Vec<(usize, usize)>> = BTreeMap::new();
    for (file_index, file) in tokens.iter().enumerate() {
        if file.ids.len() < min_tokens {
            continue;
        }
        let mut hash = 0u64;
        let power = power_base(min_tokens);
        for id in &file.ids[..min_tokens] {
            hash = hash.wrapping_mul(BASE).wrapping_add(u64::from(*id));
        }
        buckets.entry(hash).or_default().push((file_index, 0));
        for start in 1..=file.ids.len() - min_tokens {
            let outgoing = u64::from(file.ids[start - 1]);
            hash = hash
                .wrapping_sub(outgoing.wrapping_mul(power))
                .wrapping_mul(BASE)
                .wrapping_add(u64::from(file.ids[start + min_tokens - 1]));
            buckets.entry(hash).or_default().push((file_index, start));
        }
    }

    let mut groups: BTreeMap<String, GroupBuilder> = BTreeMap::new();
    let mut comparisons = 0usize;
    let mut incomplete_reason = None;
    'buckets: for candidates in buckets.values() {
        if candidates.len() < 2 {
            continue;
        }
        for left in 0..candidates.len() {
            for right in left + 1..candidates.len() {
                let (left_file, left_start) = candidates[left];
                let (right_file, right_start) = candidates[right];
                if tokens[left_file].language != tokens[right_file].language {
                    continue;
                }
                comparisons += 1;
                if comparisons > comparison_ceiling {
                    incomplete_reason = Some("comparison_ceiling_exceeded".to_owned());
                    break 'buckets;
                }
                let mut start_left = left_start;
                let mut start_right = right_start;
                let mut length = min_tokens;
                while start_left > 0
                    && start_right > 0
                    && tokens[left_file].ids[start_left - 1]
                        == tokens[right_file].ids[start_right - 1]
                {
                    start_left -= 1;
                    start_right -= 1;
                    length += 1;
                }
                while start_left + length < tokens[left_file].ids.len()
                    && start_right + length < tokens[right_file].ids.len()
                    && tokens[left_file].ids[start_left + length]
                        == tokens[right_file].ids[start_right + length]
                {
                    length += 1;
                }
                let language = tokens[left_file].language;
                let texts = &tokens[left_file].texts[start_left..start_left + length];
                let key = format!("{language:?}\u{1}{}", texts.join("\u{1}"));
                let builder = groups.entry(key).or_insert_with(|| GroupBuilder {
                    id: String::new(),
                    language,
                    texts: texts.to_vec(),
                    occurrences: Vec::new(),
                });
                insert_occurrence(builder, left_file, start_left, length, min_tokens);
                insert_occurrence(builder, right_file, start_right, length, min_tokens);
            }
        }
    }

    let mut groups: Vec<CloneGroup> = groups
        .into_values()
        .filter_map(|mut builder| builder.finish(&tokens, config.min_lines as u32))
        .collect();
    groups.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.occurrences[0].path.cmp(&right.occurrences[0].path))
    });
    let duplicated_lines = union_line_total(&groups);
    report.comparisons = comparisons;
    report.duplicated_lines = duplicated_lines;
    report.groups = groups;
    if let Some(reason) = incomplete_reason {
        report.complete = false;
        report.reason = Some(reason);
        report.groups.clear();
        report.duplicated_lines = 0;
    }
    report
}

const BASE: u64 = 1_000_003;

fn power_base(exponent: usize) -> u64 {
    let mut value = 1u64;
    for _ in 0..exponent {
        value = value.wrapping_mul(BASE);
    }
    value
}

fn insert_occurrence(
    builder: &mut GroupBuilder,
    file: usize,
    start: usize,
    length: usize,
    min_tokens: usize,
) {
    if length < min_tokens {
        return;
    }
    let end = start + length - 1;
    for existing in &builder.occurrences {
        if existing.file == file
            && start < existing.start + existing.length
            && existing.start < end + 1
        {
            return;
        }
    }
    builder.occurrences.push(OccurrenceBuilder {
        file,
        start,
        length,
    });
}

struct GroupBuilder {
    id: String,
    language: Language,
    texts: Vec<String>,
    occurrences: Vec<OccurrenceBuilder>,
}

#[derive(Clone)]
struct OccurrenceBuilder {
    file: usize,
    start: usize,
    length: usize,
}

impl GroupBuilder {
    fn finish(&mut self, tokens: &[TokenFile], min_lines: u32) -> Option<CloneGroup> {
        let language = self.language;
        let id_source = format!(
            "{DUPLICATION_PROFILE}\u{1}{language:?}\u{1}{}",
            self.texts.join("\u{1}")
        );
        self.id = blake3::hash(id_source.as_bytes()).to_hex().to_string();
        let mut ordered = self.occurrences.clone();
        ordered.sort_by_key(|occurrence| (occurrence.file, occurrence.start));
        let mut per_file_end: BTreeMap<usize, usize> = BTreeMap::new();
        let mut context_ordinals: BTreeMap<(usize, String), u64> = BTreeMap::new();
        let mut occurrences = Vec::new();
        for occurrence in ordered {
            if let Some(end) = per_file_end.get(&occurrence.file)
                && occurrence.start < *end
            {
                continue;
            }
            let file = &tokens[occurrence.file];
            let start = occurrence.start;
            let end = start + occurrence.length - 1;
            let start_line = file.lines[start];
            let end_line = file.lines[end];
            let duplicated_lines = end_line.saturating_sub(start_line) + 1;
            if duplicated_lines < min_lines {
                continue;
            }
            per_file_end.insert(occurrence.file, end + 1);
            let context = occurrence_context(&self.id, file, start, occurrence.length);
            let ordinal = context_ordinals
                .entry((occurrence.file, context.clone()))
                .or_default();
            let fingerprint = blake3::hash(format!("{context}\u{1}{ordinal}").as_bytes())
                .to_hex()
                .to_string();
            *ordinal += 1;
            occurrences.push(CloneOccurrence {
                path: file.path.clone(),
                start_line,
                end_line,
                token_count: occurrence.length,
                duplicated_lines,
                status: None,
                fingerprint,
            });
        }
        if occurrences.len() < 2 {
            return None;
        }
        Some(CloneGroup {
            id: self.id.clone(),
            language,
            token_count: self.texts.len(),
            occurrences,
            status: None,
        })
    }
}

fn occurrence_context(group_id: &str, file: &TokenFile, start: usize, length: usize) -> String {
    let left_start = start.saturating_sub(CONTEXT_TOKENS);
    let left = file.texts[left_start..start].join("\u{2}");
    let right_start = start + length;
    let right_end = (right_start + CONTEXT_TOKENS).min(file.texts.len());
    let right = file.texts[right_start..right_end].join("\u{2}");
    format!("{group_id}\u{1}{left}\u{1}{right}")
}

fn union_line_total(groups: &[CloneGroup]) -> u64 {
    let mut per_file: BTreeMap<&str, Vec<(u32, u32)>> = BTreeMap::new();
    for group in groups {
        for occurrence in &group.occurrences {
            per_file
                .entry(occurrence.path.as_str())
                .or_default()
                .push((occurrence.start_line, occurrence.end_line));
        }
    }
    let mut total = 0u64;
    for intervals in per_file.values_mut() {
        intervals.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for (start, end) in intervals.iter().copied() {
            match merged.last_mut() {
                Some((_, last_end)) if start <= *last_end => {
                    *last_end = (*last_end).max(end);
                }
                _ => merged.push((start, end)),
            }
        }
        for (start, end) in merged {
            total += u64::from(end.saturating_sub(start) + 1);
        }
    }
    total
}

/// Compares clone occurrences across two reports with rename mapping.
pub fn compare(
    before: &DuplicationReport,
    after: &DuplicationReport,
    renames: &BTreeMap<String, String>,
) -> DuplicationDriftReport {
    let before_by_group: BTreeMap<&str, Vec<(&CloneOccurrence, String)>> = before
        .groups
        .iter()
        .map(|group| {
            let rows = group
                .occurrences
                .iter()
                .map(|occurrence| {
                    let path = renames
                        .get(&occurrence.path)
                        .cloned()
                        .unwrap_or_else(|| occurrence.path.clone());
                    (occurrence, path)
                })
                .collect();
            (group.id.as_str(), rows)
        })
        .collect();
    let mut groups = Vec::new();
    let mut added_occurrences = 0u64;
    let mut removed_occurrences = 0u64;
    for group in &after.groups {
        let mut occurrences = Vec::new();
        let base_rows = before_by_group.get(group.id.as_str());
        let mut base_seen: BTreeSet<&str> = BTreeSet::new();
        for occurrence in &group.occurrences {
            let existing = base_rows.is_some_and(|rows| {
                rows.iter().any(|(row, mapped)| {
                    mapped == &occurrence.path && row.fingerprint == occurrence.fingerprint
                })
            });
            if existing {
                base_seen.insert(occurrence.fingerprint.as_str());
                added_occurrences += u64::from(false);
                occurrences.push(CloneOccurrence {
                    status: Some(OccurrenceStatus::Existing),
                    ..occurrence.clone()
                });
            } else {
                added_occurrences += 1;
                occurrences.push(CloneOccurrence {
                    status: Some(OccurrenceStatus::Added),
                    ..occurrence.clone()
                });
            }
        }
        let mut removed = Vec::new();
        if let Some(rows) = base_rows {
            for (row, mapped) in rows {
                if !base_seen.contains(row.fingerprint.as_str()) {
                    removed_occurrences += 1;
                    removed.push(CloneOccurrence {
                        path: mapped.clone(),
                        status: Some(OccurrenceStatus::Removed),
                        ..(**row).clone()
                    });
                }
            }
        }
        occurrences.extend(removed);
        let status = if base_rows.is_none() {
            GroupStatus::New
        } else {
            GroupStatus::Existing
        };
        groups.push(CloneGroup {
            id: group.id.clone(),
            language: group.language,
            token_count: group.token_count,
            occurrences,
            status: Some(status),
        });
    }
    for group in &before.groups {
        if !after.groups.iter().any(|row| row.id == group.id) {
            let mut occurrences = Vec::new();
            for occurrence in &group.occurrences {
                removed_occurrences += 1;
                let path = renames
                    .get(&occurrence.path)
                    .cloned()
                    .unwrap_or_else(|| occurrence.path.clone());
                occurrences.push(CloneOccurrence {
                    path,
                    status: Some(OccurrenceStatus::Removed),
                    ..occurrence.clone()
                });
            }
            groups.push(CloneGroup {
                id: group.id.clone(),
                language: group.language,
                token_count: group.token_count,
                occurrences,
                status: Some(GroupStatus::Resolved),
            });
        }
    }
    groups.sort_by(|left, right| left.id.cmp(&right.id));
    DuplicationDriftReport {
        schema_version: DUPLICATION_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        model: DUPLICATION_DRIFT_MODEL,
        complete: before.complete && after.complete,
        added_occurrences,
        removed_occurrences,
        groups,
    }
}

struct ExcludeMatcher {
    matcher: ignore::gitignore::Gitignore,
}

impl ExcludeMatcher {
    fn new(patterns: &[String]) -> Self {
        let mut builder = ignore::gitignore::GitignoreBuilder::new("");
        for pattern in patterns {
            let _ = builder.add_line(None, pattern);
        }
        Self {
            matcher: builder.build().unwrap_or_else(|_| {
                ignore::gitignore::GitignoreBuilder::new("")
                    .build()
                    .expect("empty matcher")
            }),
        }
    }

    fn matches(&self, path: &str) -> bool {
        self.matcher
            .matched_path_or_any_parents(std::path::Path::new(path), false)
            .is_ignore()
    }
}
