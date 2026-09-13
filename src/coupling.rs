//! Temporal (change) coupling.
//!
//! Files that repeatedly change in the same commits are historically related
//! even when no import connects them: shared release steps, copy-pasted
//! validation, parallel implementations of one concept. This module indexes
//! co-changes from the same streamed history walk that powers
//! [`crate::history`] and answers "what tends to change with this file?".
//!
//! Terminology for target `A` and candidate `B`:
//!
//! - `co_changes`: commits that touched both.
//! - `directional` (A -> B): `co_changes / commits(A)`.
//! - `reverse_directional` (B -> A): `co_changes / commits(B)`.
//! - `jaccard`: `co_changes / (commits(A) + commits(B) - co_changes)`.
//!
//! Co-change is process evidence, not a dependency. Two files can share
//! commits for unrelated reasons (formatting sweeps, dependency bumps, a
//! release touching both), so treat coupling as an inspection hint, never as
//! build-time truth. Commits wider than [`MAX_COMMIT_FILES`] are excluded
//! from pair counting because they create thousands of spurious pairs; they
//! still count toward each file's commit total.
//!
//! [`ProjectCoupling`] is the whole-project index built by the same history
//! walk. It keeps every undirected pair once, ordered by `(source, target)`,
//! with `directional` measured from `source` and `reverse_directional` from
//! `target`, so an edge carries exactly the two per-target values the
//! [`CouplingReport`] would report. The index is capped at
//! [`MAX_PROJECT_PAIRS`] unique pairs; crossing the ceiling makes coupling
//! incomplete instead of silently dropping pairs.

use crate::Result;
use crate::history::{CommitFile, CommitMeta, walk_commits};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const COUPLING_SCHEMA_VERSION: u32 = 1;

/// Commits touching more files than this do not contribute co-change pairs.
pub const MAX_COMMIT_FILES: usize = 50;

/// Unique whole-project pairs retained before the index is incomplete.
pub const MAX_PROJECT_PAIRS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CouplingOptions {
    pub min_co_changes: u64,
    pub limit: usize,
}

impl Default for CouplingOptions {
    fn default() -> Self {
        Self {
            min_co_changes: 2,
            limit: 20,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RelatedFile {
    pub path: String,
    /// Commits touching the related file.
    pub commits: u64,
    pub co_changes: u64,
    /// `co_changes / target_commits`.
    pub directional: f64,
    /// `co_changes / commits`, from the related file's perspective.
    pub reverse_directional: f64,
    pub jaccard: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CouplingReport {
    pub schema_version: u32,
    pub analyzer_version: &'static str,
    pub target: String,
    pub git_available: bool,
    /// Commits touching the target, including wide commits.
    pub target_commits: u64,
    /// Target commits small enough to contribute pairs.
    pub pair_commits: u64,
    pub max_commit_files: usize,
    pub related: Vec<RelatedFile>,
    pub truncated: bool,
}

/// One undirected pair of files that changed together.
///
/// `source` is the lexicographically smaller path. `directional` is measured
/// from `source` and `reverse_directional` from `target`, matching the
/// [`RelatedFile`] values for `target = source` and `target = target`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CouplingEdge {
    pub source: String,
    pub target: String,
    pub co_changes: u64,
    /// `co_changes / commits(source)`.
    pub directional: f64,
    /// `co_changes / commits(target)`.
    pub reverse_directional: f64,
    pub jaccard: f64,
}

/// Deterministic whole-project coupling from one revision-bounded walk.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectCoupling {
    pub available: bool,
    /// Stable reason when `available` is false, e.g. `pair_limit_exceeded`.
    pub reason: Option<String>,
    /// Undirected pairs sorted by `(source, target)`.
    pub edges: Vec<CouplingEdge>,
}

impl ProjectCoupling {
    pub(crate) fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_owned()),
            edges: Vec::new(),
        }
    }
}

