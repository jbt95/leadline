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

use crate::Result;
use crate::history::{CommitFile, CommitMeta, walk_commits};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const COUPLING_SCHEMA_VERSION: u32 = 1;

/// Commits touching more files than this do not contribute co-change pairs.
pub const MAX_COMMIT_FILES: usize = 50;

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
    let Some(_head) = crate::history::repository_head(workdir)? else {
        return Ok(unavailable(target));
    };
    let mut accumulator = CouplingAccumulator::new(target.to_owned());
    walk_commits(workdir, |meta, files| accumulator.record(meta, files))?;
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
}
