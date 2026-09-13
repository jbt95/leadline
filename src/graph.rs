//! Deterministic internal source dependency graph extraction.

use crate::core::METRIC_PROFILE;
use crate::parser::{ParsedDependencies, RawDependency, RawDependencyKind, extract_dependencies};
use crate::source_snapshot::SourceEntry;
use crate::{Result, normalized_relative_path};
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
    let mut parsed_files = Vec::with_capacity(discovered.len());
    for file in discovered {
        let display_path = normalized_relative_path(&file, root);
        let source = std::fs::read(&file)?;
        let parsed = extract_dependencies(&display_path, &source)?;
        parsed_files.push(ParsedFile {
            path: display_path,
            parsed,
        });
    }
    Ok(dependency_report(parsed_files))
}

/// Extracts and resolves dependencies among in-memory source entries.
pub fn analyze_dependencies_from_sources(entries: &[SourceEntry]) -> Result<DependencyReport> {
    let mut parsed_files = Vec::with_capacity(entries.len());
    for entry in entries {
        let parsed = extract_dependencies(&entry.path, &entry.bytes)?;
        parsed_files.push(ParsedFile {
            path: entry.path.clone(),
            parsed,
        });
    }
    Ok(dependency_report(parsed_files))
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
