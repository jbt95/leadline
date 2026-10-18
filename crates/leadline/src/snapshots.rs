//! Compact trend snapshots keyed by HEAD commit.
//!
//! Capture analyzes the resolved HEAD tree (never dirty worktree state) and
//! stores one point per `(commit, scope, config, models)` key. Appends are
//! lock-protected and atomically renamed so concurrent writers cannot lose a
//! point or leave a torn store.

use crate::Result;
use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotOutcome {
    Added,
    Unchanged,
    Replaced,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrendStore {
    pub schema_version: u32,
    pub points: Vec<TrendPoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrendPoint {
    pub commit: String,
    pub commit_timestamp: i64,
    pub scope: String,
    pub analyzer_version: String,
    pub config_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailmap_hash: Option<String>,
    pub metric_profile: String,
    pub risk_model: String,
    pub duplication_profile: String,
    pub mutation_model: String,
    pub source_hash: String,
    pub input_fingerprint: String,
    pub files: u64,
    pub functions: u64,
    pub parse_errors: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_cognitive: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cognitive: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotspot_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_hotspot_score: Option<u64>,
    pub risk_files_ge_70: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_risk_score: Option<f64>,
    pub dependency_edges: u64,
    pub cycles: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupling_edges: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_ownership_concentration_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scored_mutants: Option<u64>,
    pub duplication_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_groups: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicated_lines: Option<u64>,
    pub policy_info: u64,
    pub policy_warning: u64,
    pub policy_error: u64,
}

impl TrendPoint {
    /// Derives one compact point from a complete Project plus input hashes.
    pub fn from_project(
        project: &Project,
        scope: String,
        config_hash: String,
        mailmap_hash: Option<String>,
        source_hash: String,
        input_fingerprint: String,
    ) -> TrendPoint {
        let commit = project.meta.head_commit.clone().unwrap_or_default();
        let commit_timestamp = project.meta.head_timestamp.unwrap_or(0);
        let total_cognitive: u64 = project
            .functions
            .iter()
            .map(|function| u64::from(function.cognitive))
            .sum();
        let mean_cognitive = if project.functions.is_empty() {
            None
        } else {
            Some(total_cognitive as f64 / project.functions.len() as f64)
        };
        let max_cognitive = project
            .functions
            .iter()
            .map(|function| function.cognitive)
            .max();
        let changes_90d: BTreeMap<&str, u64> = project
            .git_activity
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|file| (file.path.as_str(), file.changes_90d))
            .collect();
        let mut hotspot_files: u64 = 0;
        let mut max_hotspot_score = None;
        if !changes_90d.is_empty() {
            for file in &project.files {
                let Some(changes) = changes_90d.get(file.path.as_str()) else {
                    continue;
                };
                let score = u64::from(file.max_cognitive) * changes;
                if score > 0 {
                    hotspot_files += 1;
                    max_hotspot_score =
                        Some(max_hotspot_score.map_or(score, |max: u64| max.max(score)));
                }
            }
        }
        let coupling_edges = project
            .temporal_coupling
            .as_ref()
            .and_then(|section| section.available.then_some(section.edges.len() as u64));
        let max_ownership_concentration_percent =
            project.ownership.as_ref().and_then(|ownership| {
                ownership
                    .files
                    .iter()
                    .filter_map(|file| file.concentration_percent)
                    .reduce(f64::max)
            });
        let mutation = project.mutation.as_ref().map(|mutation| &mutation.summary);
        let (duplicate_groups, duplicated_lines) = if project.duplication.complete {
            (
                Some(project.duplication.groups.len() as u64),
                Some(project.duplication.duplicated_lines),
            )
        } else {
            (None, None)
        };
        let (mut policy_info, mut policy_warning, mut policy_error) = (0_u64, 0_u64, 0_u64);
        for violation in &project.architecture_violations {
            if matches!(
                violation.status,
                Some(crate::policy::PolicyStatus::Resolved)
            ) {
                continue;
            }
            match violation.severity {
                crate::config::Severity::Info => policy_info += 1,
                crate::config::Severity::Warning => policy_warning += 1,
                crate::config::Severity::Error => policy_error += 1,
            }
        }
        let risk_files_ge_70 = project
            .risk
            .rows
            .iter()
            .filter(|row| row.score >= 70.0)
            .count() as u64;
        let max_risk_score = project
            .risk
            .rows
            .iter()
            .map(|row| row.score)
            .reduce(f64::max);
        TrendPoint {
            commit,
            commit_timestamp,
            scope,
            analyzer_version: project.meta.analyzer_version.to_owned(),
            config_hash,
            mailmap_hash,
            metric_profile: project.meta.metric_profile.to_owned(),
            risk_model: project.risk.model.to_owned(),
            duplication_profile: project.duplication.profile.to_owned(),
            mutation_model: project
                .mutation
                .as_ref()
                .map(|mutation| mutation.model.to_owned())
                .unwrap_or_else(|| "none".to_owned()),
            source_hash,
            input_fingerprint,
            files: project.summary.files,
            functions: project.summary.functions,
            parse_errors: project.summary.parse_errors,
            mean_cognitive,
            max_cognitive,
            coverage_percent: project.summary.coverage_percent,
            hotspot_files: (!changes_90d.is_empty()).then_some(hotspot_files),
            max_hotspot_score,
            risk_files_ge_70,
            max_risk_score,
            dependency_edges: project.summary.dependency_edges,
            cycles: project.summary.dependency_cycles,
            coupling_edges,
            max_ownership_concentration_percent,
            mutation_score: mutation.and_then(|mutation| mutation.score),
            scored_mutants: mutation.map(|mutation| mutation.scored_mutants),
            duplication_complete: project.duplication.complete,
            duplicate_groups,
            duplicated_lines,
            policy_info,
            policy_warning,
            policy_error,
        }
    }

    fn key(&self) -> (String, String, String, String, String, String, String) {
        (
            self.commit.clone(),
            self.scope.clone(),
            self.config_hash.clone(),
            self.metric_profile.clone(),
            self.risk_model.clone(),
            self.duplication_profile.clone(),
            self.mutation_model.clone(),
        )
    }
}

impl TrendStore {
    pub fn new() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            points: Vec::new(),
        }
    }

    /// Reads and validates a store; missing files start a new store.
    pub fn read(path: &Path) -> Result<TrendStore> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(error) => {
                return Err(format!("snapshot: cannot read {}: {error}", path.display()).into());
            }
        };
        let store: TrendStore = serde_json::from_str(&text)
            .map_err(|error| format!("snapshot: cannot parse {}: {error}", path.display()))?;
        if store.schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(format!(
                "snapshot: unsupported schema_version {}, expected {SNAPSHOT_SCHEMA_VERSION}",
                store.schema_version
            )
            .into());
        }
        Ok(store)
    }

    /// Adds or replaces one point; identical keys with identical fingerprints
    /// are idempotent and changed fingerprints require `replace`.
    pub fn append(&mut self, point: TrendPoint, replace: bool) -> Result<SnapshotOutcome> {
        let key = point.key();
        match self.points.iter().position(|row| row.key() == key) {
            Some(index) => {
                if self.points[index].input_fingerprint == point.input_fingerprint {
                    return Ok(SnapshotOutcome::Unchanged);
                }
                if !replace {
                    return Err(format!(
                        "snapshot: {} already has a different fingerprint; pass --replace",
                        point.commit
                    )
                    .into());
                }
                self.points[index] = point;
                self.sort_points();
                Ok(SnapshotOutcome::Replaced)
            }
            None => {
                self.points.push(point);
                self.sort_points();
                Ok(SnapshotOutcome::Added)
            }
        }
    }

    fn sort_points(&mut self) {
        self.points.sort_by(|left, right| {
            left.commit_timestamp
                .cmp(&right.commit_timestamp)
                .then_with(|| left.commit.cmp(&right.commit))
                .then_with(|| left.scope.cmp(&right.scope))
        });
    }

    /// Writes the store atomically under a sibling lock.
    pub fn write(&self, path: &Path) -> Result<()> {
        let _guard = LockGuard::acquire(path)?;
        let text = serde_json::to_string_pretty(self)?;
        write_atomic(path, &text)
    }
}

impl Default for TrendStore {
    fn default() -> Self {
        Self::new()
    }
}

struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    fn acquire(target: &Path) -> Result<LockGuard> {
        let file_name = target
            .file_name()
            .ok_or("snapshot: output path has no file name")?;
        let mut path = target.to_path_buf();
        path.set_file_name(format!(".{}.lock", file_name.to_string_lossy()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "snapshot: cannot lock {} ({error}); remove it if no writer is running",
                    path.display()
                )
            })?;
        let _ = writeln!(file, "{}", std::process::id());
        Ok(LockGuard { path })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .ok_or("snapshot: output path has no file name")?;
    let mut sibling = path.to_path_buf();
    sibling.set_file_name(format!(".{}.tmp", file_name.to_string_lossy()));
    if let Err(error) = std::fs::write(&sibling, text) {
        let _ = std::fs::remove_file(&sibling);
        return Err(format!("snapshot: cannot write {}: {error}", sibling.display()).into());
    }
    if let Err(error) = std::fs::rename(&sibling, path) {
        let _ = std::fs::remove_file(&sibling);
        return Err(format!("snapshot: cannot write {}: {error}", path.display()).into());
    }
    Ok(())
}
