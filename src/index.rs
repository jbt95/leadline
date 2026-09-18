//! Persistent analysis index.
//!
//! File metrics are keyed by a versioned content key; Git facts are keyed by
//! HEAD. A warm run re-parses only files whose content key changed. Coverage
//! merges after analysis, so any coverage input bypasses the index. Files with
//! parse errors are never stored.

use crate::core::{
    FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE, MetricContribution,
    OUTPUT_SCHEMA_VERSION,
};
use crate::history::FileHistory;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::path::Path;

pub const INDEX_SCHEMA_VERSION: u32 = 1;
pub const INDEX_FILE_NAME: &str = "index.json";
pub const DEFAULT_INDEX_DIR: &str = ".leadline";

/// Parser backend versions baked into the index key.
/// Bump when tree-sitter or any grammar crate changes.
pub const PARSER_VERSIONS: &str = "tree-sitter-0.27:java-0.23.5:javascript-0.25:typescript-0.23.2";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IndexedFile {
    /// Byte length of the indexed content.
    pub size: u64,
    /// Versioned content key; see [`content_key`].
    pub key: String,
    pub functions: Vec<IndexedFunction>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HistoryFacts {
    pub head_commit: Option<String>,
    pub head_timestamp: Option<i64>,
    pub files: Vec<FileHistory>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnalysisIndex {
    pub schema_version: u32,
    pub analyzer_version: String,
    pub metric_profile: String,
    pub parser_versions: String,
    pub config_fingerprint: String,
    pub scope: String,
    pub files: BTreeMap<String, IndexedFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<HistoryFacts>,
}

impl AnalysisIndex {
    pub fn empty(scope: &str, config_fingerprint: &str) -> AnalysisIndex {
        AnalysisIndex {
            schema_version: INDEX_SCHEMA_VERSION,
            analyzer_version: env!("CARGO_PKG_VERSION").to_owned(),
            metric_profile: METRIC_PROFILE.to_owned(),
            parser_versions: PARSER_VERSIONS.to_owned(),
            config_fingerprint: config_fingerprint.to_owned(),
            scope: scope.to_owned(),
            files: BTreeMap::new(),
            history: None,
        }
    }

    /// Read `index.json` from `dir`. A missing, unreadable, corrupt, or
    /// schema-mismatched file yields an empty index; it is never an error.
    pub fn open(dir: &Path) -> AnalysisIndex {
        std::fs::read(dir.join(INDEX_FILE_NAME))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AnalysisIndex>(&bytes).ok())
            .filter(|index| index.schema_version == INDEX_SCHEMA_VERSION)
            .unwrap_or_else(|| AnalysisIndex::empty(".", "none"))
    }

    /// True when every version input still matches, so entries may be reused.
    pub fn is_usable(&self, scope: &str, config_fingerprint: &str) -> bool {
        file_reusable(self, config_fingerprint) && self.scope == scope
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let file = std::fs::File::create(dir.join(INDEX_FILE_NAME))?;
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer(&mut writer, self).map_err(std::io::Error::other)?;
        std::io::Write::flush(&mut writer)
    }
}

/// Versioned content key for one file.
pub fn content_key(content: &[u8]) -> String {
    content_key_for(env!("CARGO_PKG_VERSION"), content)
}

fn content_key_for(version: &str, content: &[u8]) -> String {
    let schema_version = OUTPUT_SCHEMA_VERSION.to_le_bytes();
    let mut hasher = blake3::Hasher::new();
    for part in [
        version.as_bytes(),
        METRIC_PROFILE.as_bytes(),
        schema_version.as_slice(),
        PARSER_VERSIONS.as_bytes(),
        content,
    ] {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().to_hex().to_string()
}

// Core analysis types only derive Serialize (source_fingerprint is skipped),
// so the index keeps its own serializable mirror and converts on load/save.

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IndexedFunction {
    name: String,
    id: String,
    kind: IndexedKind,
    start_line: u32,
    end_line: u32,
    start_byte: u64,
    end_byte: u64,
    metrics: IndexedMetrics,
    contributions: Vec<IndexedContribution>,
    source_fingerprint: u64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexedKind {
    Function,
    Method,
    Constructor,
    Lambda,
    Arrow,
    Anonymous,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IndexedContribution {
    rule: String,
    line: u32,
    nesting: u32,
    cognitive: u32,
    cyclomatic: u32,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IndexedMetrics {
    loc: u32,
    logical_loc: u32,
    function_length: u32,
    parameters: u32,
    max_nesting: u32,
    cyclomatic: u32,
    cognitive: u32,
    halstead_n1: u32,
    halstead_n2: u32,
    #[serde(rename = "halstead_N1")]
    halstead_total_operators: u32,
    #[serde(rename = "halstead_N2")]
    halstead_total_operands: u32,
    halstead_vocabulary: u32,
    halstead_length: u32,
    halstead_volume: f64,
    halstead_difficulty: f64,
    halstead_effort: f64,
    maintainability_index: f64,
    coverage: Option<f64>,
    crap: Option<f64>,
}

impl From<FunctionKind> for IndexedKind {
    fn from(kind: FunctionKind) -> IndexedKind {
        match kind {
            FunctionKind::Function => IndexedKind::Function,
            FunctionKind::Method => IndexedKind::Method,
            FunctionKind::Constructor => IndexedKind::Constructor,
            FunctionKind::Lambda => IndexedKind::Lambda,
            FunctionKind::Arrow => IndexedKind::Arrow,
            FunctionKind::Anonymous => IndexedKind::Anonymous,
        }
    }
}

impl From<IndexedKind> for FunctionKind {
    fn from(kind: IndexedKind) -> FunctionKind {
        match kind {
            IndexedKind::Function => FunctionKind::Function,
            IndexedKind::Method => FunctionKind::Method,
            IndexedKind::Constructor => FunctionKind::Constructor,
            IndexedKind::Lambda => FunctionKind::Lambda,
            IndexedKind::Arrow => FunctionKind::Arrow,
            IndexedKind::Anonymous => FunctionKind::Anonymous,
        }
    }
}

impl From<&MetricContribution> for IndexedContribution {
    fn from(contribution: &MetricContribution) -> IndexedContribution {
        IndexedContribution {
            rule: contribution.rule.clone(),
            line: contribution.line,
            nesting: contribution.nesting,
            cognitive: contribution.cognitive,
            cyclomatic: contribution.cyclomatic,
        }
    }
}

impl From<IndexedContribution> for MetricContribution {
    fn from(contribution: IndexedContribution) -> MetricContribution {
        MetricContribution {
            rule: contribution.rule,
            line: contribution.line,
            nesting: contribution.nesting,
            cognitive: contribution.cognitive,
            cyclomatic: contribution.cyclomatic,
        }
    }
}

impl From<&FunctionMetrics> for IndexedMetrics {
    fn from(metrics: &FunctionMetrics) -> IndexedMetrics {
        IndexedMetrics {
            loc: metrics.loc,
            logical_loc: metrics.logical_loc,
            function_length: metrics.function_length,
            parameters: metrics.parameters,
            max_nesting: metrics.max_nesting,
            cyclomatic: metrics.cyclomatic,
            cognitive: metrics.cognitive,
            halstead_n1: metrics.halstead_n1,
            halstead_n2: metrics.halstead_n2,
            halstead_total_operators: metrics.halstead_total_operators,
            halstead_total_operands: metrics.halstead_total_operands,
            halstead_vocabulary: metrics.halstead_vocabulary,
            halstead_length: metrics.halstead_length,
            halstead_volume: metrics.halstead_volume,
            halstead_difficulty: metrics.halstead_difficulty,
            halstead_effort: metrics.halstead_effort,
            maintainability_index: metrics.maintainability_index,
            coverage: metrics.coverage,
            crap: metrics.crap,
        }
    }
}

impl From<IndexedMetrics> for FunctionMetrics {
    fn from(metrics: IndexedMetrics) -> FunctionMetrics {
        FunctionMetrics {
            loc: metrics.loc,
            logical_loc: metrics.logical_loc,
            function_length: metrics.function_length,
            parameters: metrics.parameters,
            max_nesting: metrics.max_nesting,
            cyclomatic: metrics.cyclomatic,
            cognitive: metrics.cognitive,
            halstead_n1: metrics.halstead_n1,
            halstead_n2: metrics.halstead_n2,
            halstead_total_operators: metrics.halstead_total_operators,
            halstead_total_operands: metrics.halstead_total_operands,
            halstead_vocabulary: metrics.halstead_vocabulary,
            halstead_length: metrics.halstead_length,
            halstead_volume: metrics.halstead_volume,
            halstead_difficulty: metrics.halstead_difficulty,
            halstead_effort: metrics.halstead_effort,
            maintainability_index: metrics.maintainability_index,
            coverage: metrics.coverage,
            crap: metrics.crap,
        }
    }
}

impl From<&FunctionAnalysis> for IndexedFunction {
    fn from(function: &FunctionAnalysis) -> IndexedFunction {
        IndexedFunction {
            name: function.name.clone(),
            id: function.id.clone(),
            kind: IndexedKind::from(function.kind),
            start_line: function.start_line,
            end_line: function.end_line,
            start_byte: function.start_byte,
            end_byte: function.end_byte,
            metrics: IndexedMetrics::from(&function.metrics),
            contributions: function
                .contributions
                .iter()
                .map(IndexedContribution::from)
                .collect(),
            source_fingerprint: function.source_fingerprint,
        }
    }
}

impl From<IndexedFunction> for FunctionAnalysis {
    fn from(function: IndexedFunction) -> FunctionAnalysis {
        FunctionAnalysis {
            name: function.name,
            id: function.id,
            kind: FunctionKind::from(function.kind),
            start_line: function.start_line,
            end_line: function.end_line,
            start_byte: function.start_byte,
            end_byte: function.end_byte,
            metrics: FunctionMetrics::from(function.metrics),
            contributions: function
                .contributions
                .into_iter()
                .map(MetricContribution::from)
                .collect(),
            source_fingerprint: function.source_fingerprint,
        }
    }
}

/// Counts of files analyzed and files reused in one refresh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reuse {
    pub analyzed: usize,
    pub reused: usize,
}

/// Analysis-root label used to keep one index from mixing scopes.
pub fn scope_label(path: &Path) -> String {
    let label = crate::normalize_path(path);
    if label.is_empty() {
        ".".to_owned()
    } else {
        label
    }
}

/// Read every discovered source file under `root` into an owned entry.
pub fn load_entries(
    root: &Path,
    excludes: &[String],
) -> crate::Result<Vec<crate::source_snapshot::SourceEntry>> {
    let discovered = crate::discovery::discover_with_excludes(root, excludes)?;
    let mut entries = Vec::with_capacity(discovered.len());
    for file in discovered {
        let bytes = std::fs::read(&file)?;
        entries.push(crate::source_snapshot::SourceEntry {
            path: crate::normalized_relative_path(&file, root),
            bytes,
        });
    }
    Ok(entries)
}

/// Rebuild the report and index for `entries`, reusing unchanged files.
///
/// Files are processed in path order so the report and the index are
/// deterministic. Only files without parse errors are stored.
pub fn refresh_files(
    previous: Option<&AnalysisIndex>,
    scope: &str,
    config_fingerprint: &str,
    entries: &[crate::source_snapshot::SourceEntry],
) -> crate::Result<(crate::core::AnalysisReport, AnalysisIndex, Reuse)> {
    let reusable = previous.filter(|index| index.is_usable(scope, config_fingerprint));
    let mut ordered: Vec<&crate::source_snapshot::SourceEntry> = entries.iter().collect();
    ordered.sort_by(|left, right| left.path.cmp(&right.path));

    // Per-entry decisions are independent, so misses are parsed in parallel;
    // `collect` preserves the path-sorted order and the serial pass below
    // assembles `files` and the index exactly as the serial version did.
    enum Outcome {
        Reused(IndexedFile, crate::core::FileAnalysis),
        Analyzed(String, crate::Result<crate::core::FileAnalysis>),
    }
    let outcomes: Vec<Outcome> = ordered
        .par_iter()
        .map(|entry| {
            let key = content_key(&entry.bytes);
            let hit = reusable
                .and_then(|index| index.files.get(&entry.path))
                .filter(|file| file.key == key)
                .and_then(|file| rebuild(entry, file).map(|analysis| (file, analysis)));
            match hit {
                Some((file, analysis)) => Outcome::Reused(file.clone(), analysis),
                None => Outcome::Analyzed(key, crate::analyze_source(&entry.path, &entry.bytes)),
            }
        })
        .collect();

    let mut index = AnalysisIndex::empty(scope, config_fingerprint);
    // Warm runs must not evict stored Git facts: `leadline index` reuses
    // history when HEAD is unchanged, so dropping it here would force a
    // full Git walk on the next index build.
    index.history = reusable.and_then(|previous| previous.history.clone());
    let mut files = Vec::with_capacity(ordered.len());
    let mut reuse = Reuse::default();
    for (entry, outcome) in ordered.iter().zip(outcomes) {
        match outcome {
            Outcome::Reused(file, analysis) => {
                reuse.reused += 1;
                index.files.insert(entry.path.clone(), file);
                files.push(analysis);
            }
            Outcome::Analyzed(key, analysis) => {
                let analysis = analysis?;
                reuse.analyzed += 1;
                if analysis.parse_errors.is_empty() {
                    index.files.insert(
                        entry.path.clone(),
                        IndexedFile {
                            size: entry.bytes.len() as u64,
                            key,
                            functions: analysis
                                .functions
                                .iter()
                                .map(IndexedFunction::from)
                                .collect(),
                        },
                    );
                }
                files.push(analysis);
            }
        }
    }
    Ok((crate::analysis_report(files), index, reuse))
}

fn rebuild(
    entry: &crate::source_snapshot::SourceEntry,
    file: &IndexedFile,
) -> Option<crate::core::FileAnalysis> {
    let language = crate::parser::detect_language(&entry.path)?;
    Some(crate::core::FileAnalysis {
        path: entry.path.clone(),
        language,
        functions: file
            .functions
            .iter()
            .cloned()
            .map(crate::core::FunctionAnalysis::from)
            .collect(),
        parse_errors: Vec::new(),
    })
}

/// Git facts for `scope`, reused when the stored HEAD is unchanged.
///
/// Returns `None` outside a repository or when Git fails; the index is never
/// an error path, so a missing history section only means `git_available:
/// false` for the commands that consume it.
pub fn refresh_history(previous: Option<&AnalysisIndex>, scope: &Path) -> Option<HistoryFacts> {
    let (head, _) = crate::history::scope_head(scope).ok().flatten()?;
    if let Some(facts) = previous.and_then(|index| index.history.as_ref())
        && facts.head_commit == head
        && !facts.files.is_empty()
    {
        return Some(facts.clone());
    }
    let report = crate::history::analyze_history(scope).ok()?;
    Some(HistoryFacts {
        head_commit: report.head_commit,
        head_timestamp: report.head_timestamp,
        files: report.files,
    })
}

/// Everything one index build needs.
pub struct IndexRequest<'a> {
    pub root: &'a Path,
    pub index_dir: &'a Path,
    pub scope: &'a str,
    pub config_fingerprint: &'a str,
    pub excludes: &'a [String],
    pub verify: bool,
}

pub struct IndexOutcome {
    pub index: AnalysisIndex,
    pub reuse: Reuse,
    /// `Some(true)` when `--verify` re-derived an index value-identical to the
    /// stored one; `Some(false)` when it did not or none was stored.
    pub verified: Option<bool>,
    pub path: std::path::PathBuf,
}

/// Build or refresh the index, then write it.
pub fn build(request: &IndexRequest<'_>) -> crate::Result<IndexOutcome> {
    let stored = std::fs::read(request.index_dir.join(INDEX_FILE_NAME)).ok();
    let previous = if request.verify {
        None
    } else {
        Some(AnalysisIndex::open(request.index_dir))
    };
    let entries = load_entries(request.root, request.excludes)?;
    let (_, mut index, reuse) = refresh_files(
        previous.as_ref(),
        request.scope,
        request.config_fingerprint,
        &entries,
    )?;
    let usable = previous
        .as_ref()
        .filter(|index| index.is_usable(request.scope, request.config_fingerprint));
    index.history = refresh_history(usable, request.root);
    let verified = request.verify.then(|| {
        stored
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<AnalysisIndex>(bytes).ok())
            .is_some_and(|previous| previous == index)
    });
    if verified != Some(false) {
        index.save(request.index_dir)?;
    }
    Ok(IndexOutcome {
        path: request.index_dir.join(INDEX_FILE_NAME),
        index,
        reuse,
        verified,
    })
}

/// Version and configuration match ignoring scope, so a stored entry may be
/// reused for a file target even when the index was built for a directory.
fn file_reusable(previous: &AnalysisIndex, config_fingerprint: &str) -> bool {
    previous.schema_version == INDEX_SCHEMA_VERSION
        && previous.analyzer_version == env!("CARGO_PKG_VERSION")
        && previous.metric_profile == METRIC_PROFILE
        && previous.parser_versions == PARSER_VERSIONS
        && previous.config_fingerprint == config_fingerprint
}

/// Storage key for `display` (the cold `normalize_path` of the file) inside
/// `previous`: the display key itself for file-specific indexes, else the
/// repository-relative suffix for indexes built by `leadline index`.
fn resolve_file_key(display: &str, previous: &AnalysisIndex) -> String {
    if previous.files.contains_key(display) {
        return display.to_owned();
    }
    // Normalized paths never start with `./`, so the scope-"." prefix
    // match fails on its own without a special case.
    if let Some(relative) = display.strip_prefix(&format!("{}/", previous.scope)) {
        return relative.to_owned();
    }
    // Display keys are already ruled out above, so only `/`-separated
    // suffixes remain; the longest one is the most specific match.
    previous
        .files
        .keys()
        .filter(|key| display.ends_with(&format!("/{key}")))
        .max_by_key(|key| key.len())
        .cloned()
        .unwrap_or_else(|| display.to_owned())
}

/// Point `analysis` at the cold display path, re-deriving function ids so a
/// repository-relative entry reports exactly what a cold file analysis would.
fn retarget_to_display(analysis: &mut crate::core::FileAnalysis, display: &str) {
    analysis.path = display.to_owned();
    for function in &mut analysis.functions {
        function.id = crate::core::function_id(
            display,
            function.kind,
            function.start_byte,
            function.end_byte,
        );
    }
}

/// Warm a single file against `previous`, reusing a repository index built by
/// `leadline index` as well as file-specific indexes.
///
/// A usable index stores the file under its repository key, so the refresh
/// goes through [`refresh_files`] with that one entry: the stored metrics are
/// reused when the content key matches, and the refreshed entry is merged back
/// so every other file and the stored history survive. The report is retargeted
/// to the cold display path. Without a usable index the file is analyzed cold
/// into a fresh, file-scoped index.
pub fn refresh_file_index(
    file: &Path,
    previous: &AnalysisIndex,
    config_fingerprint: &str,
) -> crate::Result<(crate::core::AnalysisReport, AnalysisIndex, Reuse)> {
    let display = crate::normalize_path(file);
    let bytes = std::fs::read(file)?;
    let usable = file_reusable(previous, config_fingerprint);
    let key = if usable {
        resolve_file_key(&display, previous)
    } else {
        display.clone()
    };
    let scope = if usable {
        previous.scope.clone()
    } else {
        scope_label(file)
    };
    let entry = crate::source_snapshot::SourceEntry {
        path: key.clone(),
        bytes,
    };
    let (mut report, refreshed, reuse) = refresh_files(
        usable.then_some(previous),
        &scope,
        config_fingerprint,
        std::slice::from_ref(&entry),
    )?;
    for analysis in &mut report.files {
        retarget_to_display(analysis, &display);
    }
    if !usable {
        return Ok((report, refreshed, reuse));
    }
    // Merge the one refreshed entry back; an entry dropped by a parse error
    // must remove the stale stored metrics instead of reusing them.
    let mut merged = previous.clone();
    merged.files.remove(&key);
    merged.files.extend(refreshed.files);
    Ok((report, merged, reuse))
}

/// Read-only warm report: reuse the stored index, never write, never walk Git.
///
/// A missing or unusable index degrades to a full analysis. This is the entry
/// point read-only callers (MCP) use.
pub fn warm_report(
    root: &Path,
    index_dir: &Path,
    scope: &str,
    config_fingerprint: &str,
    excludes: &[String],
) -> crate::Result<(crate::core::AnalysisReport, Reuse)> {
    let previous = AnalysisIndex::open(index_dir);
    warm_report_with(&previous, root, scope, config_fingerprint, excludes)
}

/// Warm report against an index the caller already holds open.
pub fn warm_report_with(
    previous: &AnalysisIndex,
    root: &Path,
    scope: &str,
    config_fingerprint: &str,
    excludes: &[String],
) -> crate::Result<(crate::core::AnalysisReport, Reuse)> {
    if root.is_file() {
        let (report, _, reuse) = refresh_file_index(root, previous, config_fingerprint)?;
        return Ok((report, reuse));
    }
    let entries = load_entries(root, excludes)?;
    let (report, _, reuse) = refresh_files(Some(previous), scope, config_fingerprint, &entries)?;
    Ok((report, reuse))
}
