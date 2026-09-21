//! Unused code: files, dependencies, and exports nothing reaches.
//!
//! Reachability comes from the resolved dependency graph: a file is unused
//! when no entry point reaches it over resolved import edges. Entry points are
//! the `--entry` patterns, `[unused] entries`, the `package.json` fields that
//! name source files in every discovered manifest, and the `index.*`/`main.*`
//! conventions at the root, under `src/`, and inside every directory that
//! declares a `package.json`.
//!
//! Honesty rules, reported instead of hidden:
//!
//! - Unresolved references make the report incomplete
//!   (`reason = "unresolved_references"`): reachability cannot be trusted.
//!   An empty entry set also makes it incomplete (`"no_entry_points"`), and no
//!   file is reported unused because reachability from nothing means nothing.
//!   Unresolved references win when both hold: they describe the graph the
//!   other verdicts rest on.
//! - Test files (`*.test.*`, `*.spec.*`, `test`/`tests`/`__tests__`
//!   directories) stay out of both unused lists unless `include_tests` is set,
//!   and the skipped files are counted in `excluded_test_files`.
//! - `export * from "./x"` is treated conservatively: every export of `x`
//!   counts as used, and the re-exporting file is listed in
//!   `export_star_files`.
//! - Exports of entry points count as used: they are the public surface.
//! - Uses with no static trace stay invisible: dynamic `module[name]` access,
//!   string-concatenated requires, namespace imports (`import * as ns`), and
//!   framework conventions.
//!
//! Nothing here writes to the analyzed repository.

use crate::Result;
use crate::config::UnusedConfig;
use crate::core::METRIC_PROFILE;
use crate::graph::DependencyReport;
use crate::parser::{ExportKind, ExportedSymbol, ParsedDependencies, extract_dependencies};
use crate::source_snapshot::SourceEntry;
use crate::{discovery, graph, normalized_relative_path};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const UNUSED_SCHEMA_VERSION: u32 = 1;

/// `complete` reason: the graph holds references it could not resolve.
const REASON_UNRESOLVED: &str = "unresolved_references";
/// `complete` reason: no entry point was found.
const REASON_NO_ENTRY_POINTS: &str = "no_entry_points";

/// Entry-point provenance: a `--entry` pattern.
const SOURCE_ENTRY: &str = "entry";
/// Entry-point provenance: `[unused] entries`.
const SOURCE_CONFIG: &str = "config";
/// Entry-point provenance: a `package.json` field.
const SOURCE_PACKAGE_JSON: &str = "package.json";
/// Entry-point provenance: the `index.*`/`main.*` conventions.
const SOURCE_CONVENTION: &str = "convention";

/// The only manifest section in scope; tooling consumes the others.
const DEPENDENCY_KIND: &str = "dependencies";

/// Reported kind of a named export.
const KIND_NAMED: &str = "named";
/// Reported kind of a default export.
const KIND_DEFAULT: &str = "default";

/// One file reachability starts from, with the reason it was chosen.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EntryPoint {
    pub path: String,
    /// One of `entry`, `config`, `package.json`, `convention`.
    pub source: &'static str,
}

/// One analyzed file no entry point reaches.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnusedFile {
    pub path: String,
}

/// One `dependencies` entry no source file of its manifest imports.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnusedDependency {
    /// Analysis-root-relative path of the declaring `package.json`.
    pub manifest: String,
    pub package: String,
    /// Always `dependencies`: the other sections are out of scope.
    pub kind: &'static str,
}

/// One export no import, re-export, or entry-point exemption covers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnusedExport {
    pub path: String,
    pub name: String,
    pub line: u32,
    /// `named` or `default`.
    pub kind: &'static str,
}

/// Unused files, dependencies, and exports of one analysis root.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnusedReport {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub complete: bool,
    pub reason: Option<&'static str>,
    pub files_analyzed: usize,
    pub entry_points: Vec<EntryPoint>,
    pub unused_files: Vec<UnusedFile>,
    pub unused_dependencies: Vec<UnusedDependency>,
    pub unused_exports: Vec<UnusedExport>,
    /// Candidates skipped as test files.
    pub excluded_test_files: usize,
    /// Files whose exports a star re-export makes used.
    pub export_star_files: Vec<String>,
    /// References the graph could not resolve.
    pub unresolved: usize,
}

