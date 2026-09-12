use crate::Result;
use crate::config::Thresholds;
use crate::core::{
    FunctionAnalysis, METRIC_PROFILE, MetricSpecs, OUTPUT_SCHEMA_VERSION, ParseDiagnostic,
};
use crate::parser::detect_language;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

pub fn analyze_changed(path: &Path, base: &str) -> Result<ChangedReport> {
    validate_revision(base)?;
    let requested = strip_verbatim_prefix(&std::fs::canonicalize(path)?);
    let start = if requested.is_dir() {
        requested.as_path()
    } else {
        requested.parent().unwrap_or(Path::new("."))
    };
    let root_output = git(start, ["rev-parse", "--show-toplevel"])?;
    let toplevel = String::from_utf8(root_output.stdout)?;
    let root = strip_verbatim_prefix(&std::fs::canonicalize(toplevel.trim())?);
    let scope = crate::normalized_relative_path(&requested, &root);
    let mut paths = changed_paths(&root, base)?;
    paths.extend(untracked_paths(&root)?);
    if !scope.is_empty() {
        let prefix = format!("{scope}/");
        paths.retain(|candidate| candidate == &scope || candidate.starts_with(&prefix));
    }

    let mut functions = Vec::new();
    let mut parse_errors = Vec::new();
    for relative in paths {
        if detect_language(&relative).is_none() {
            continue;
        }
        let before = git_optional(&root, ["show", &format!("{base}:{relative}")])?
            .map(|source| crate::analyze_source(&relative, &source))
            .transpose()?;
        let after_path = root.join(&relative);
        let after = after_path
            .is_file()
            .then(|| std::fs::read(&after_path))
            .transpose()?
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
        base: base.to_owned(),
        functions,
        parse_errors,
    })
}

fn changed_paths(root: &Path, base: &str) -> Result<BTreeSet<String>> {
    let output = git(
        root,
        ["diff", "--no-renames", "--name-only", "-z", base, "--"],
    )?;
    nul_paths(&output.stdout)
}

fn untracked_paths(root: &Path) -> Result<BTreeSet<String>> {
    let output = git(root, ["ls-files", "--others", "--exclude-standard", "-z"])?;
    nul_paths(&output.stdout)
}

fn nul_paths(bytes: &[u8]) -> Result<BTreeSet<String>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| Ok(std::str::from_utf8(value)?.to_owned()))
        .collect()
}

fn validate_revision(base: &str) -> Result<()> {
    if base.is_empty()
        || base.starts_with('-')
        || base.contains(':')
        || base.chars().any(char::is_control)
    {
        return Err("base revision contains unsupported characters".into());
    }
    Ok(())
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
/// `std::fs::canonicalize` returns verbatim (`\\?\`) paths on Windows, which
/// never match the plain paths git prints. Both the requested path and the
/// git-reported root pass through canonicalization plus this strip, so scope
/// comparison always compares like with like. Identity off Windows.
#[cfg(windows)]
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
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
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn git<'a>(cwd: &Path, args: impl IntoIterator<Item = &'a str>) -> Result<Output> {
    let output = Command::new("git").current_dir(cwd).args(args).output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_owned()
            .into())
    }
}

fn git_optional<'a>(
    cwd: &Path,
    args: impl IntoIterator<Item = &'a str>,
) -> Result<Option<Vec<u8>>> {
    let output = Command::new("git").current_dir(cwd).args(args).output()?;
    Ok(output.status.success().then_some(output.stdout))
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