/// Indexes concurrent changes for one target file within `scope`.
///
/// `scope` must be the directory whose history keys the caller uses; results
/// are keyed the same way as [`crate::history::analyze_history`]. A missing
/// repository yields an unavailable report instead of an error.
pub fn analyze_coupling(
    scope: &Path,
    target: &str,
    options: &CouplingOptions,
) -> Result<CouplingReport> {
    let workdir = if scope.is_dir() {
        scope
    } else {
        scope.parent().unwrap_or(Path::new("."))
    };
    let Some((head_commit, _)) = crate::history::repository_head(workdir)? else {
        return Ok(unavailable(target));
    };
    let Some(head_commit) = head_commit else {
        return Ok(unavailable(target));
    };
    let mut accumulator = CouplingAccumulator::new(target.to_owned());
    walk_commits(workdir, &head_commit, |meta, files| {
        accumulator.record(meta, files)
    })?;
    Ok(accumulator.finish(options))
}

fn unavailable(target: &str) -> CouplingReport {
    CouplingReport {
        schema_version: COUPLING_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        target: target.to_owned(),
        git_available: false,
        target_commits: 0,
        pair_commits: 0,
        max_commit_files: MAX_COMMIT_FILES,
        related: Vec::new(),
        truncated: false,
    }
}

struct CouplingAccumulator {
    target: String,
    /// Commits touching each file, including wide commits.
    file_commits: BTreeMap<String, u64>,
    target_commits: u64,
    pair_commits: u64,
    co_changes: BTreeMap<String, u64>,
}

impl CouplingAccumulator {
    fn new(target: String) -> Self {
        Self {
            target,
            file_commits: BTreeMap::new(),
            target_commits: 0,
            pair_commits: 0,
            co_changes: BTreeMap::new(),
        }
    }

    fn record(&mut self, _meta: &CommitMeta, files: &[CommitFile]) {
        // Out-of-scope pre-images can leak from cross-scope renames on some
        // git versions; they are never part of this scope's pairs.
        let scoped: Vec<&CommitFile> = files
            .iter()
            .filter(|file| !file.path.starts_with("../"))
            .collect();
        for file in &scoped {
            *self.file_commits.entry(file.path.clone()).or_default() += 1;
        }
        if !scoped.iter().any(|file| file.path == self.target) {
            return;
        }
        self.target_commits += 1;
        if scoped.len() > MAX_COMMIT_FILES {
            return;
        }
        self.pair_commits += 1;
        for file in &scoped {
            if file.path != self.target {
                *self.co_changes.entry(file.path.clone()).or_default() += 1;
            }
        }
    }

    fn finish(self, options: &CouplingOptions) -> CouplingReport {
        let mut related: Vec<RelatedFile> = self
            .co_changes
            .into_iter()
            .filter(|(_, co_changes)| *co_changes >= options.min_co_changes)
            .map(|(path, co_changes)| {
                let commits = self.file_commits.get(&path).copied().unwrap_or(co_changes);
                let target = self.target_commits;
                RelatedFile {
                    path,
                    commits,
                    co_changes,
                    directional: co_changes as f64 / target as f64,
                    reverse_directional: co_changes as f64 / commits as f64,
                    jaccard: co_changes as f64 / (target + commits - co_changes) as f64,
                }
            })
            .collect();
        related.sort_by(|left, right| {
            right
                .directional
                .total_cmp(&left.directional)
                .then_with(|| right.co_changes.cmp(&left.co_changes))
                .then_with(|| left.path.cmp(&right.path))
        });
        let truncated = related.len() > options.limit;
        related.truncate(options.limit);
        CouplingReport {
            schema_version: COUPLING_SCHEMA_VERSION,
            analyzer_version: env!("CARGO_PKG_VERSION"),
            target: self.target,
            git_available: true,
            target_commits: self.target_commits,
            pair_commits: self.pair_commits,
            max_commit_files: MAX_COMMIT_FILES,
            related,
            truncated,
        }
    }
}

const PAIR_LIMIT_EXCEEDED: &str = "pair_limit_exceeded";

