//! Test selection: a change set plus a dependency graph, turned into the test
//! files a change can affect.
//!
//! The ladder is the whole design. Every changed file is placed on a step, and
//! a file that cannot be reasoned about escalates instead of being silently
//! dropped. The table is in the order the steps are evaluated, which is what
//! makes `always_run` win over the documentation skip:
//!
//! | Step | Condition | Action |
//! | --- | --- | --- |
//! | T4 `all` | it matches `always_run` | every test file |
//! | skip | a documentation or data extension | nothing, and no escalation |
//! | T3 `directory` | unsupported extension, a parse error, absent from the graph, or no test reached it | every test file under its top-level directory |
//! | T2 `graph` | the file is a graph node | test files in its reverse closure, and itself when it is one |
//!
//! The skip is a decision, not a gap: a changed file whose extension is in
//! [`NON_BEHAVIORAL_EXTENSIONS`] cannot change program behavior, so it selects
//! nothing and does not escalate. It is never a seed, so it is also excluded
//! from the `no test found` check below, which exists to catch a hole in the
//! conventions. A repository whose docs feed codegen declares
//! `always_run = ["**/*.md"]` and gets the full suite back, because T4 is
//! tested first.
//!
//! "Always produces something" is enforced rather than assumed: after the
//! conventions, the content check, and the closure have run, a changed file
//! that contributed no selected test escalates to T3 with reason
//! `no test found`. That is what catches a repository whose tests are inline,
//! and a change that genuinely has no test.
//!
//! Two constraints on the reverse walk are correctness-adjacent and shape the
//! implementation:
//!
//! - The closure is **complete, never truncated**. `analyze_impact` takes a
//!   display `limit`, so this crate walks the reverse adjacency through
//!   [`analysis::impact::reverse_reachability`] instead. That function is the
//!   small public multi-source addition: `analyze_impact` rebuilds its reverse
//!   map and re-walks per target, which would have made the cost multiplicative
//!   in the diff. One shared map and one walk answer every changed file and
//!   record, per reached file, which changed files reached it — which is `via`.
//! - The content check ([`crate::conventions::declares_tests`]) reads bytes, so
//!   it is applied **only to files the closure already reaches**: the reading
//!   scales with the blast radius of the diff, not with the repository.
//!
//! `totals.test_files` is the denominator `ratio` is measured against: the
//! union of the path-convention test files `parse::discovery` enumerates and
//! the selected set, so it can never be smaller than the numerator and `ratio`
//! is always ≤ 1. It is a *lower bound* on the repository's true test count,
//! not a total: a content-detected test file is only discovered when a change
//! reaches it, so a repository with heavy inline testing has more test files
//! than the denominator counts. The ratio therefore **over-estimates** the
//! fraction of the suite a selection runs, which is the safe direction to be
//! wrong in.
//!
//! `confidence.parse_errors` counts **changed** files that failed to parse: the
//! ladder parses those anyway to decide the T3 reason, which is exact and free.
//! It is not a repository-wide count, and it cannot see a syntax-broken
//! *dependant* — that file loses its outgoing edges and can drop a test from
//! the closure, while still being present in the graph. `unresolved` does not
//! cover it either: `unresolved` counts import forms that failed to resolve,
//! not files that failed to parse. Stage 0 accepts that blind spot; the parse
//! that lands with the index closes it for free.
//!
//! [`Selection`] is the whole output contract. There is no second shape for
//! `--format agent-json`: the compact envelope is the command's rendering of
//! this same value, the way every other report type in the workspace is
//! rendered by the surface that prints it.

use crate::conventions::{accepts_candidate, declares_tests, is_test_file};
use analysis::diff::{ChangeOptions, changed_paths};
use analysis::duplication::ExcludeMatcher;
use analysis::graph::{DependencyReport, analyze_dependencies};
use analysis::impact::reverse_reachability;
use config::TiaConfig;
use model::Result;
use parse::parser::detect_language;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const SELECTION_SCHEMA_VERSION: u32 = 1;
pub const SELECTION_MODEL: &str = "test-selection";