/// Reports unused files, dependencies, and exports under `path`.
///
/// `extra_entries` and `config.entries` are gitignore-style patterns relative
/// to the analysis root; `excludes` skips files the same way the rest of the
/// tool does. Every list in the report is sorted, so equal inputs produce
/// byte-identical output.
pub fn analyze_unused(
    path: &Path,
    excludes: &[String],
    extra_entries: &[String],
    config: &UnusedConfig,
) -> Result<UnusedReport> {
    let root = analysis_root(path);
    let discovered = discovery::discover_with_excludes(path, excludes)?;
    let sources = read_sources(&discovered, root)?;
    let files: Vec<String> = sources.iter().map(|entry| entry.path.clone()).collect();
    let index = path_index(&files);
    let manifests = if path.is_dir() {
        discover_manifests(path, excludes, root)?
    } else {
        Vec::new()
    };

    let mut outcomes: Vec<(String, Result<ParsedDependencies>)> = sources
        .par_iter()
        .map(|entry| {
            (
                entry.path.clone(),
                extract_dependencies(&entry.path, &entry.bytes),
            )
        })
        .collect();
    outcomes.sort_by(|left, right| left.0.cmp(&right.0));
    let parsed = outcomes
        .into_iter()
        .map(|(path, dependencies)| Ok((path, dependencies?)))
        .collect::<Result<Vec<_>>>()?;

    let symbols = SymbolIndex::new(&parsed, &index);
    let graph = graph::dependency_report_from_parsed(parsed);

    let mut entry_points = matched_entries(&files, extra_entries, SOURCE_ENTRY)?;
    entry_points.extend(matched_entries(&files, &config.entries, SOURCE_CONFIG)?);
    entry_points.extend(manifest_entries(&manifests, root, &index));
    entry_points.extend(convention_entries(&manifests, &files, &index));
    entry_points.sort_by(|left, right| {
        (left.path.as_str(), left.source).cmp(&(right.path.as_str(), right.source))
    });
    entry_points.dedup();

    let reachable = reachable_files(&entry_points, &graph);
    let entry_paths: BTreeSet<&str> = entry_points
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();

    let mut unused_files = Vec::new();
    let mut excluded_test_files = 0;
    if !entry_points.is_empty() {
        for file in &files {
            if reachable.contains(file.as_str()) || entry_paths.contains(file.as_str()) {
                continue;
            }
            if !config.include_tests && is_test_file(file) {
                excluded_test_files += 1;
                continue;
            }
            unused_files.push(UnusedFile { path: file.clone() });
        }
    }

    let unused_file_paths: BTreeSet<&str> =
        unused_files.iter().map(|file| file.path.as_str()).collect();
    // A file nobody reaches already carries its own row; listing each of its
    // exports beside it is the same finding three times.
    let unused_exports: Vec<UnusedExport> = collect_unused_exports(&symbols, &entry_paths, config)
        .into_iter()
        .filter(|export| !unused_file_paths.contains(export.path.as_str()))
        .collect();
    let unused_dependencies = collect_unused_dependencies(&manifests, root, &sources)?;

    let unresolved = graph.unresolved.len();
    let (complete, reason) = if unresolved > 0 {
        (false, Some(REASON_UNRESOLVED))
    } else if entry_points.is_empty() {
        (false, Some(REASON_NO_ENTRY_POINTS))
    } else {
        (true, None)
    };

    Ok(UnusedReport {
        schema_version: UNUSED_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        complete,
        reason,
        files_analyzed: files.len(),
        entry_points,
        unused_files,
        unused_dependencies,
        unused_exports,
        excluded_test_files,
        export_star_files: symbols.star_files,
        unresolved,
    })
}

/// Export and import symbols of every analyzed file.
struct SymbolIndex {
    /// Export lists by file, in path order.
    exports: Vec<(String, Vec<ExportedSymbol>)>,
    /// Every name imported or re-exported by name anywhere.
    imported: BTreeSet<String>,
    /// Files whose exports a star re-export re-exports wholesale.
    starred: BTreeSet<String>,
    /// Files containing a star re-export.
    star_files: Vec<String>,
}