/// Accumulates every undirected co-change pair for the whole project.
///
/// The rules mirror [`CouplingAccumulator`]: every in-scope file records every
/// commit that touched it, but only commits at or below [`MAX_COMMIT_FILES`]
/// contribute pairs. Crossing [`MAX_PROJECT_PAIRS`] marks the result
/// incomplete instead of dropping or truncating pairs silently.
pub(crate) struct ProjectCouplingAccumulator {
    /// Commits touching each file, including wide commits.
    file_commits: BTreeMap<String, u64>,
    /// `source -> target -> co_changes` with `source < target`.
    pairs: BTreeMap<String, BTreeMap<String, u64>>,
    pair_count: usize,
    limit: usize,
    overflowed: bool,
}

impl ProjectCouplingAccumulator {
    pub(crate) fn new() -> Self {
        Self::with_limit(MAX_PROJECT_PAIRS)
    }

    /// Test seam: exercises the ceiling without building a million pairs.
    fn with_limit(limit: usize) -> Self {
        Self {
            file_commits: BTreeMap::new(),
            pairs: BTreeMap::new(),
            pair_count: 0,
            limit,
            overflowed: false,
        }
    }

    pub(crate) fn record(&mut self, files: &[CommitFile]) {
        // Out-of-scope pre-images can leak from cross-scope renames on some
        // git versions; they are never part of this scope's pairs.
        let scoped: Vec<&CommitFile> = files
            .iter()
            .filter(|file| !file.path.starts_with("../"))
            .collect();
        for file in &scoped {
            *self.file_commits.entry(file.path.clone()).or_default() += 1;
        }
        if self.overflowed || scoped.len() > MAX_COMMIT_FILES {
            return;
        }
        for (index, left) in scoped.iter().enumerate() {
            for right in &scoped[index + 1..] {
                let (source, target) = ordered_paths(&left.path, &right.path);
                let existing = self
                    .pairs
                    .get_mut(source)
                    .and_then(|row| row.get_mut(target));
                match existing {
                    Some(co_changes) => *co_changes += 1,
                    None => {
                        if self.pair_count >= self.limit {
                            // Keep history and ownership valid; only coupling
                            // becomes unavailable. Dropping the map bounds
                            // memory for pathological histories.
                            self.overflowed = true;
                            self.pairs.clear();
                            self.pair_count = 0;
                            return;
                        }
                        self.pairs
                            .entry(source.to_owned())
                            .or_default()
                            .insert(target.to_owned(), 1);
                        self.pair_count += 1;
                    }
                }
            }
        }
    }

    pub(crate) fn finish(self) -> ProjectCoupling {
        if self.overflowed {
            return ProjectCoupling {
                available: false,
                reason: Some(PAIR_LIMIT_EXCEEDED.to_owned()),
                edges: Vec::new(),
            };
        }
        let ProjectCouplingAccumulator {
            file_commits,
            pairs,
            ..
        } = self;
        let mut edges = Vec::new();
        for (source, row) in pairs {
            let source_total = file_commits.get(&source).copied();
            for (target, co_changes) in row {
                let source_commits = source_total.unwrap_or(co_changes);
                let target_commits = file_commits.get(&target).copied().unwrap_or(co_changes);
                edges.push(CouplingEdge {
                    source: source.clone(),
                    target,
                    co_changes,
                    directional: co_changes as f64 / source_commits as f64,
                    reverse_directional: co_changes as f64 / target_commits as f64,
                    jaccard: co_changes as f64
                        / (source_commits + target_commits - co_changes) as f64,
                });
            }
        }
        ProjectCoupling {
            available: true,
            reason: None,
            edges,
        }
    }
}

