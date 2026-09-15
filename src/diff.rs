use crate::Result;
use crate::config::{RegressionLimits, Thresholds};
use crate::core::{
    FunctionAnalysis, METRIC_PROFILE, MetricSpecs, OUTPUT_SCHEMA_VERSION, ParseDiagnostic,
};
use crate::git;
use crate::parser::detect_language;
use crate::source_snapshot::SourceEntry;
use crate::strip_verbatim_prefix;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FunctionChange {
    pub path: String,
    pub name: String,
    pub before: Option<FunctionAnalysis>,
    pub after: Option<FunctionAnalysis>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChangedParseDiagnostics {
    pub path: String,
    pub before: Vec<ParseDiagnostic>,
    pub after: Vec<ParseDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChangedReport {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub metric_specs: MetricSpecs,
    pub base: String,
    pub functions: Vec<FunctionChange>,
    pub parse_errors: Vec<ChangedParseDiagnostics>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComparisonTarget {
    Worktree,
    Index,
    Revision(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangeOptions {
    pub base: String,
    pub target: ComparisonTarget,
    pub detect_renames: bool,
}

/// Returns which metric deltas exceed their configured limits, in gate order.
pub fn regression_dimensions(
    before: &FunctionAnalysis,
    after: &FunctionAnalysis,
    limits: &RegressionLimits,
) -> [bool; 4] {
    [
        after
            .metrics
            .cognitive
            .saturating_sub(before.metrics.cognitive)
            > limits.cognitive,
        after
            .metrics
            .cyclomatic
            .saturating_sub(before.metrics.cyclomatic)
            > limits.cyclomatic,
        match (before.metrics.crap, after.metrics.crap) {
            (Some(before), Some(after)) if before.is_finite() && after.is_finite() => {
                after - before > limits.crap
            }
            _ => false,
        },
        after
            .metrics
            .max_nesting
            .saturating_sub(before.metrics.max_nesting)
            > limits.max_nesting,
    ]
}

/// Returns true when a paired function exceeds any allowed positive metric delta.
pub fn regression_violates(
    before: &FunctionAnalysis,
    after: &FunctionAnalysis,
    limits: &RegressionLimits,
) -> bool {
    regression_dimensions(before, after, limits)
        .into_iter()
        .any(|value| value)
}

/// Changed paired functions that exceed regression limits, in deterministic report order.
pub fn changed_regressions<'a>(
    report: &'a ChangedReport,
    limits: &RegressionLimits,
) -> Vec<&'a FunctionChange> {
    report
        .functions
        .iter()
        .filter(|change| {
            change
                .before
                .as_ref()
                .zip(change.after.as_ref())
                .is_some_and(|(before, after)| regression_violates(before, after, limits))
        })
        .collect()
}

// ponytail: mirrors core::Thresholds::violates; unify the two Thresholds types if this drifts.
pub fn changed_violations<'a>(
    report: &'a ChangedReport,
    thresholds: &Thresholds,
) -> Vec<&'a FunctionChange> {
    report
        .functions
        .iter()
        .filter(|change| {
            change.after.as_ref().is_some_and(|after| {
                thresholds
                    .cognitive
                    .is_some_and(|limit| after.metrics.cognitive > limit)
                    || thresholds
                        .cyclomatic
                        .is_some_and(|limit| after.metrics.cyclomatic > limit)
                    || thresholds
                        .max_nesting
                        .is_some_and(|limit| after.metrics.max_nesting > limit)
                    || thresholds
                        .crap
                        .is_some_and(|limit| after.metrics.crap.is_none_or(|value| value > limit))
            })
        })
        .collect()
}

/// Compatibility wrapper: worktree comparison, no rename detection.
pub fn analyze_changed(path: &Path, base: &str) -> Result<ChangedReport> {
    analyze_changes(
        path,
        &ChangeOptions {
            base: base.to_owned(),
            target: ComparisonTarget::Worktree,
            detect_renames: false,
        },
    )
}

/// After-side source entries for every changed supported file.
///
/// Unlike [`analyze_changes`], nothing is parsed: files without functions
/// still yield entries. Entries sort by path for deterministic output.
pub fn changed_source_entries(
    path: &Path,
    options: &ChangeOptions,
) -> crate::Result<Vec<SourceEntry>> {
    let changed = changed_paths_in_scope(path, options)?;
    let mut entries = Vec::new();
    for relative in changed.paths {
        if detect_language(&relative).is_none() {
            continue;
        }
        if let Some(bytes) = read_after(&changed.root, &relative, &options.target)? {
            entries.push(SourceEntry {
                path: relative,
                bytes,
            });
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

/// Every changed path in scope, supported source or not, relative to the
/// analysis root (the directory, or the file's parent directory).
///
/// Attribution inputs (security, secret gating) must see `.env`, YAML, JSON,
/// and other non-source changes; source filtering belongs to parsers, not to
/// Git state. Renames resolve to their new path through the same rename map
/// as [`analyze_changes`], and deleted paths stay in the set. Paths match the
/// artifact and project paths of the same root, so a subdirectory run
/// attributes `src/app.py` as `app.py`.
pub fn changed_paths(path: &Path, options: &ChangeOptions) -> crate::Result<BTreeSet<String>> {
    let changed = changed_paths_in_scope(path, options)?;
    if changed.base.is_empty() {
        return Ok(changed.paths);
    }
    let prefix = format!("{}/", changed.base);
    Ok(changed
        .paths
        .into_iter()
        .map(|candidate| {
            candidate
                .strip_prefix(&prefix)
                .map_or(candidate.clone(), str::to_owned)
        })
        .collect())
}

/// One enumeration result: the Git root, the analysis-root prefix stripped
/// from attribution paths, and the in-scope changed paths.
struct ChangedPaths {
    root: PathBuf,
    base: String,
    paths: BTreeSet<String>,
    renames: BTreeMap<String, String>,
}

/// Shared path enumeration with [`analyze_changes`]: same revision
/// validation, scope filtering, rename handling, and untracked pickup for
/// worktree targets. Paths stay Git-root-relative here; [`changed_paths`]
/// rebases them.
fn changed_paths_in_scope(path: &Path, options: &ChangeOptions) -> crate::Result<ChangedPaths> {
    git::validate_revision(&options.base)?;
    if let ComparisonTarget::Revision(target) = &options.target {
        git::validate_revision(target)?;
    }
    let (requested, start) =
        resolve_requested(path, !matches!(options.target, ComparisonTarget::Worktree))?;
    let root_output = git::run(&start, &["rev-parse", "--show-toplevel"])?;
    let toplevel = String::from_utf8(root_output.stdout)?;
    let root = strip_verbatim_prefix(&std::fs::canonicalize(toplevel.trim())?);
    let scope = crate::normalized_relative_path(&requested, &root);
    let base = if requested.is_dir() {
        scope.clone()
    } else {
        scope
            .rsplit_once('/')
            .map_or_else(String::new, |(parent, _)| parent.to_owned())
    };
    let (mut paths, renames) = diff_paths(&root, options)?;
    if matches!(options.target, ComparisonTarget::Worktree) {
        paths.extend(untracked_paths(&root)?);
    }
    if !scope.is_empty() {
        let prefix = format!("{scope}/");
        paths.retain(|candidate| candidate == &scope || candidate.starts_with(&prefix));
    }
    Ok(ChangedPaths {
        root,
        base,
        paths,
        renames,
    })
}

pub fn analyze_changes(path: &Path, options: &ChangeOptions) -> Result<ChangedReport> {
    let ChangedPaths {
        root,
        paths,
        renames,
        ..
    } = changed_paths_in_scope(path, options)?;
    let mut functions = Vec::new();
    let mut parse_errors = Vec::new();
    for relative in paths {
        if detect_language(&relative).is_none() {
            continue;
        }
        let before_path = renames
            .get(&relative)
            .map_or(relative.as_str(), String::as_str);
        let before = git::read_revision_blob_optional(&root, &options.base, before_path)?
            .map(|source| crate::analyze_source(before_path, &source))
            .transpose()?;
        let after = read_after(&root, &relative, &options.target)?
            .map(|source| crate::analyze_source(&relative, &source))
            .transpose()?;
        let before_errors = before
            .as_ref()
            .map(|file| file.parse_errors.clone())
            .unwrap_or_default();
        let after_errors = after
            .as_ref()
            .map(|file| file.parse_errors.clone())
            .unwrap_or_default();
        if !before_errors.is_empty() || !after_errors.is_empty() {
            parse_errors.push(ChangedParseDiagnostics {
                path: relative.clone(),
                before: before_errors,
                after: after_errors,
            });
        }
        functions.extend(pair_functions(
            &relative,
            before.map(|file| file.functions).unwrap_or_default(),
            after.map(|file| file.functions).unwrap_or_default(),
        ));
    }
    functions.sort_by(|left, right| {
        left.path.cmp(&right.path).then_with(|| {
            let left_line = left
                .after
                .as_ref()
                .or(left.before.as_ref())
                .map_or(0, |function| function.start_line);
            let right_line = right
                .after
                .as_ref()
                .or(right.before.as_ref())
                .map_or(0, |function| function.start_line);
            left_line.cmp(&right_line)
        })
    });
    Ok(ChangedReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        metric_specs: MetricSpecs::default(),
        base: options.base.clone(),
        functions,
        parse_errors,
    })
}

fn resolve_requested(path: &Path, allow_missing: bool) -> Result<(PathBuf, PathBuf)> {
    if !allow_missing {
        let requested = strip_verbatim_prefix(&std::fs::canonicalize(path)?);
        let start = if requested.is_dir() {
            requested.clone()
        } else {
            requested.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        return Ok((requested, start));
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let ancestor = absolute
        .ancestors()
        .find(|candidate| candidate.exists())
        .ok_or("path has no existing ancestor")?;
    let resolved_ancestor = strip_verbatim_prefix(&std::fs::canonicalize(ancestor)?);
    let requested = resolved_ancestor.join(absolute.strip_prefix(ancestor)?);
    let start = if resolved_ancestor.is_dir() {
        resolved_ancestor
    } else {
        resolved_ancestor
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf()
    };
    Ok((requested, start))
}

/// Reads after-side content per target: worktree file, staged (index) blob,
/// or a blob from the comparison revision. Revision never touches the
/// worktree or index, so a dirty tree cannot leak into that comparison.
fn read_after(root: &Path, relative: &str, target: &ComparisonTarget) -> Result<Option<Vec<u8>>> {
    match target {
        ComparisonTarget::Worktree => {
            let after_path = root.join(relative);
            after_path
                .is_file()
                .then(|| std::fs::read(&after_path))
                .transpose()
                .map_err(Into::into)
        }
        ComparisonTarget::Index => git::read_index_blob_optional(root, relative),
        ComparisonTarget::Revision(revision) => {
            git::read_revision_blob_optional(root, revision, relative)
        }
    }
}

fn diff_paths(
    root: &Path,
    options: &ChangeOptions,
) -> Result<(BTreeSet<String>, BTreeMap<String, String>)> {
    let mut args: Vec<String> = vec!["diff".to_owned()];
    args.push(if options.detect_renames {
        "-M".to_owned()
    } else {
        "--no-renames".to_owned()
    });
    args.push("--name-status".to_owned());
    args.push("-z".to_owned());
    if matches!(options.target, ComparisonTarget::Index) {
        args.push("--cached".to_owned());
    }
    args.push(options.base.clone());
    if let ComparisonTarget::Revision(revision) = &options.target {
        args.push(revision.clone());
    }
    args.push("--".to_owned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = git::run(root, &refs)?;
    parse_name_status(&output.stdout)
}

/// Parses `git diff --name-status -z` output. A rename record is
/// `R<score>\0old\0new`; every other status is `<status>\0path`. Only the
/// new path enters the returned path set, so scope filtering and output
/// naturally land on the renamed file's current location.
fn parse_name_status(bytes: &[u8]) -> Result<(BTreeSet<String>, BTreeMap<String, String>)> {
    let fields: Vec<String> = bytes
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| Ok(std::str::from_utf8(value)?.to_owned()))
        .collect::<Result<_>>()?;
    let mut paths = BTreeSet::new();
    let mut renames = BTreeMap::new();
    let mut index = 0;
    while index < fields.len() {
        if fields[index].starts_with('R') {
            let old = fields.get(index + 1).ok_or("malformed rename status")?;
            let new = fields.get(index + 2).ok_or("malformed rename status")?;
            renames.insert(new.clone(), old.clone());
            paths.insert(new.clone());
            index += 3;
        } else {
            let path = fields.get(index + 1).ok_or("malformed status")?;
            paths.insert(path.clone());
            index += 2;
        }
    }
    Ok((paths, renames))
}

fn untracked_paths(root: &Path) -> Result<BTreeSet<String>> {
    let output = git::run(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    nul_paths(&output.stdout)
}

fn nul_paths(bytes: &[u8]) -> Result<BTreeSet<String>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| Ok(std::str::from_utf8(value)?.to_owned()))
        .collect()
}

fn pair_functions(
    path: &str,
    before: Vec<FunctionAnalysis>,
    after: Vec<FunctionAnalysis>,
) -> Vec<FunctionChange> {
    let mut before_by_name = group_by_name(before);
    let mut after_by_name = group_by_name(after);
    let names: BTreeSet<_> = before_by_name
        .keys()
        .chain(after_by_name.keys())
        .cloned()
        .collect();
    let mut changes = Vec::new();
    for name in names {
        let before = before_by_name.remove(&name).unwrap_or_default();
        let after = after_by_name.remove(&name).unwrap_or_default();
        let count = before.len().max(after.len());
        for index in 0..count {
            let previous = before.get(index).cloned();
            let current = after.get(index).cloned();
            if previous.as_ref().map(|value| value.source_fingerprint)
                == current.as_ref().map(|value| value.source_fingerprint)
            {
                continue;
            }
            changes.push(FunctionChange {
                path: path.to_owned(),
                name: name.clone(),
                before: previous,
                after: current,
            });
        }
    }
    changes
}

fn group_by_name(functions: Vec<FunctionAnalysis>) -> BTreeMap<String, Vec<FunctionAnalysis>> {
    let mut grouped = BTreeMap::new();
    for function in functions {
        grouped
            .entry(function.name.clone())
            .or_insert_with(Vec::new)
            .push(function);
    }
    grouped
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn verbatim_disk_prefix_strips_to_git_form() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\C:\repo\calc.ts")),
            PathBuf::from(r"C:\repo\calc.ts")
        );
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_unc_prefix_strips_to_git_form() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\host\share\calc.ts")),
            PathBuf::from(r"\\host\share\calc.ts")
        );
    }

    #[cfg(windows)]
    #[test]
    fn plain_path_is_unchanged() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"C:\repo\calc.ts")),
            PathBuf::from(r"C:\repo\calc.ts")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_path_is_unchanged() {
        assert_eq!(
            strip_verbatim_prefix(Path::new("/tmp/repo/calc.ts")),
            PathBuf::from("/tmp/repo/calc.ts")
        );
    }

    use crate::core::{FunctionKind, FunctionMetrics};

    fn test_analysis(name: &str, cognitive: u32) -> FunctionAnalysis {
        FunctionAnalysis {
            name: name.to_owned(),
            id: format!("src/a.ts:function:{name}"),
            kind: FunctionKind::Function,
            start_line: 1,
            end_line: 10,
            start_byte: 0,
            end_byte: 100,
            metrics: FunctionMetrics {
                loc: 10,
                logical_loc: 10,
                function_length: 10,
                parameters: 0,
                max_nesting: 0,
                cyclomatic: 1,
                cognitive,
                halstead_n1: 1,
                halstead_n2: 1,
                halstead_total_operators: 1,
                halstead_total_operands: 1,
                halstead_vocabulary: 2,
                halstead_length: 2,
                halstead_volume: 2.0,
                halstead_difficulty: 0.5,
                halstead_effort: 1.0,
                maintainability_index: 100.0,
                coverage: None,
                crap: None,
            },
            contributions: Vec::new(),
            source_fingerprint: 1,
        }
    }

    fn test_change(
        name: &str,
        before: Option<FunctionAnalysis>,
        after: Option<FunctionAnalysis>,
    ) -> FunctionChange {
        FunctionChange {
            path: "src/a.ts".to_owned(),
            name: name.to_owned(),
            before,
            after,
        }
    }

    fn test_report(changes: Vec<FunctionChange>) -> ChangedReport {
        ChangedReport {
            schema_version: OUTPUT_SCHEMA_VERSION,
            metric_profile: METRIC_PROFILE,
            analyzer_version: "test",
            metric_specs: MetricSpecs::default(),
            base: "base".to_owned(),
            functions: changes,
            parse_errors: Vec::new(),
        }
    }

    fn cognitive_gate() -> Thresholds {
        Thresholds {
            cognitive: Some(15),
            ..Thresholds::default()
        }
    }

    #[test]
    fn added_violating_function_is_kept() {
        let report = test_report(vec![test_change(
            "new",
            None,
            Some(test_analysis("new", 20)),
        )]);
        assert_eq!(changed_violations(&report, &cognitive_gate()).len(), 1);
    }

    #[test]
    fn fixed_function_is_dropped() {
        let report = test_report(vec![test_change(
            "fixed",
            Some(test_analysis("fixed", 20)),
            Some(test_analysis("fixed", 5)),
        )]);
        assert!(changed_violations(&report, &cognitive_gate()).is_empty());
    }

    #[test]
    fn removed_function_is_dropped() {
        let report = test_report(vec![test_change(
            "gone",
            Some(test_analysis("gone", 20)),
            None,
        )]);
        assert!(changed_violations(&report, &cognitive_gate()).is_empty());
    }

    #[test]
    fn improved_but_still_over_gate_is_kept() {
        let report = test_report(vec![test_change(
            "better",
            Some(test_analysis("better", 30)),
            Some(test_analysis("better", 20)),
        )]);
        assert_eq!(changed_violations(&report, &cognitive_gate()).len(), 1);
    }

    #[test]
    fn empty_thresholds_keep_nothing() {
        let report = test_report(vec![test_change(
            "new",
            None,
            Some(test_analysis("new", 20)),
        )]);
        assert!(changed_violations(&report, &Thresholds::default()).is_empty());
    }
}