impl SymbolIndex {
    fn new(parsed: &[(String, ParsedDependencies)], index: &BTreeMap<String, Vec<String>>) -> Self {
        let mut exports = Vec::new();
        let mut imported = BTreeSet::new();
        let mut starred = BTreeSet::new();
        let mut star_files = Vec::new();
        for (path, dependencies) in parsed {
            let mut reexports = false;
            for symbol in &dependencies.exports {
                if symbol.kind != ExportKind::Star {
                    continue;
                }
                reexports = true;
                for target in star_targets(path, &symbol.name, index) {
                    starred.insert(target);
                }
            }
            if reexports {
                star_files.push(path.clone());
            }
            if !dependencies.exports.is_empty() {
                exports.push((path.clone(), dependencies.exports.clone()));
            }
            for symbol in &dependencies.imports {
                if !symbol.star && !symbol.name.is_empty() {
                    imported.insert(symbol.name.clone());
                }
            }
        }
        Self {
            exports,
            imported,
            starred,
            star_files,
        }
    }
}

/// Exports no import, star re-export, or entry-point exemption covers.
///
/// Matching is by name across the whole analysis set, not per importing
/// module: a name that any file imports counts as used everywhere, which
/// under-reports rather than inventing dead code.
fn collect_unused_exports(
    symbols: &SymbolIndex,
    entry_paths: &BTreeSet<&str>,
    config: &UnusedConfig,
) -> Vec<UnusedExport> {
    let mut unused = Vec::new();
    for (path, exports) in &symbols.exports {
        if !config.include_tests && is_test_file(path) {
            continue;
        }
        if entry_paths.contains(path.as_str()) || symbols.starred.contains(path) {
            continue;
        }
        for export in exports {
            let kind = match export.kind {
                // A star re-export names no export of its own to judge.
                ExportKind::Star => continue,
                ExportKind::Default => KIND_DEFAULT,
                ExportKind::Named => KIND_NAMED,
            };
            if symbols.imported.contains(&export.name) {
                continue;
            }
            unused.push(UnusedExport {
                path: path.clone(),
                name: export.name.clone(),
                line: export.line,
                kind,
            });
        }
    }
    unused.sort_by(|left, right| {
        (&left.path, left.line, &left.name).cmp(&(&right.path, right.line, &right.name))
    });
    unused
}

/// Every `package.json` under `path`, as `(directory, path)` display pairs.
///
/// One discovery serves both manifest-declared entry points and dependency
/// attribution, so a package's own manifest is read once.
fn discover_manifests(
    path: &Path,
    excludes: &[String],
    root: &Path,
) -> Result<Vec<(String, String)>> {
    let mut manifests = Vec::new();
    for manifest in discovery::discover_matching(path, excludes, is_manifest)? {
        let display = normalized_relative_path(&manifest, root);
        manifests.push((manifest_directory(&display), display));
    }
    Ok(manifests)
}

/// Dependencies of every manifest that none of its own source files imports.
///
/// A file belongs to the nearest ancestor manifest, so a workspace package's
/// imports never mark the root manifest's dependencies used. A manifest that
/// cannot be read or parsed declares nothing: `unused` never fails on a file
/// it does not own.
fn collect_unused_dependencies(
    manifests: &[(String, String)],
    root: &Path,
    sources: &[SourceEntry],
) -> Result<Vec<UnusedDependency>> {
    if manifests.is_empty() {
        return Ok(Vec::new());
    }
    let directories: Vec<String> = manifests.iter().map(|(dir, _)| dir.clone()).collect();
    let mut imported: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for import in &graph::external_packages_from_sources(sources)? {
        if let Some(directory) = nearest_directory(&import.path, &directories) {
            imported
                .entry(directory.to_owned())
                .or_default()
                .insert(import.package.clone());
        }
    }

    let mut unused = Vec::new();
    for (directory, manifest) in manifests {
        let used = imported.get(directory);
        for package in declared_dependencies(&root.join(manifest)) {
            if used.is_some_and(|used| used.contains(&package)) {
                continue;
            }
            unused.push(UnusedDependency {
                manifest: manifest.clone(),
                package,
                kind: DEPENDENCY_KIND,
            });
        }
    }
    unused.sort_by(|left, right| {
        (&left.manifest, &left.package).cmp(&(&right.manifest, &right.package))
    });
    Ok(unused)
}

/// `dependencies` keys of one manifest, sorted.
fn declared_dependencies(manifest: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(manifest) else {
        return Vec::new();
    };
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    let Some(dependencies) = document
        .get(DEPENDENCY_KIND)
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };
    let mut packages: Vec<String> = dependencies.keys().cloned().collect();
    packages.sort();
    packages
}