fn ordered_paths<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> CommitFile {
        CommitFile {
            path: path.to_owned(),
            lines_added: 1,
            lines_deleted: 0,
        }
    }

    fn options() -> CouplingOptions {
        CouplingOptions {
            min_co_changes: 1,
            limit: 10,
        }
    }

    #[test]
    fn out_of_scope_preimage_paths_are_ignored() {
        let mut accumulator = CouplingAccumulator::new("a.ts".to_owned());
        accumulator.record(
            &CommitMeta::default(),
            &[file("a.ts"), file("b.ts"), file("../outside.ts")],
        );
        let report = accumulator.finish(&options());
        assert_eq!(report.target_commits, 1);
        let paths: Vec<&str> = report
            .related
            .iter()
            .map(|related| related.path.as_str())
            .collect();
        assert_eq!(paths, ["b.ts"]);
    }

    #[test]
    fn exactly_max_commit_files_still_pairs() {
        let mut files = vec![file("a.ts")];
        for index in 0..MAX_COMMIT_FILES - 1 {
            files.push(file(&format!("other{index}.ts")));
        }
        assert_eq!(files.len(), MAX_COMMIT_FILES);
        let mut accumulator = CouplingAccumulator::new("a.ts".to_owned());
        accumulator.record(&CommitMeta::default(), &files);
        let report = accumulator.finish(&CouplingOptions {
            min_co_changes: 1,
            limit: MAX_COMMIT_FILES,
        });
        assert_eq!(report.target_commits, 1);
        assert_eq!(report.pair_commits, 1);
        assert_eq!(report.related.len(), MAX_COMMIT_FILES - 1);
    }

    #[test]
    fn wide_commits_do_not_pair() {
        let mut files = vec![file("a.ts")];
        for index in 0..MAX_COMMIT_FILES {
            files.push(file(&format!("wide{index}.ts")));
        }
        assert_eq!(files.len(), MAX_COMMIT_FILES + 1);
        let mut accumulator = CouplingAccumulator::new("a.ts".to_owned());
        accumulator.record(&CommitMeta::default(), &files);
        let report = accumulator.finish(&options());
        assert_eq!(report.target_commits, 1);
        assert_eq!(report.pair_commits, 0);
        assert!(report.related.is_empty());
    }

    #[test]
    fn pair_limit_is_exact_and_overflow_marks_coupling_incomplete() {
        let files = [file("a.ts"), file("b.ts"), file("c.ts"), file("d.ts")];
        // Four files have six unique pairs; a limit of six still completes.
        let mut complete = ProjectCouplingAccumulator::with_limit(6);
        complete.record(&files);
        let coupling = complete.finish();
        assert!(coupling.available);
        assert_eq!(coupling.reason, None);
        assert_eq!(coupling.edges.len(), 6);

        let mut exceeded = ProjectCouplingAccumulator::with_limit(5);
        exceeded.record(&files);
        let coupling = exceeded.finish();
        assert!(!coupling.available);
        assert_eq!(coupling.reason.as_deref(), Some("pair_limit_exceeded"));
        assert!(coupling.edges.is_empty());
    }

    #[test]
    fn project_edges_are_sorted_and_measure_both_directions() {
        let mut accumulator = ProjectCouplingAccumulator::new();
        accumulator.record(&[file("b.ts"), file("a.ts"), file("c.ts")]);
        accumulator.record(&[file("a.ts"), file("b.ts")]);
        let coupling = accumulator.finish();
        let pairs: Vec<(&str, &str)> = coupling
            .edges
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [("a.ts", "b.ts"), ("a.ts", "c.ts"), ("b.ts", "c.ts")]
        );
        let ab = &coupling.edges[0];
        assert_eq!(ab.co_changes, 2);
        assert!((ab.directional - 1.0).abs() < 1e-9);
        assert!((ab.reverse_directional - 1.0).abs() < 1e-9);
        assert!((ab.jaccard - 1.0).abs() < 1e-9);
        let ac = &coupling.edges[1];
        assert_eq!(ac.co_changes, 1);
        assert!((ac.directional - 0.5).abs() < 1e-9);
        assert!((ac.reverse_directional - 1.0).abs() < 1e-9);
    }
}
