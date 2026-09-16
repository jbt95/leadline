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
        self.schema_version == INDEX_SCHEMA_VERSION
            && self.analyzer_version == env!("CARGO_PKG_VERSION")
            && self.metric_profile == METRIC_PROFILE
            && self.parser_versions == PARSER_VERSIONS
            && self.config_fingerprint == config_fingerprint
            && self.scope == scope
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(INDEX_FILE_NAME), json)
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

    let mut index = AnalysisIndex::empty(scope, config_fingerprint);
    let mut files = Vec::with_capacity(ordered.len());
    let mut reuse = Reuse::default();
    for entry in ordered {
        let key = content_key(&entry.bytes);
        let hit = reusable
            .and_then(|index| index.files.get(&entry.path))
            .filter(|file| file.key == key)
            .and_then(|file| rebuild(entry, file).map(|analysis| (file, analysis)));
        if let Some((file, analysis)) = hit {
            reuse.reused += 1;
            index.files.insert(entry.path.clone(), file.clone());
            files.push(analysis);
        } else {
            let analysis = crate::analyze_source(&entry.path, &entry.bytes)?;
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
    /// `Some(true)` when `--verify` re-derived an index byte-identical to the
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
    index.save(request.index_dir)?;
    Ok(IndexOutcome {
        path: request.index_dir.join(INDEX_FILE_NAME),
        index,
        reuse,
        verified,
    })
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
    let entries = load_entries(root, excludes)?;
    let (report, _, reuse) = refresh_files(Some(&previous), scope, config_fingerprint, &entries)?;
    Ok((report, reuse))
}