/// Files reachable from the entry points over resolved edges, entries
/// included.
fn reachable_files<'a>(
    entries: &'a [EntryPoint],
    graph: &'a DependencyReport,
) -> BTreeSet<&'a str> {
    let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &graph.edges {
        adjacency
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
    }
    let mut reachable = BTreeSet::new();
    let mut stack: Vec<&str> = Vec::new();
    for entry in entries {
        if reachable.insert(entry.path.as_str()) {
            stack.push(entry.path.as_str());
        }
    }
    while let Some(file) = stack.pop() {
        for target in adjacency.get(file).into_iter().flatten().copied() {
            if reachable.insert(target) {
                stack.push(target);
            }
        }
    }
    reachable
}

/// Files matching gitignore-style entry patterns.
///
/// A pattern that matches nothing contributes no entry point; a pattern the
/// gitignore syntax rejects fails the analysis instead of silently dropping
/// an entry point.
fn matched_entries(
    files: &[String],
    patterns: &[String],
    source: &'static str,
) -> Result<Vec<EntryPoint>> {
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new("");
    for pattern in patterns {
        builder.add_line(None, pattern)?;
    }
    let matcher = builder.build()?;
    let mut entries = Vec::new();
    for file in files {
        if matcher
            .matched_path_or_any_parents(Path::new(file), false)
            .is_ignore()
        {
            entries.push(EntryPoint {
                path: file.clone(),
                source,
            });
        }
    }
    Ok(entries)
}

/// Entry points named by the discovered manifests' source-naming fields.
///
/// Each value is relative to its own manifest's directory, so a workspace
/// package declares its own entry points instead of leaving its subtree
/// unreachable.
fn manifest_entries(
    manifests: &[(String, String)],
    root: &Path,
    index: &BTreeMap<String, Vec<String>>,
) -> Vec<EntryPoint> {
    let mut entries = Vec::new();
    for (directory, manifest) in manifests {
        for value in manifest_values(&root.join(manifest)) {
            let value = value.strip_prefix("./").unwrap_or(value.as_str());
            if value.is_empty() {
                continue;
            }
            let key = if directory.is_empty() {
                value.to_owned()
            } else {
                format!("{directory}/{value}")
            };
            for path in index.get(&key).into_iter().flatten() {
                entries.push(EntryPoint {
                    path: path.clone(),
                    source: SOURCE_PACKAGE_JSON,
                });
            }
        }
    }
    entries
}

/// Every string of the root manifest's source-naming fields.
fn manifest_values(manifest: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(manifest) else {
        return Vec::new();
    };
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for field in ["main", "module", "browser", "types", "bin", "exports"] {
        if let Some(value) = document.get(field) {
            collect_strings(value, &mut values);
        }
    }
    values
}

/// Every string in a JSON subtree.
fn collect_strings(value: &serde_json::Value, values: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => values.push(text.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(item, values);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_strings(item, values);
            }
        }
        _ => {}
    }
}

/// Entry points from the `index.*` and `main.*` conventions: at the root, under
/// `src/`, and inside every directory that declares a `package.json`.
///
/// A directory import and a host loader both resolve those files without an
/// import edge, so a package's own `index.*` is an entry point even when the
/// manifest names no source field.
fn convention_entries(
    manifests: &[(String, String)],
    files: &[String],
    index: &BTreeMap<String, Vec<String>>,
) -> Vec<EntryPoint> {
    let mut keys = vec![
        "index".to_owned(),
        "main".to_owned(),
        "src/index".to_owned(),
        "src/main".to_owned(),
    ];
    for (directory, _) in manifests {
        if directory.is_empty() {
            continue;
        }
        keys.push(format!("{directory}/index"));
        keys.push(format!("{directory}/main"));
    }
    let mut entries = Vec::new();
    for key in keys {
        for path in index.get(&key).into_iter().flatten() {
            entries.push(EntryPoint {
                path: path.clone(),
                source: SOURCE_CONVENTION,
            });
        }
    }
    // Tooling loads `*.config.*` by name rather than by import, so those files
    // are entry points too; without them every config file reads as unused.
    for path in files {
        if is_config_file(path) {
            entries.push(EntryPoint {
                path: path.clone(),
                source: SOURCE_CONVENTION,
            });
        }
    }
    // Cargo compiles crate roots by name rather than through an import edge, so
    // without them a Rust crate reads as entirely unused.
    for path in files {
        if crate::parser::rust::is_crate_root(path) {
            entries.push(EntryPoint {
                path: path.clone(),
                source: SOURCE_CONVENTION,
            });
        }
    }
    // A C or C++ translation unit is compiled rather than included, so no edge
    // can reach it either; headers stay subject to reachability, because a
    // library's public header has no in-repository includer.
    for path in files {
        if crate::parser::c_family::is_translation_unit(path) {
            entries.push(EntryPoint {
                path: path.clone(),
                source: SOURCE_CONVENTION,
            });
        }
    }
    entries
}