/// Extensions whose bytes cannot change program behavior: prose (`.md`,
/// `.markdown`, `.rst`, `.adoc`, `.txt`, `.pdf`) and images. A changed file
/// with one of these is skipped — nothing selected, no escalation — unless it
/// also matches `always_run`, which is evaluated first and wins.
const NON_BEHAVIORAL_EXTENSIONS: &[&str] = &[
    "md", "markdown", "rst", "adoc", "txt", "pdf", "png", "jpg", "jpeg", "gif", "svg", "webp",
    "ico", "bmp",
];

/// The escalation ladder tier one selected test was reached by.
///
/// Ordered by strength, so the strongest tier a test was reached by is the one
/// reported: a test in a reverse closure and in a directory escalation is
/// reported as `directory`, which is why it is really running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// T2: the changed file is a graph node and the test is in its closure.
    Graph,
    /// T3: the changed file could not be reasoned about, so every test file
    /// under its top-level directory was selected.
    Directory,
    /// T4: the changed file matches `always_run`, so every test file was
    /// selected.
    All,
}

/// One selected test file, with the tier that selected it and the changed files
/// that caused it, so `--explain` re-derives nothing.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SelectedTest {
    pub path: String,
    pub tier: Tier,
    pub via: BTreeSet<String>,
}

/// One changed file the ladder deliberately did not reason about, with the
/// class that explains it (currently always `documentation`). The entry is what
/// turns "nothing was selected" into an explanation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

/// The strongest escalation the selection needed, with one reason per changed
/// file that caused it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Escalation {
    pub tier: Tier,
    pub reasons: Vec<String>,
}

/// The selection's size against its denominator.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Totals {
    /// The union of the path-convention test files and the selected set: a
    /// lower bound on the true test-file count, never below `selected`.
    pub test_files: usize,
    pub selected: usize,
    /// `selected / test_files`, always ≤ 1, `0.0` when there is no test file.
    pub ratio: f64,
}

/// What the graph could see, so a small selection is never mistaken for a
/// confident one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Confidence {
    pub graph_files: usize,
    pub graph_edges: usize,
    /// Import forms no resolver could turn into an edge — in Go and
    /// package-qualified Python the whole graph is missing here.
    pub unresolved: usize,
    /// Changed files that failed to parse. Not a repository-wide count; see the
    /// module documentation for the blind spot.
    pub parse_errors: usize,
}

/// One selection, in the shape the command renders.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Selection {
    pub schema_version: u32,
    pub model: &'static str,
    pub base: String,
    pub changed: BTreeSet<String>,
    pub selected: Vec<SelectedTest>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation: Option<Escalation>,
    pub totals: Totals,
    pub confidence: Confidence,
}

/// Where a selection reads its inputs from.
#[derive(Clone, Debug)]
pub struct SelectionRequest<'a> {
    /// The analysis root: the directory the graph and discovery are scoped to.
    pub path: &'a Path,
    /// Which revision to compare against; `ComparisonTarget` decides whether
    /// untracked files join the change set.
    pub options: &'a ChangeOptions,
    /// The `[tia]` section.
    pub config: &'a TiaConfig,
    /// Analysis excludes, exactly as every other graph reader takes them.
    pub excludes: &'a [String],
}

/// Selection inputs that have already been read: the change set, the graph, the
/// path-convention test files, and a way to read one file's bytes.
///
/// [`select`] builds these from disk; tests build them from in-memory
/// [`parse::source_snapshot::SourceEntry`] fixtures.
pub struct SelectionInputs<'a> {
    /// The base revision label, carried through to the output.
    pub base: &'a str,
    /// The `[tia]` section.
    pub config: &'a TiaConfig,
    /// The change set, analysis-root-relative.
    pub changed: &'a BTreeSet<String>,
    /// The dependency graph over the same root.
    pub graph: &'a DependencyReport,
    /// Every path-convention test file under the root.
    pub test_files: &'a BTreeSet<String>,
    /// One root-relative file's bytes, `None` when it cannot be read.
    pub source_of: &'a dyn Fn(&str) -> Option<Vec<u8>>,
}

