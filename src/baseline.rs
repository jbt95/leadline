//! Saved baselines: deterministic snapshots for regression gates without Git.
//!
//! A baseline stores one row per function: path, stable function id, name,
//! line, and the gate metrics. Rows pair with current functions by path and
//! name in same-name source order (mirroring [`crate::diff`] changed
//! pairing), because byte-span ids shift on any edit above or inside the
//! function. Stored ids are still validated unique on read and reported
//! back in findings. Deltas reuse [`crate::diff::regression_violates`], so
//! Git and baseline gates share one predicate. New functions are never
//! delta regressions; deleted functions are ignored.

use crate::config::RegressionLimits;
use crate::core::{
    AnalysisReport, FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Schema version of the baseline file format.
pub const BASELINE_SCHEMA_VERSION: u32 = 1;

/// One snapshotted function: identity plus the gate metrics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaselineFunction {
    pub path: String,
    pub id: String,
    pub name: String,
    pub line: u32,
    pub cognitive: u32,
    pub cyclomatic: u32,
    pub crap: Option<f64>,
    pub max_nesting: u32,
}

impl BaselineFunction {
    /// Rebuild a comparable analysis row so baseline deltas share the Git
    /// gate predicate. Only gate metrics carry over; contributions stay
    /// empty because causes are a worktree-comparison feature.
    pub fn to_analysis(&self) -> FunctionAnalysis {
        FunctionAnalysis {
            name: self.name.clone(),
            id: self.id.clone(),
            kind: FunctionKind::Function,
            start_line: self.line,
            end_line: self.line,
            start_byte: 0,
            end_byte: 0,
            metrics: FunctionMetrics {
                loc: 0,
                logical_loc: 0,
                function_length: 0,
                parameters: 0,
                max_nesting: self.max_nesting,
                cyclomatic: self.cyclomatic,
                cognitive: self.cognitive,
                halstead_n1: 0,
                halstead_n2: 0,
                halstead_total_operators: 0,
                halstead_total_operands: 0,
                halstead_vocabulary: 0,
                halstead_length: 0,
                halstead_volume: 0.0,
                halstead_difficulty: 0.0,
                halstead_effort: 0.0,
                maintainability_index: 0.0,
                coverage: None,
                crap: self.crap,
            },
            contributions: Vec::new(),
            source_fingerprint: 0,
        }
    }
}

/// Deterministic snapshot of an [`AnalysisReport`]: rows sorted by path,
/// then function line, name, and id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub schema_version: u32,
    pub metric_profile: String,
    pub functions: Vec<BaselineFunction>,
}

/// One paired function whose positive delta exceeds its allowed limits.
#[derive(Clone, Debug, PartialEq)]
pub struct BaselineRegression {
    pub path: String,
    pub id: String,
    pub name: String,
    pub before: BaselineFunction,
    pub after: FunctionAnalysis,
}

impl Baseline {
    /// Snapshot every function in report order (files sorted by path,
    /// functions in source order), then sort rows for determinism.
    pub fn from_report(report: &AnalysisReport) -> Self {
        let mut functions = Vec::new();
        for file in &report.files {
            for function in &file.functions {
                functions.push(BaselineFunction {
                    path: file.path.clone(),
                    id: function.id.clone(),
                    name: function.name.clone(),
                    line: function.start_line,
                    cognitive: function.metrics.cognitive,
                    cyclomatic: function.metrics.cyclomatic,
                    crap: function.metrics.crap,
                    max_nesting: function.metrics.max_nesting,
                });
            }
        }
        functions.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.line.cmp(&right.line))
                .then(left.name.cmp(&right.name))
                .then(left.id.cmp(&right.id))
        });
        Self {
            schema_version: BASELINE_SCHEMA_VERSION,
            metric_profile: METRIC_PROFILE.to_owned(),
            functions,
        }
    }

    /// Read and validate a baseline file. Rejects unknown schemas,
    /// unknown metric profiles, and duplicate `(path, id)` identities.
    pub fn read(path: &Path) -> crate::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("baseline: cannot read {}: {error}", path.display()))?;
        let baseline: Self = serde_json::from_str(&text)
            .map_err(|error| format!("baseline: cannot parse {}: {error}", path.display()))?;
        baseline.validate()?;
        Ok(baseline)
    }

    /// Write atomically: encode to a sibling temporary file, then rename
    /// over the destination so readers never see a partial snapshot.
    pub fn write(&self, path: &Path) -> crate::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        let file_name = path
            .file_name()
            .ok_or("baseline: output path has no file name")?;
        let mut sibling = path.to_path_buf();
        sibling.set_file_name(format!(".{}.tmp", file_name.to_string_lossy()));
        if let Err(error) = std::fs::write(&sibling, &text) {
            let _ = std::fs::remove_file(&sibling);
            return Err(format!("baseline: cannot write {}: {error}", sibling.display()).into());
        }
        if let Err(error) = std::fs::rename(&sibling, path) {
            let _ = std::fs::remove_file(&sibling);
            return Err(format!("baseline: cannot write {}: {error}", path.display()).into());
        }
        Ok(())
    }

    /// Paired functions whose positive delta exceeds `limits`, in
    /// deterministic `(path, line, name, id)` order. New functions are
    /// never delta regressions; deleted functions are ignored.
    pub fn compare(
        &self,
        report: &AnalysisReport,
        limits: &RegressionLimits,
    ) -> Vec<BaselineRegression> {
        let mut before_by_key: BTreeMap<(String, String), Vec<&BaselineFunction>> = BTreeMap::new();
        for function in &self.functions {
            before_by_key
                .entry((function.path.clone(), function.name.clone()))
                .or_default()
                .push(function);
        }
        let mut regressions = Vec::new();
        for file in &report.files {
            let mut after_by_name: BTreeMap<String, Vec<&FunctionAnalysis>> = BTreeMap::new();
            for function in &file.functions {
                after_by_name
                    .entry(function.name.clone())
                    .or_default()
                    .push(function);
            }
            for (name, after_group) in &after_by_name {
                let before_group = before_by_key
                    .get(&(file.path.clone(), name.clone()))
                    .cloned()
                    .unwrap_or_default();
                for (index, after) in after_group.iter().enumerate() {
                    let Some(before) = before_group.get(index) else {
                        continue;
                    };
                    if crate::diff::regression_violates(&before.to_analysis(), after, limits) {
                        regressions.push(BaselineRegression {
                            path: file.path.clone(),
                            id: after.id.clone(),
                            name: after.name.clone(),
                            before: (*before).clone(),
                            after: (*after).clone(),
                        });
                    }
                }
            }
        }
        regressions.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.after.start_line.cmp(&right.after.start_line))
                .then(left.name.cmp(&right.name))
                .then(left.id.cmp(&right.id))
        });
        regressions
    }

    fn validate(&self) -> crate::Result<()> {
        if self.schema_version != BASELINE_SCHEMA_VERSION {
            return Err(format!(
                "baseline: unsupported schema_version {}, expected {BASELINE_SCHEMA_VERSION}",
                self.schema_version
            )
            .into());
        }
        if self.metric_profile != METRIC_PROFILE {
            return Err(format!(
                "baseline: unsupported metric_profile {:?}, expected {METRIC_PROFILE:?}",
                self.metric_profile
            )
            .into());
        }
        let mut seen = BTreeSet::new();
        for function in &self.functions {
            if !seen.insert((&function.path, &function.id)) {
                return Err(format!(
                    "baseline: duplicate function {} {}",
                    function.path, function.id
                )
                .into());
            }
        }
        Ok(())
    }
}