/// True for the `*.config.<source extension>` files tooling loads by name.
fn is_config_file(path: &str) -> bool {
    let (_, name) = split_path(path);
    let Some((stem, _)) = name.rsplit_once('.') else {
        return false;
    };
    stem.ends_with(".config")
}

/// Discovered paths keyed by exact path, extension-stripped stem, and — for
/// `index.*` files — their directory.
///
/// One map answers every value that names a file: `"src/cli.ts"` by path,
/// `"src/cli"` by stem, and `"src"` by the directory of `src/index.ts`. A key
/// several files answer to keeps all of them, so an ambiguous value
/// over-approximates the entry set instead of guessing.
fn path_index(files: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut index: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in files {
        index.entry(file.clone()).or_default().insert(file.clone());
        index
            .entry(strip_extension(file))
            .or_default()
            .insert(file.clone());
        let (directory, name) = split_path(file);
        if name.starts_with("index.") {
            index
                .entry(directory.to_owned())
                .or_default()
                .insert(file.clone());
        }
    }
    index
        .into_iter()
        .map(|(key, files)| (key, files.into_iter().collect()))
        .collect()
}

/// Path without its final extension; a name without one is unchanged.
fn strip_extension(path: &str) -> String {
    let (directory, name) = split_path(path);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if directory.is_empty() {
        stem.to_owned()
    } else {
        format!("{directory}/{stem}")
    }
}

/// Directory and file name of an analysis-root-relative path.
fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// Directory of a manifest's analysis-root-relative path.
fn manifest_directory(manifest: &str) -> String {
    split_path(manifest).0.to_owned()
}

/// Nearest ancestor manifest directory of `path`, root manifest included.
fn nearest_directory<'a>(path: &str, directories: &'a [String]) -> Option<&'a str> {
    directories
        .iter()
        .filter(|directory| {
            directory.is_empty()
                || path
                    .strip_prefix(directory.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|directory| directory.len())
        .map(String::as_str)
}

/// Files a star re-export names.
///
/// The specifier resolves through [`path_index`] rather than through the
/// graph's own resolver, which is not reachable from here: a specifier that
/// resolves to several files exempts all of them, which errs towards calling
/// an export used instead of reporting live code as dead.
fn star_targets(
    source: &str,
    specifier: &str,
    index: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let Some(base) = relative_target(source, specifier) else {
        return Vec::new();
    };
    index.get(&base).cloned().unwrap_or_default()
}

/// `source`'s directory joined with a relative specifier, or `None` when the
/// specifier names a package instead of a file.
fn relative_target(source: &str, specifier: &str) -> Option<String> {
    if !matches!(specifier, "." | "..")
        && !specifier.starts_with("./")
        && !specifier.starts_with("../")
    {
        return None;
    }
    let mut components: Vec<&str> = source.split('/').collect();
    components.pop();
    for component in specifier.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            value => components.push(value),
        }
    }
    Some(components.join("/"))
}

/// Test-file conventions: `*.test.*`, `*.spec.*`, and `test`/`tests`/
/// `__tests__` directories.
fn is_test_file(path: &str) -> bool {
    let (directory, name) = split_path(path);
    if directory
        .split('/')
        .any(|component| matches!(component, "test" | "tests" | "__tests__"))
    {
        return true;
    }
    let mut parts = name.split('.');
    parts.next();
    parts.any(|part| matches!(part, "test" | "spec"))
}

/// True for files named `package.json`.
fn is_manifest(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("package.json")
}

/// Directory the analysis paths are relative to, matching the graph.
fn analysis_root(path: &Path) -> &Path {
    if path.is_file() {
        path.parent().unwrap_or(Path::new("."))
    } else {
        path
    }
}

/// Reads every discovered file, keyed by its analysis-root-relative path.
fn read_sources(discovered: &[PathBuf], root: &Path) -> Result<Vec<SourceEntry>> {
    discovered
        .par_iter()
        .map(|file| {
            Ok(SourceEntry {
                path: normalized_relative_path(file, root),
                bytes: std::fs::read(file)?,
            })
        })
        .collect::<Result<Vec<_>>>()
}
