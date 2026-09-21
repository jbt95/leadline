//! Deterministic internal source dependency graph extraction.

use crate::core::METRIC_PROFILE;
use crate::parser::{ParsedDependencies, RawDependency, RawDependencyKind, extract_dependencies};
use crate::source_snapshot::SourceEntry;
use crate::{Result, normalized_relative_path};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const DEPENDENCY_SCHEMA_VERSION: u32 = 1;

const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];
const TYPESCRIPT_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts"];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DependencyFile {
    pub path: String,
    pub fan_in: usize,
    pub fan_out: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DependencyEdge {
    pub source: String,
    pub target: String,
    pub kind: &'static str,
    pub confidence: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnresolvedDependency {
    pub source: String,
    pub specifier: String,
    pub line: u32,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DependencyCycle {
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DependencyReport {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub files: Vec<DependencyFile>,
    pub edges: Vec<DependencyEdge>,
    pub unresolved: Vec<UnresolvedDependency>,
    pub cycles: Vec<DependencyCycle>,
}

struct ParsedFile {
    path: String,
    parsed: ParsedDependencies,
}

enum Resolution {
    Resolved(String),
    Unresolved(&'static str),
    Ignored,
}

/// Extracts and resolves dependencies among supported source files in `path`.
pub fn analyze_dependencies(path: &Path, excludes: &[String]) -> Result<DependencyReport> {
    let discovered = crate::discovery::discover_with_excludes(path, excludes)?;
    let root = analysis_root(path);
    let outcomes: Vec<(String, Result<ParsedDependencies>)> = discovered
        .par_iter()
        .map(|file| {
            let display_path = normalized_relative_path(file, root);
            let parsed = std::fs::read(file)
                .map_err(crate::Error::from)
                .and_then(|source| extract_dependencies(&display_path, &source));
            (display_path, parsed)
        })
        .collect();
    report_from_parsed(outcomes)
}

/// Extracts and resolves dependencies among in-memory source entries.
pub fn analyze_dependencies_from_sources(entries: &[SourceEntry]) -> Result<DependencyReport> {
    let outcomes: Vec<(String, Result<ParsedDependencies>)> = entries
        .par_iter()
        .map(|entry| {
            (
                entry.path.clone(),
                extract_dependencies(&entry.path, &entry.bytes),
            )
        })
        .collect();
    report_from_parsed(outcomes)
}

/// Resolves dependencies from already-extracted references.
pub(crate) fn dependency_report_from_parsed(
    parsed_files: Vec<(String, ParsedDependencies)>,
) -> DependencyReport {
    dependency_report(
        parsed_files
            .into_iter()
            .map(|(path, parsed)| ParsedFile { path, parsed })
            .collect(),
    )
}

/// Sorts extraction outcomes by path, then resolves the first error in path
/// order so failures stay deterministic under parallel extraction.
pub(crate) fn report_from_parsed(
    mut outcomes: Vec<(String, Result<ParsedDependencies>)>,
) -> Result<DependencyReport> {
    outcomes.sort_by(|left, right| left.0.cmp(&right.0));
    let parsed_files = outcomes
        .into_iter()
        .map(|(path, parsed)| {
            Ok(ParsedFile {
                path,
                parsed: parsed?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(dependency_report(parsed_files))
}

/// One bare npm import from a changed source file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExternalPackageImport {
    pub path: String,
    pub line: u32,
    pub package: String,
}

/// Bare JavaScript import/call specifiers from in-memory changed entries.
///
/// Only references the internal graph ignores: non-relative specifiers.
/// Scoped subpaths normalize to `@scope/name`, plain subpaths to `name`;
/// relative, absolute, and empty specifiers are skipped. Sorted and
/// deduplicated by package, path, and line.
pub fn external_packages_from_sources(
    entries: &[SourceEntry],
) -> Result<Vec<ExternalPackageImport>> {
    use crate::parser::{RawDependencyKind, extract_dependencies};
    let mut imports = BTreeSet::new();
    for entry in entries {
        let parsed = extract_dependencies(&entry.path, &entry.bytes)?;
        for reference in &parsed.references {
            match reference.kind {
                RawDependencyKind::JavaScriptImport | RawDependencyKind::JavaScriptCall => {
                    if let Some(package) = bare_package(&reference.specifier) {
                        imports.insert((package, entry.path.clone(), reference.line));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(imports
        .into_iter()
        .map(|(package, path, line)| ExternalPackageImport {
            path,
            line,
            package,
        })
        .collect())
}

/// Normalize one specifier to its npm package, or `None` when relative,
/// absolute, empty, or a Node builtin (`node:fs`).
fn bare_package(specifier: &str) -> Option<String> {
    if specifier.is_empty()
        || specifier == "."
        || specifier == ".."
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('/')
        || specifier.starts_with("node:")
    {
        return None;
    }
    if let Some(rest) = specifier.strip_prefix('@') {
        let mut parts = rest.split('/');
        match (parts.next(), parts.next()) {
            (Some(scope), Some(name)) if !scope.is_empty() && !name.is_empty() => {
                Some(format!("@{scope}/{name}"))
            }
            _ => None,
        }
    } else {
        specifier
            .split('/')
            .next()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
    }
}

fn dependency_report(mut parsed_files: Vec<ParsedFile>) -> DependencyReport {
    parsed_files.sort_by(|left, right| left.path.cmp(&right.path));

    let paths: BTreeSet<String> = parsed_files.iter().map(|file| file.path.clone()).collect();
    let java_types = java_type_index(&parsed_files);
    let mut edge_pairs: BTreeMap<(String, String), &'static str> = BTreeMap::new();
    let mut unresolved = Vec::new();

    for file in &parsed_files {
        for reference in &file.parsed.references {
            match resolve(reference, &file.path, &paths, &java_types) {
                Resolution::Resolved(target) => {
                    let kind = edge_kind(reference);
                    edge_pairs
                        .entry((file.path.clone(), target))
                        .and_modify(|existing| {
                            if kind == "import" {
                                *existing = "import";
                            }
                        })
                        .or_insert(kind);
                }
                Resolution::Unresolved(reason) => unresolved.push(UnresolvedDependency {
                    source: file.path.clone(),
                    specifier: reference.specifier.clone(),
                    line: reference.line,
                    reason,
                }),
                Resolution::Ignored => {}
            }
        }
    }
    unresolved.sort_by(|left, right| {
        (&left.source, &left.specifier, left.line, left.reason).cmp(&(
            &right.source,
            &right.specifier,
            right.line,
            right.reason,
        ))
    });

    let edges: Vec<DependencyEdge> = edge_pairs
        .iter()
        .map(|((source, target), kind)| DependencyEdge {
            source: source.clone(),
            target: target.clone(),
            kind,
            confidence: "high",
        })
        .collect();
    let files = dependency_files(&parsed_files, &edge_pairs);
    let cycles = dependency_cycles(&parsed_files, &edge_pairs);

    DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        files,
        edges,
        unresolved,
        cycles,
    }
}

fn analysis_root(path: &Path) -> &Path {
    if path.is_file() {
        path.parent().unwrap_or(Path::new("."))
    } else {
        path
    }
}

fn java_type_index(files: &[ParsedFile]) -> BTreeMap<String, Vec<String>> {
    let mut types: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in files {
        for name in &file.parsed.java_top_level_types {
            let qualified = file
                .parsed
                .java_package
                .as_ref()
                .map_or_else(|| name.clone(), |package| format!("{package}.{name}"));
            types.entry(qualified).or_default().push(file.path.clone());
        }
    }
    types
}

fn resolve(
    reference: &RawDependency,
    source: &str,
    paths: &BTreeSet<String>,
    java_types: &BTreeMap<String, Vec<String>>,
) -> Resolution {
    match reference.kind {
        RawDependencyKind::JavaScriptImport | RawDependencyKind::JavaScriptCall => {
            resolve_javascript(&reference.specifier, source, paths)
        }
        RawDependencyKind::JavaScriptUndecodable => Resolution::Unresolved("unsupported"),
        RawDependencyKind::Java => resolve_java(&reference.specifier, false, java_types),
        RawDependencyKind::JavaStatic => resolve_java(&reference.specifier, true, java_types),
        RawDependencyKind::JavaWildcard => Resolution::Unresolved("unsupported"),
        RawDependencyKind::LocalInclude => {
            resolve_local_include(&reference.specifier, source, paths)
        }
        // Go imports are module-qualified paths (`fmt`,
        // `github.com/org/repo/pkg`), never file-relative, so without
        // module-graph resolution there is nothing sound to resolve.
        RawDependencyKind::GoImport => Resolution::Ignored,
        // Rust modules are files: `mod foo;` names `foo.rs` or `foo/mod.rs`
        // beside the declaring file. `use` paths are module-qualified and
        // never reach this resolver.
        RawDependencyKind::RustModule => resolve_rust_module(&reference.specifier, source, paths),
        // Python relative imports name a module beside the declaring file, so
        // `from .mod import y` is resolvable: level 1 is the file's own
        // directory, deeper levels climb one directory each.
        RawDependencyKind::PythonImport => {
            resolve_python_module(&reference.specifier, source, paths)
        }
        // Absolute Python imports are package-qualified: sys.path and the
        // package root are invisible to a parser, so the reference is reported
        // rather than dropped, keeping `complete` honest, and adds no edge —
        // the same treatment Java wildcard imports get.
        RawDependencyKind::PythonAbsoluteImport => Resolution::Unresolved("unsupported"),
    }
}

fn edge_kind(reference: &RawDependency) -> &'static str {
    match reference.kind {
        RawDependencyKind::JavaScriptCall => "call",
        _ => "import",
    }
}

fn resolve_javascript(specifier: &str, source: &str, paths: &BTreeSet<String>) -> Resolution {
    if !matches!(specifier, "." | "..")
        && !specifier.starts_with("./")
        && !specifier.starts_with("../")
    {
        return Resolution::Ignored;
    }
    let Some(base) = relative_target(source, specifier) else {
        return Resolution::Unresolved("outside_scope");
    };
    // Exact matches win before the extension gate: discovered files are
    // supported by definition, so an explicit case-variant specifier still
    // resolves. Extension probing below stays lowercase-only, so an
    // extensionless specifier only matches lowercase candidate extensions.
    if paths.contains(&base) {
        return Resolution::Resolved(base);
    }
    if let Some(extension) = Path::new(&base)
        .extension()
        .and_then(|value| value.to_str())
        && !SOURCE_EXTENSIONS.contains(&extension)
    {
        return Resolution::Unresolved("unsupported");
    }

    if Path::new(&base).extension().is_none() {
        let appended = candidates(
            paths,
            SOURCE_EXTENSIONS
                .iter()
                .map(|extension| format!("{base}.{extension}")),
        );
        if let Some(result) = candidate_resolution(appended) {
            return result;
        }
        let indexed = candidates(
            paths,
            SOURCE_EXTENSIONS
                .iter()
                .map(|extension| format!("{base}/index.{extension}")),
        );
        if let Some(result) = candidate_resolution(indexed) {
            return result;
        }
    }

    if matches!(
        Path::new(&base)
            .extension()
            .and_then(|value| value.to_str()),
        Some("js" | "mjs" | "cjs")
    ) {
        let stem = base
            .rsplit_once('.')
            .map_or(base.as_str(), |(stem, _)| stem);
        let emitted = candidates(
            paths,
            TYPESCRIPT_EXTENSIONS
                .iter()
                .map(|extension| format!("{stem}.{extension}")),
        );
        if let Some(result) = candidate_resolution(emitted) {
            return result;
        }
    }
    Resolution::Unresolved("not_found")
}

fn resolve_local_include(specifier: &str, source: &str, paths: &BTreeSet<String>) -> Resolution {
    let Some(target) = relative_target(source, specifier) else {
        return Resolution::Unresolved("not_found");
    };
    candidate_resolution(candidates(paths, [target].into_iter()))
        .unwrap_or(Resolution::Unresolved("not_found"))
}

/// Resolves one Rust `mod foo;` declaration to `foo.rs` or `foo/mod.rs` inside
/// the declaring file's module directory.
///
/// The directory is not simply the file's own: `src/parser/mod.rs` declares
/// siblings, while `src/telemetry.rs` declares children under `src/telemetry/`.
fn resolve_rust_module(specifier: &str, source: &str, paths: &BTreeSet<String>) -> Resolution {
    let directory = crate::parser::rust::module_directory(source);
    let base = if directory.is_empty() {
        specifier.to_owned()
    } else {
        format!("{directory}/{specifier}")
    };
    candidate_resolution(candidates(
        paths,
        [format!("{base}.rs"), format!("{base}/mod.rs")].into_iter(),
    ))
    .unwrap_or(Resolution::Unresolved("not_found"))
}

/// Resolves one Python relative-import specifier to `<base>.py` or
/// `<base>/__init__.py`.
///
/// The extractor encodes the import level as leading `../` segments (level 1 =
/// none), so the shared relative walk already places the base: level 1 lands in
/// the declaring file's own directory and each further level climbs one
/// directory. Climbing above the analysis root is `outside_scope`, matching
/// the JavaScript resolver.
fn resolve_python_module(specifier: &str, source: &str, paths: &BTreeSet<String>) -> Resolution {
    let Some(base) = relative_target(source, specifier) else {
        return Resolution::Unresolved("outside_scope");
    };
    candidate_resolution(candidates(
        paths,
        [format!("{base}.py"), format!("{base}/__init__.py")].into_iter(),
    ))
    .unwrap_or(Resolution::Unresolved("not_found"))
}

fn candidates(paths: &BTreeSet<String>, candidates: impl Iterator<Item = String>) -> Vec<String> {
    candidates
        .filter(|candidate| paths.contains(candidate))
        .collect()
}

fn candidate_resolution(mut candidates: Vec<String>) -> Option<Resolution> {
    match candidates.len() {
        0 => None,
        1 => Some(Resolution::Resolved(
            candidates.pop().expect("one candidate"),
        )),
        _ => Some(Resolution::Unresolved("ambiguous")),
    }
}

fn relative_target(source: &str, specifier: &str) -> Option<String> {
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

fn resolve_java(
    specifier: &str,
    is_static: bool,
    java_types: &BTreeMap<String, Vec<String>>,
) -> Resolution {
    let mut candidate = specifier;
    loop {
        if let Some(paths) = java_types.get(candidate) {
            return match paths.as_slice() {
                [path] => Resolution::Resolved(path.clone()),
                _ => Resolution::Unresolved("ambiguous"),
            };
        }
        if !is_static {
            break;
        }
        let Some((parent, _)) = candidate.rsplit_once('.') else {
            break;
        };
        candidate = parent;
    }
    Resolution::Unresolved("not_found")
}

fn dependency_files(
    parsed_files: &[ParsedFile],
    edges: &BTreeMap<(String, String), &'static str>,
) -> Vec<DependencyFile> {
    let mut fan_in: BTreeMap<&str, usize> = BTreeMap::new();
    let mut fan_out: BTreeMap<&str, usize> = BTreeMap::new();
    for (source, target) in edges.keys() {
        *fan_out.entry(source).or_default() += 1;
        *fan_in.entry(target).or_default() += 1;
    }
    parsed_files
        .iter()
        .map(|file| DependencyFile {
            path: file.path.clone(),
            fan_in: fan_in.get(file.path.as_str()).copied().unwrap_or(0),
            fan_out: fan_out.get(file.path.as_str()).copied().unwrap_or(0),
        })
        .collect()
}

fn dependency_cycles(
    parsed_files: &[ParsedFile],
    edges: &BTreeMap<(String, String), &'static str>,
) -> Vec<DependencyCycle> {
    let indexes: BTreeMap<&str, usize> = parsed_files
        .iter()
        .enumerate()
        .map(|(index, file)| (file.path.as_str(), index))
        .collect();
    let mut adjacency = vec![Vec::new(); parsed_files.len()];
    let mut transpose = vec![Vec::new(); parsed_files.len()];
    for (source, target) in edges.keys() {
        let source = indexes[source.as_str()];
        let target = indexes[target.as_str()];
        adjacency[source].push(target);
        transpose[target].push(source);
    }

    let order = finish_order(&adjacency);
    let mut visited = vec![false; parsed_files.len()];
    let mut cycles = Vec::new();
    for start in order.into_iter().rev() {
        if visited[start] {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![start];
        visited[start] = true;
        while let Some(node) = stack.pop() {
            component.push(parsed_files[node].path.clone());
            for &next in transpose[node].iter().rev() {
                if !visited[next] {
                    visited[next] = true;
                    stack.push(next);
                }
            }
        }
        if component.len() >= 2 {
            component.sort();
            cycles.push(DependencyCycle { files: component });
        }
    }
    cycles.sort_by(|left, right| left.files.cmp(&right.files));
    cycles
}

fn finish_order(adjacency: &[Vec<usize>]) -> Vec<usize> {
    let mut visited = vec![false; adjacency.len()];
    let mut order = Vec::with_capacity(adjacency.len());
    for start in 0..adjacency.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, next_index)) = stack.last_mut() {
            if let Some(&next) = adjacency[*node].get(*next_index) {
                *next_index += 1;
                if !visited[next] {
                    visited[next] = true;
                    stack.push((next, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }
    order
}
