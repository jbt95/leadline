pub mod agent;
pub mod analytics;
pub mod baseline;
pub mod config;
pub mod core;
pub mod coupling;
pub mod coverage;
pub mod debt;
pub mod diff;
pub mod discovery;
pub mod duplication;
pub mod external;
pub mod git;
pub mod graph;
pub mod history;
pub mod hotspots;
pub mod http;
pub mod impact;
pub mod index;
pub mod mcp;
pub mod mutation;
pub mod ownership;
pub mod parser;
pub mod pg_plan;
pub mod policy;
pub mod project;
pub mod render;
pub mod report;
pub mod risk;
pub mod sarif;
pub mod security;
pub mod snapshots;
pub mod source_snapshot;
pub mod sql;
pub mod stats;
pub mod telemetry;
pub mod test_relationships;
pub mod test_targets;
pub mod unused;
pub mod update;
pub mod vulnerabilities;

use crate::core::{
    AnalysisReport, FileAnalysis, METRIC_PROFILE, MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use crate::coverage::CoverageMap;
use crate::parser::{ParserBackend, TreeSitterBackend};
use crate::source_snapshot::SourceEntry;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub fn analyze_path(path: &Path, coverage: Option<&CoverageMap>) -> Result<AnalysisReport> {
    analyze_path_with_excludes(path, coverage, &[])
}

pub fn analyze_path_with_excludes(
    path: &Path,
    coverage: Option<&CoverageMap>,
    excludes: &[String],
) -> Result<AnalysisReport> {
    if path.is_file() {
        let source = std::fs::read(path)?;
        let display = normalize_path(path);
        let backend = TreeSitterBackend;
        let mut result = backend.analyze(&display, &source)?;
        if let Some(coverage) = coverage {
            coverage.apply(&mut result);
        }
        return Ok(analysis_report(vec![result]));
    }
    let paths = discovery::discover_with_excludes(path, excludes)?;
    let mut files = paths
        .par_iter()
        .map(|file| analyze_file(file, path, coverage))
        .collect::<Result<Vec<_>>>()?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(analysis_report(files))
}

/// Analyzes in-memory source entries as if they were discovered under one root.
///
/// Entries use analysis-root-relative `/` paths. Parallel analysis preserves
/// no input order: the returned report sorts files by path so equivalent
/// filesystem and in-memory file sets produce identical reports.
pub fn analyze_sources(
    entries: &[SourceEntry],
    coverage: Option<&CoverageMap>,
) -> Result<AnalysisReport> {
    let backend = TreeSitterBackend;
    let mut files = entries
        .par_iter()
        .map(|entry| {
            let mut result = backend.analyze(&entry.path, &entry.bytes)?;
            if let Some(coverage) = coverage {
                coverage.apply(&mut result);
            }
            Ok(result)
        })
        .collect::<Result<Vec<_>>>()?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(analysis_report(files))
}

pub(crate) fn analysis_report(files: Vec<FileAnalysis>) -> AnalysisReport {
    AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        metric_specs: MetricSpecs::default(),
        files,
    }
}

pub fn analyze_file(
    path: &Path,
    root: &Path,
    coverage: Option<&CoverageMap>,
) -> Result<FileAnalysis> {
    let source = std::fs::read(path)?;
    let display_path = normalized_relative_path(path, root);
    let backend = TreeSitterBackend;
    let mut result = backend.analyze(&display_path, &source)?;
    if let Some(coverage) = coverage {
        coverage.apply(&mut result);
    }
    Ok(result)
}

pub fn analyze_source(path: &str, source: &[u8]) -> Result<FileAnalysis> {
    TreeSitterBackend.analyze(path, source)
}

pub fn normalized_relative_path(path: &Path, root: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    normalize_path(relative)
}

/// Why a scoped target could not be resolved.
#[derive(Debug)]
pub enum ScopedTargetError {
    /// The target path does not exist under the scope root.
    Missing(String),
    /// The target path resolves outside the scope root.
    Outside(String),
    /// The root or target could not be canonicalized.
    Io(String),
}

impl std::fmt::Display for ScopedTargetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(message) | Self::Outside(message) | Self::Io(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for ScopedTargetError {}

/// Resolve a user-supplied target to a scope-relative normalized path.
///
/// Validates the file exists, then canonicalizes both sides so symlinks and
/// Windows verbatim prefixes cannot split the join. Shared by the CLI and MCP
/// `coupling`/`impact` so containment validation is implemented once.
pub fn resolve_scoped_target(
    root: &Path,
    target: &str,
) -> std::result::Result<String, ScopedTargetError> {
    let requested = Path::new(target);
    let absolute_target = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    if !absolute_target.is_file() {
        return Err(ScopedTargetError::Missing(format!(
            "target '{target}' was not found under {}",
            root.display()
        )));
    }
    let canonical_root =
        std::fs::canonicalize(root).map_err(|error| ScopedTargetError::Io(error.to_string()))?;
    let canonical_target = std::fs::canonicalize(&absolute_target)
        .map_err(|error| ScopedTargetError::Io(error.to_string()))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(ScopedTargetError::Outside(format!(
            "target '{target}' is outside the scope {}",
            root.display()
        )));
    }
    Ok(normalized_relative_path(&canonical_target, &canonical_root))
}

pub fn normalize_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for part in path.components() {
        use std::path::Component;
        match part {
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop();
            }
            Component::RootDir | Component::Prefix(_) => parts.clear(),
            Component::CurDir => {}
        }
    }
    parts.join("/")
}

/// `std::fs::canonicalize` returns verbatim (`\\?\`) paths on Windows, which
/// never match the plain paths git prints. Git-facing code passes
/// canonicalized paths through this strip so comparisons always compare like
/// with like. Identity off Windows.
#[cfg(windows)]
pub(crate) fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    const UNC_PREFIX: &str = r"\\?\UNC\";
    const VERBATIM_PREFIX: &str = r"\\?\";
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(UNC_PREFIX) {
        return PathBuf::from(format!("\\\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(VERBATIM_PREFIX) {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

#[cfg(not(windows))]
pub(crate) fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
}