/// Selects the test files a change can affect.
pub fn select(request: &SelectionRequest<'_>) -> Result<Selection> {
    let changed = changed_paths(request.path, request.options)?;
    let graph = analyze_dependencies(request.path, request.excludes)?;
    let test_files = test_files(request.path, request.config, request.excludes)?;
    let root = analysis_root(request.path).to_owned();
    let source_of = |path: &str| std::fs::read(root.join(path)).ok();
    Ok(select_from(&SelectionInputs {
        base: &request.options.base,
        config: request.config,
        changed: &changed,
        graph: &graph,
        test_files: &test_files,
        source_of: &source_of,
    }))
}

/// Selects over inputs that have already been read, without touching the
/// filesystem beyond `source_of`.
pub fn select_from(inputs: &SelectionInputs<'_>) -> Selection {
    let mut selected: BTreeMap<String, (Tier, BTreeSet<String>)> = BTreeMap::new();
    let mut skipped = Vec::new();
    let mut reasons = Vec::new();
    let mut escalation: Option<Tier> = None;
    let mut seeds: BTreeSet<&str> = BTreeSet::new();
    let mut parse_errors = 0usize;

    let always_run = ExcludeMatcher::new(&inputs.config.always_run);
    for path in inputs.changed {
        // T4 first: a root config or lockfile edit has no edge to any test, and
        // this list is the only thing that keeps it from selecting nothing. It
        // is checked before the documentation skip so a repository whose docs
        // feed codegen can pull them back onto the full suite.
        if always_run.matches(path) {
            escalation = strongest(escalation, Tier::All);
            reasons.push(format!("always run: {path}"));
            for test in inputs.test_files {
                record(&mut selected, test, Tier::All, path);
            }
            continue;
        }
        // Documentation and data: bytes that cannot change program behavior, so
        // no test can be affected. Skipped rather than escalated, and never a
        // seed, so the `no test found` check below cannot see it.
        if is_documentation_or_data(path) {
            skipped.push(SkippedFile {
                path: path.clone(),
                reason: "documentation".to_owned(),
            });
            continue;
        }
        // T3: a file the ladder cannot reason about escalates to its top-level
        // directory rather than being skipped.
        if detect_language(path).is_none() {
            reasons.push(format!("unsupported extension: {path}"));
        } else if is_parse_error(path, inputs.source_of) {
            parse_errors += 1;
            reasons.push(format!("parse error: {path}"));
        } else if !inputs.graph.files.iter().any(|file| file.path == *path) {
            reasons.push(format!("not in graph: {path}"));
        } else {
            seeds.insert(path);
            continue;
        }
        escalation = strongest(escalation, Tier::Directory);
        escalate_directory(path, inputs.test_files, &mut selected);
    }

    // One shared walk for every seed: complete, untruncated, and linear in the
    // graph rather than in the diff.
    let reached = reverse_reachability(inputs.graph, seeds.iter().copied());
    let mut covered: BTreeSet<&str> = BTreeSet::new();
    for (node, sources) in &reached {
        if !is_test_candidate(node, inputs) {
            continue;
        }
        for source in sources {
            covered.insert(source);
        }
        for source in sources {
            record(&mut selected, node, Tier::Graph, source);
        }
    }
    // A changed file that contributed nothing escalates: an incomplete
    // convention table, an inline-test repository, or a change with genuinely
    // no test all end up here instead of silently selecting nothing.
    for seed in &seeds {
        if covered.contains(seed) {
            continue;
        }
        reasons.push(format!("no test found: {seed}"));
        escalation = strongest(escalation, Tier::Directory);
        escalate_directory(seed, inputs.test_files, &mut selected);
    }

    reasons.sort();
    reasons.dedup();
    let selected: Vec<SelectedTest> = selected
        .into_iter()
        .map(|(path, (tier, via))| SelectedTest { path, tier, via })
        .collect();
    // Selection can also pick a content-detected test file the path-convention
    // set never named, so the denominator is the union of the two: it can never
    // be smaller than the numerator, and `ratio` is always ≤ 1.
    let test_files = selected
        .iter()
        .filter(|test| !inputs.test_files.contains(&test.path))
        .count()
        + inputs.test_files.len();
    Selection {
        schema_version: SELECTION_SCHEMA_VERSION,
        model: SELECTION_MODEL,
        base: inputs.base.to_owned(),
        changed: inputs.changed.clone(),
        totals: Totals {
            test_files,
            selected: selected.len(),
            ratio: if test_files == 0 {
                0.0
            } else {
                selected.len() as f64 / test_files as f64
            },
        },
        selected,
        skipped,
        escalation: escalation.map(|tier| Escalation { tier, reasons }),
        confidence: Confidence {
            graph_files: inputs.graph.files.len(),
            graph_edges: inputs.graph.edges.len(),
            unresolved: inputs.graph.unresolved.len(),
            parse_errors,
        },
    }
}

/// Every path-convention test file under `path`, through `parse::discovery`.
fn test_files(path: &Path, config: &TiaConfig, excludes: &[String]) -> Result<BTreeSet<String>> {
    let root = analysis_root(path);
    Ok(parse::discovery::discover_with_excludes(path, excludes)?
        .iter()
        .map(|file| parse::normalized_relative_path(file, root))
        .filter(|relative| is_test_file(relative, config))
        .collect())
}

/// The root the graph and discovery are scoped to: the directory itself, or a
/// file's parent. Mirrors `analysis::graph`'s own rule so paths agree.
fn analysis_root(path: &Path) -> &Path {
    if path.is_file() {
        path.parent().unwrap_or(Path::new("."))
    } else {
        path
    }
}

/// True when `path`'s extension marks it as documentation or data, matched
/// case-insensitively the way `detect_language` matches its own.
fn is_documentation_or_data(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            NON_BEHAVIORAL_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// True when the file parses with errors, which means it can only escalate.
fn is_parse_error(path: &str, source_of: &dyn Fn(&str) -> Option<Vec<u8>>) -> bool {
    source_of(path)
        .and_then(|source| parse::analyze_source(path, &source).ok())
        .is_some_and(|file| !file.parse_errors.is_empty())
}

/// True when `path` is a test file: by path convention, or — for a closure file
/// the path cannot name — by the tests it declares.
fn is_test_candidate(path: &str, inputs: &SelectionInputs<'_>) -> bool {
    if inputs.test_files.contains(path) {
        return true;
    }
    if !accepts_candidate(path, inputs.config) {
        return false;
    }
    let Some(language) = detect_language(path) else {
        return false;
    };
    (inputs.source_of)(path).is_some_and(|source| declares_tests(&source, language))
}

/// Records one test file, keeping the strongest tier and the union of the
/// changed files that reached it.
fn record(
    selected: &mut BTreeMap<String, (Tier, BTreeSet<String>)>,
    test: &str,
    tier: Tier,
    via: &str,
) {
    let entry = selected
        .entry(test.to_owned())
        .or_insert((tier, BTreeSet::new()));
    entry.0 = entry.0.max(tier);
    entry.1.insert(via.to_owned());
}

/// T3: every test file under `path`'s top-level directory. A file at the root
/// has the whole repository as its top-level directory.
fn escalate_directory(
    path: &str,
    test_files: &BTreeSet<String>,
    selected: &mut BTreeMap<String, (Tier, BTreeSet<String>)>,
) {
    let prefix = path.split_once('/').map(|(top, _)| format!("{top}/"));
    for test in test_files {
        if prefix
            .as_ref()
            .is_none_or(|prefix| test.starts_with(prefix))
        {
            record(selected, test, Tier::Directory, path);
        }
    }
}

fn strongest(current: Option<Tier>, tier: Tier) -> Option<Tier> {
    Some(current.map_or(tier, |current| current.max(tier)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use analysis::graph::analyze_dependencies_from_sources;
    use parse::source_snapshot::SourceEntry;

    fn sources(pairs: &[(&str, &str)]) -> Vec<SourceEntry> {
        pairs
            .iter()
            .map(|(path, source)| SourceEntry {
                path: (*path).to_owned(),
                bytes: source.as_bytes().to_vec(),
            })
            .collect()
    }

    fn bytes(pairs: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(path, source)| ((*path).to_owned(), source.as_bytes().to_vec()))
            .collect()
    }

    fn changed(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    fn select_over(
        config: &TiaConfig,
        changed: &BTreeSet<String>,
        entries: &[SourceEntry],
        test_files: &[&str],
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Selection {
        let graph = analyze_dependencies_from_sources(entries).expect("fixture graph resolves");
        let test_files: BTreeSet<String> =
            test_files.iter().map(|path| (*path).to_owned()).collect();
        let source_of = |path: &str| files.get(path).cloned();
        select_from(&SelectionInputs {
            base: "HEAD~1",
            config,
            changed,
            graph: &graph,
            test_files: &test_files,
            source_of: &source_of,
        })
    }

    fn selected_paths(selection: &Selection) -> Vec<&str> {
        selection
            .selected
            .iter()
            .map(|test| test.path.as_str())
            .collect()
    }

    #[test]
    fn graph_tier_selects_the_whole_transitive_closure_untruncated() {
        // Forty direct dependents and a chain behind one of them: a display
        // limit would have cut the list, and a display walk per changed file
        // would have walked the graph forty times.
        let mut pairs: Vec<(String, String)> = vec![(
            "src/base.ts".to_owned(),
            "export const base = 1;\n".to_owned(),
        )];
        let mut test_paths: Vec<String> = Vec::new();
        for index in 0..40 {
            let path = format!("web/leaf{index}.test.ts");
            pairs.push((
                path.clone(),
                "import { base } from '../src/base';\nexport const v = base;\n".to_owned(),
            ));
            test_paths.push(path);
        }
        pairs.push((
            "src/mid.ts".to_owned(),
            "import { base } from './base';\nexport const mid = base;\n".to_owned(),
        ));
        pairs.push((
            "web/deep.test.ts".to_owned(),
            "import { mid } from '../src/mid';\nexport const v = mid;\n".to_owned(),
        ));
        test_paths.push("web/deep.test.ts".to_owned());

        let owned: Vec<(String, String)> = pairs;
        let borrowed: Vec<(&str, &str)> = owned
            .iter()
            .map(|(path, source)| (path.as_str(), source.as_str()))
            .collect();
        let entries = sources(&borrowed);
        let files = bytes(&borrowed);
        let test_paths: Vec<&str> = test_paths.iter().map(String::as_str).collect();

        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["src/base.ts"]),
            &entries,
            &test_paths,
            &files,
        );

        assert_eq!(selection.selected.len(), 41, "{:?}", selection.selected);
        assert_eq!(selection.totals.selected, 41);
        for test in &selection.selected {
            assert_eq!(test.tier, Tier::Graph);
            assert_eq!(test.via, changed(&["src/base.ts"]));
        }
        assert_eq!(selection.escalation, None);
    }

    #[test]
    fn a_diamond_dependency_is_recorded_once() {
        let pairs = [
            ("src/base.ts", "export const base = 1;\n"),
            (
                "src/left.ts",
                "import { base } from './base';\nexport const left = base;\n",
            ),
            (
                "src/right.ts",
                "import { base } from './base';\nexport const right = base;\n",
            ),
            (
                "src/top.ts",
                "import { left } from './left';\nimport { right } from './right';\nexport const top = left + right;\n",
            ),
            (
                "tests/top.test.ts",
                "import { top } from '../src/top';\nexport const v = top;\n",
            ),
        ];
        let entries = sources(&pairs);
        let files = bytes(&pairs);

        let one_source = select_over(
            &TiaConfig::default(),
            &changed(&["src/base.ts"]),
            &entries,
            &["tests/top.test.ts"],
            &files,
        );
        assert_eq!(selected_paths(&one_source), ["tests/top.test.ts"]);
        assert_eq!(one_source.selected[0].via, changed(&["src/base.ts"]));

        // Two changed files above the diamond still produce one entry, with
        // both causes recorded.
        let two_sources = select_over(
            &TiaConfig::default(),
            &changed(&["src/left.ts", "src/right.ts"]),
            &entries,
            &["tests/top.test.ts"],
            &files,
        );
        assert_eq!(selected_paths(&two_sources), ["tests/top.test.ts"]);
        assert_eq!(
            two_sources.selected[0].via,
            changed(&["src/left.ts", "src/right.ts"])
        );
    }

    #[test]
    fn an_unsupported_extension_escalates_to_the_top_level_directory() {
        let pairs = [
            ("src/a.ts", "export const a = 1;\n"),
            ("docs/docs.test.ts", "export const doc = 1;\n"),
            ("service/service.test.ts", "export const service = 1;\n"),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["docs/notes.hbs"]),
            &sources(&pairs),
            &["docs/docs.test.ts", "service/service.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(selected_paths(&selection), ["docs/docs.test.ts"]);
        assert_eq!(selection.selected[0].tier, Tier::Directory);
        assert_eq!(selection.selected[0].via, changed(&["docs/notes.hbs"]));
        assert!(selection.skipped.is_empty());
        assert_eq!(
            selection.escalation,
            Some(Escalation {
                tier: Tier::Directory,
                reasons: vec!["unsupported extension: docs/notes.hbs".to_owned()],
            })
        );
    }

    #[test]
    fn a_parse_error_escalates_and_is_counted() {
        let pairs = [
            ("src/broken.ts", "function {\n"),
            ("src/other.test.ts", "export const other = 1;\n"),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["src/broken.ts"]),
            &sources(&pairs),
            &["src/other.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(selected_paths(&selection), ["src/other.test.ts"]);
        assert_eq!(selection.selected[0].tier, Tier::Directory);
        assert_eq!(
            selection.escalation.as_ref().map(|it| it.reasons.clone()),
            Some(vec!["parse error: src/broken.ts".to_owned()])
        );
        assert_eq!(selection.confidence.parse_errors, 1);
    }

    #[test]
    fn a_changed_file_absent_from_the_graph_escalates() {
        let pairs = [
            ("src/a.ts", "export const a = 1;\n"),
            ("tools/generate.test.ts", "export const tool = 1;\n"),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["tools/generate.ts"]),
            &sources(&pairs),
            &["tools/generate.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(selected_paths(&selection), ["tools/generate.test.ts"]);
        assert_eq!(
            selection.escalation.as_ref().map(|it| it.reasons.clone()),
            Some(vec!["not in graph: tools/generate.ts".to_owned()])
        );
    }

    #[test]
    fn a_documentation_change_selects_nothing_and_does_not_escalate() {
        let pairs = [
            ("src/a.ts", "export const a = 1;\n"),
            (
                "src/a.test.ts",
                "import './a';\nexport const checked = 1;\n",
            ),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["README.md"]),
            &sources(&pairs),
            &["src/a.test.ts"],
            &bytes(&pairs),
        );

        assert!(selected_paths(&selection).is_empty());
        assert_eq!(selection.escalation, None);
        assert_eq!(
            selection.skipped,
            vec![SkippedFile {
                path: "README.md".to_owned(),
                reason: "documentation".to_owned(),
            }]
        );
    }

    #[test]
    fn an_image_or_a_text_change_is_skipped_the_same_way() {
        let pairs = [
            ("src/a.ts", "export const a = 1;\n"),
            (
                "src/a.test.ts",
                "import './a';\nexport const checked = 1;\n",
            ),
        ];
        for path in ["assets/logo.png", "GUIDE.MD", "docs/notes.txt"] {
            let selection = select_over(
                &TiaConfig::default(),
                &changed(&[path]),
                &sources(&pairs),
                &["src/a.test.ts"],
                &bytes(&pairs),
            );
            assert!(selected_paths(&selection).is_empty(), "{path}");
            assert_eq!(selection.escalation, None, "{path}");
            assert_eq!(
                selection.skipped,
                vec![SkippedFile {
                    path: path.to_owned(),
                    reason: "documentation".to_owned(),
                }],
                "{path}"
            );
        }
    }

    #[test]
    fn an_unknown_code_extension_still_escalates() {
        // The regression guard: the skip narrows one case without weakening
        // the unsupported-extension escalation `.hbs` and `.proto` rely on.
        let pairs = [
            ("src/a.ts", "export const a = 1;\n"),
            (
                "src/a.test.ts",
                "import './a';\nexport const checked = 1;\n",
            ),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["src/page.hbs"]),
            &sources(&pairs),
            &["src/a.test.ts"],
            &bytes(&pairs),
        );

        assert!(selection.skipped.is_empty());
        assert_eq!(selected_paths(&selection), ["src/a.test.ts"]);
        assert_eq!(
            selection.escalation.as_ref().map(|it| it.reasons.clone()),
            Some(vec!["unsupported extension: src/page.hbs".to_owned()])
        );
    }

    #[test]
    fn always_run_overrides_the_documentation_skip() {
        let pairs = [
            ("src/a.test.ts", "export const a = 1;\n"),
            ("web/b.test.ts", "export const b = 1;\n"),
        ];
        let config = TiaConfig {
            always_run: vec!["**/*.md".to_owned()],
            ..TiaConfig::default()
        };
        let selection = select_over(
            &config,
            &changed(&["docs/effect.md"]),
            &sources(&pairs),
            &["src/a.test.ts", "web/b.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(
            selected_paths(&selection),
            ["src/a.test.ts", "web/b.test.ts"]
        );
        assert!(selection.selected.iter().all(|test| test.tier == Tier::All));
        assert!(selection.skipped.is_empty());
        assert_eq!(
            selection.escalation,
            Some(Escalation {
                tier: Tier::All,
                reasons: vec!["always run: docs/effect.md".to_owned()],
            })
        );
    }

    #[test]
    fn a_readme_only_change_does_not_report_no_test_found() {
        // `no test found` catches a hole in the conventions, not a deliberate
        // skip: a skipped file never becomes a seed, so the coverage check
        // cannot reach it.
        let pairs = [("tests/a.test.ts", "export const checked = 1;\n")];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["README.md"]),
            &sources(&pairs),
            &["tests/a.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(selection.escalation, None);
        assert_eq!(selection.totals.test_files, 1);
        assert_eq!(selection.totals.selected, 0);
    }

    #[test]
    fn always_run_selects_every_test_file() {
        let pairs = [
            ("src/a.test.ts", "export const a = 1;\n"),
            ("web/b.test.ts", "export const b = 1;\n"),
        ];
        let config = TiaConfig {
            always_run: vec!["tsconfig*.json".to_owned(), "*.lock".to_owned()],
            ..TiaConfig::default()
        };
        let selection = select_over(
            &config,
            &changed(&["tsconfig.json"]),
            &sources(&pairs),
            &["src/a.test.ts", "web/b.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(
            selected_paths(&selection),
            ["src/a.test.ts", "web/b.test.ts"]
        );
        assert!(selection.selected.iter().all(|test| test.tier == Tier::All));
        assert_eq!(
            selection.escalation,
            Some(Escalation {
                tier: Tier::All,
                reasons: vec!["always run: tsconfig.json".to_owned()],
            })
        );
        assert_eq!(selection.totals.test_files, 2);
        assert_eq!(selection.totals.ratio, 1.0);
    }

    #[test]
    fn a_changed_file_that_reaches_no_test_escalates() {
        let pairs = [
            ("src/lonely.ts", "export const lonely = 1;\n"),
            ("src/other.test.ts", "export const other = 1;\n"),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["src/lonely.ts"]),
            &sources(&pairs),
            &["src/other.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(selected_paths(&selection), ["src/other.test.ts"]);
        assert_eq!(selection.selected[0].tier, Tier::Directory);
        assert_eq!(
            selection.escalation.as_ref().map(|it| it.reasons.clone()),
            Some(vec!["no test found: src/lonely.ts".to_owned()])
        );
    }

    #[test]
    fn inline_tests_are_found_through_the_closure() {
        let pairs = [
            (
                "src/lib.rs",
                "mod parser;\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn it_works() {}\n}\n",
            ),
            ("src/parser.rs", "pub fn parse() {}\n"),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["src/parser.rs"]),
            &sources(&pairs),
            &[],
            &bytes(&pairs),
        );

        // `mod parser;` is an edge from the declaring file to the module, so
        // the declaring file is the closure of the changed one, and it
        // declares tests the path cannot name.
        assert_eq!(selected_paths(&selection), ["src/lib.rs"]);
        assert_eq!(selection.selected[0].tier, Tier::Graph);
        assert_eq!(selection.selected[0].via, changed(&["src/parser.rs"]));
        // The denominator absorbs the content-detected file, so it never
        // falls below the numerator.
        assert_eq!(selection.totals.test_files, 1);
        assert_eq!(selection.totals.ratio, 1.0);
        assert_eq!(selection.escalation, None);
    }

    #[test]
    fn a_content_detected_test_file_joins_the_denominator() {
        // One path-convention test file and one selected because its content
        // declares tests: the denominator is their union, so a repository that
        // discovers an inline-test file reports 2 of 2 rather than 2 of 1 with
        // a ratio above 1.
        let pairs = [
            (
                "src/lib.rs",
                "#[cfg(test)]\nmod tests {\n    #[test]\n    fn it_works() {}\n}\n",
            ),
            ("src/other.test.ts", "export const other = 1;\n"),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["src/lib.rs", "src/other.test.ts"]),
            &sources(&pairs),
            &["src/other.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(
            selected_paths(&selection),
            ["src/lib.rs", "src/other.test.ts"]
        );
        assert_eq!(selection.totals.selected, 2);
        assert_eq!(selection.totals.test_files, 2);
        assert!(selection.totals.ratio <= 1.0);
        assert_eq!(selection.totals.ratio, 1.0);
    }

    #[test]
    fn a_changed_file_with_inline_tests_selects_itself() {
        let pairs = [(
            "src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn it_works() {}\n}\n",
        )];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["src/lib.rs"]),
            &sources(&pairs),
            &[],
            &bytes(&pairs),
        );

        assert_eq!(selected_paths(&selection), ["src/lib.rs"]);
        assert_eq!(selection.selected[0].tier, Tier::Graph);
        assert_eq!(selection.selected[0].via, changed(&["src/lib.rs"]));
        assert_eq!(selection.escalation, None);
    }

    #[test]
    fn the_strongest_tier_and_every_cause_are_reported_once() {
        // One test file reached by a T2 closure and by a T3 directory
        // escalation: one entry, the stronger tier, both causes.
        let pairs = [
            ("src/base.ts", "export const base = 1;\n"),
            (
                "src/uses.test.ts",
                "import { base } from './base';\nexport const v = base;\n",
            ),
        ];
        let selection = select_over(
            &TiaConfig::default(),
            &changed(&["src/base.ts", "src/notes.hbs"]),
            &sources(&pairs),
            &["src/uses.test.ts"],
            &bytes(&pairs),
        );

        assert_eq!(selected_paths(&selection), ["src/uses.test.ts"]);
        assert_eq!(selection.selected[0].tier, Tier::Directory);
        assert_eq!(
            selection.selected[0].via,
            changed(&["src/base.ts", "src/notes.hbs"])
        );
        assert_eq!(selection.totals.test_files, 1);
        assert_eq!(selection.confidence.graph_files, 2);
        assert_eq!(selection.confidence.graph_edges, 1);
    }
}
