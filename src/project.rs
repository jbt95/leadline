//! Canonical project model: the one broad join across every analytics view.
//!
//! `project` owns path joins and aggregation; adapters and renderers stay
//! thin. Every section is deterministic, every optional section is `None` when
//! unavailable, and the DTO is published whole (no piecemeal schema growth).

use crate::core::{AnalysisReport, Language};
use crate::duplication::DuplicationReport;
use crate::graph::DependencyReport;
use crate::history::{FileHistory, GitAnalyticsSnapshot};
use crate::mutation::MutationReport;
use crate::ownership::OwnershipReport;
use crate::policy::{PolicyReport, PolicyViolation};
use crate::risk::RiskReport;
use crate::risk_v2::RiskV2Report;
use crate::test_relationships::TestRelationshipReport;
use serde::Serialize;
use std::collections::BTreeMap;

pub const PROJECT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectMeta {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub generated_from: String,
    pub git_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_timestamp: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectSummary {
    pub files: u64,
    pub functions: u64,
    pub parse_errors: u64,
    pub dependency_edges: u64,
    pub dependency_cycles: u64,
    pub coverage_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_score: Option<f64>,
    pub duplication_groups: u64,
    pub duplicated_lines: u64,
    pub policy_violations: u64,
    pub risk_model: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectModule {
    pub path: String,
    pub files: u64,
    pub functions: u64,
    pub loc: u64,
    pub max_cognitive: u32,
    pub max_cyclomatic: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectFile {
    pub path: String,
    pub language: Language,
    pub functions: u64,
    pub loc: u64,
    pub max_cognitive: u32,
    pub max_cyclomatic: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_crap: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_percent: Option<f64>,
    pub parse_errors: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectFunction {
    pub path: String,
    pub id: String,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub cognitive: u32,
    pub cyclomatic: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crap: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<f64>,
    pub max_nesting: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectDependencies {
    pub edges: Vec<crate::graph::DependencyEdge>,
    pub unresolved: Vec<crate::graph::UnresolvedDependency>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CoverageSection {
    pub covered_functions: u64,
    pub functions: u64,
    pub percent: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CouplingSection {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub edges: Vec<crate::coupling::CouplingEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectRisk {
    pub model: &'static str,
    pub window: String,
    pub git_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_commit: Option<String>,
    pub rows: Vec<ProjectRiskRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectRiskRow {
    pub path: String,
    pub score: f64,
    pub components: BTreeMap<&'static str, Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cognitive: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cyclomatic: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_crap: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributors: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration_percent: Option<f64>,
    pub blast_radius: usize,
    pub blast_radius_percent: f64,
    pub fan_in: usize,
    pub fan_out: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_severity: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Project {
    pub meta: ProjectMeta,
    pub summary: ProjectSummary,
    pub modules: Vec<ProjectModule>,
    pub files: Vec<ProjectFile>,
    pub functions: Vec<ProjectFunction>,
    pub dependencies: ProjectDependencies,
    pub cycles: Vec<crate::graph::DependencyCycle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_activity: Option<Vec<FileHistory>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_coupling: Option<CouplingSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ownership: Option<OwnershipReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation: Option<MutationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_relationships: Option<TestRelationshipReport>,
    pub duplication: DuplicationReport,
    pub architecture_violations: Vec<PolicyViolation>,
    pub risk: ProjectRisk,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshots: Option<Vec<crate::snapshots::TrendPoint>>,
}

/// Everything the Project join consumes; all inputs stay borrowed.
pub struct ProjectInputs<'a> {
    pub analysis: &'a AnalysisReport,
    pub generated_from: String,
    pub git: Option<&'a GitAnalyticsSnapshot>,
    pub graph: &'a DependencyReport,
    pub ownership: Option<&'a OwnershipReport>,
    pub mutation: Option<&'a MutationReport>,
    pub test_relationships: Option<&'a TestRelationshipReport>,
    pub duplication: &'a DuplicationReport,
    pub policy: Option<&'a PolicyReport>,
    pub risk_v1: Option<&'a RiskReport>,
    pub risk_v2: Option<&'a RiskV2Report>,
    pub snapshots: Option<Vec<crate::snapshots::TrendPoint>>,
}

/// Joins every section through indexed lookups, never embedded reports.
pub fn build(inputs: ProjectInputs<'_>) -> Project {
    let analysis = inputs.analysis;
    let files: Vec<ProjectFile> = analysis
        .files
        .iter()
        .map(|file| {
            let max_crap = file
                .functions
                .iter()
                .filter_map(|function| function.metrics.crap)
                .reduce(f64::max);
            let covered: Vec<f64> = file
                .functions
                .iter()
                .filter_map(|function| function.metrics.coverage)
                .collect();
            ProjectFile {
                path: file.path.clone(),
                language: file.language,
                functions: file.functions.len() as u64,
                loc: file
                    .functions
                    .iter()
                    .map(|function| u64::from(function.metrics.loc))
                    .sum(),
                max_cognitive: file
                    .functions
                    .iter()
                    .map(|function| function.metrics.cognitive)
                    .max()
                    .unwrap_or(0),
                max_cyclomatic: file
                    .functions
                    .iter()
                    .map(|function| function.metrics.cyclomatic)
                    .max()
                    .unwrap_or(0),
                max_crap,
                coverage_percent: if covered.is_empty() {
                    None
                } else {
                    Some(covered.iter().sum::<f64>() / covered.len() as f64 * 100.0)
                },
                parse_errors: file.parse_errors.len() as u64,
            }
        })
        .collect();
    let functions: Vec<ProjectFunction> = analysis
        .files
        .iter()
        .flat_map(|file| {
            file.functions.iter().map(|function| ProjectFunction {
                path: file.path.clone(),
                id: function.id.clone(),
                name: function.name.clone(),
                start_line: function.start_line,
                end_line: function.end_line,
                cognitive: function.metrics.cognitive,
                cyclomatic: function.metrics.cyclomatic,
                crap: function.metrics.crap,
                coverage: function.metrics.coverage,
                max_nesting: function.metrics.max_nesting,
            })
        })
        .collect();
    let modules = module_rows(&files);

    let mut cycles = inputs.graph.cycles.clone();
    cycles.sort_by(|left, right| left.files.cmp(&right.files));

    let (git_activity, temporal_coupling) = match inputs.git {
        Some(git) => (
            Some(git.history.files.clone()),
            Some(CouplingSection {
                available: git.coupling.available,
                reason: git.coupling.reason.clone(),
                edges: git.coupling.edges.clone(),
            }),
        ),
        None => (None, None),
    };

    let (risk, risk_model) = match (inputs.risk_v2, inputs.risk_v1) {
        (Some(v2), _) => (
            ProjectRisk {
                model: v2.model,
                window: v2.window.to_owned(),
                git_available: v2.git_available,
                head_commit: v2.head_commit.clone(),
                rows: v2
                    .risks
                    .iter()
                    .map(|row| ProjectRiskRow {
                        path: row.path.clone(),
                        score: row.score,
                        components: v2_components(&row.components),
                        max_cognitive: Some(row.raw.max_cognitive),
                        max_cyclomatic: Some(row.raw.max_cyclomatic),
                        max_crap: row.raw.max_crap,
                        changes: row.raw.changes,
                        contributors: row.raw.contributors,
                        concentration_percent: row.raw.concentration_percent,
                        blast_radius: row.raw.blast_radius,
                        blast_radius_percent: row.raw.blast_radius_percent,
                        fan_in: row.raw.fan_in,
                        fan_out: row.raw.fan_out,
                        policy_severity: row.raw.policy_severity,
                    })
                    .collect(),
            },
            Some(v2.model),
        ),
        (None, Some(v1)) => (
            ProjectRisk {
                model: v1.model,
                window: v1.window.to_owned(),
                git_available: v1.git_available,
                head_commit: v1.head_commit.clone(),
                rows: v1
                    .risks
                    .iter()
                    .map(|row| ProjectRiskRow {
                        path: row.path.clone(),
                        score: row.score,
                        components: v1_components(&row.components),
                        max_cognitive: Some(row.raw.max_cognitive),
                        max_cyclomatic: Some(row.raw.max_cyclomatic),
                        max_crap: row.raw.max_crap,
                        changes: row.raw.changes,
                        contributors: row.raw.contributors,
                        concentration_percent: None,
                        blast_radius: row.raw.blast_radius,
                        blast_radius_percent: row.raw.blast_radius_percent,
                        fan_in: row.raw.fan_in,
                        fan_out: row.raw.fan_out,
                        policy_severity: None,
                    })
                    .collect(),
            },
            Some(v1.model),
        ),
        (None, None) => (
            ProjectRisk {
                model: "change-risk-v1",
                window: "90d".to_owned(),
                git_available: false,
                head_commit: None,
                rows: Vec::new(),
            },
            None,
        ),
    };

    let coverage_functions = analysis
        .files
        .iter()
        .flat_map(|file| &file.functions)
        .count() as u64;
    let covered_functions = analysis
        .files
        .iter()
        .flat_map(|file| &file.functions)
        .filter(|function| function.metrics.coverage.is_some())
        .count() as u64;
    let coverage = if covered_functions == 0 {
        None
    } else {
        Some(CoverageSection {
            covered_functions,
            functions: coverage_functions,
            percent: Some(covered_functions as f64 / coverage_functions.max(1) as f64 * 100.0),
        })
    };

    let summary = ProjectSummary {
        files: files.len() as u64,
        functions: functions.len() as u64,
        parse_errors: files.iter().map(|file| file.parse_errors).sum(),
        dependency_edges: inputs.graph.edges.len() as u64,
        dependency_cycles: inputs.graph.cycles.len() as u64,
        coverage_percent: coverage.as_ref().and_then(|section| section.percent),
        mutation_score: inputs.mutation.and_then(|mutation| mutation.summary.score),
        duplication_groups: inputs.duplication.groups.len() as u64,
        duplicated_lines: inputs.duplication.duplicated_lines,
        policy_violations: inputs
            .policy
            .map(|policy| policy.violations.len() as u64)
            .unwrap_or(0),
        risk_model,
    };

    Project {
        meta: ProjectMeta {
            schema_version: PROJECT_SCHEMA_VERSION,
            metric_profile: analysis.metric_profile,
            analyzer_version: analysis.analyzer_version,
            generated_from: inputs.generated_from,
            git_available: inputs.git.is_some_and(|git| git.history.available),
            head_commit: inputs.git.and_then(|git| git.history.head_commit.clone()),
            head_timestamp: inputs.git.and_then(|git| git.history.head_timestamp),
        },
        summary,
        modules,
        files,
        functions,
        dependencies: ProjectDependencies {
            edges: inputs.graph.edges.clone(),
            unresolved: inputs.graph.unresolved.clone(),
        },
        cycles,
        git_activity,
        temporal_coupling,
        ownership: inputs.ownership.cloned(),
        coverage,
        mutation: inputs.mutation.cloned(),
        test_relationships: inputs.test_relationships.cloned(),
        duplication: inputs.duplication.clone(),
        architecture_violations: inputs
            .policy
            .map(|policy| policy.violations.clone())
            .unwrap_or_default(),
        risk,
        snapshots: inputs.snapshots,
    }
}

fn v1_components(components: &crate::risk::RiskComponents) -> BTreeMap<&'static str, Option<f64>> {
    [
        ("complexity", components.complexity),
        ("crap", components.crap),
        ("churn", components.churn),
        ("impact", components.impact),
        ("ownership", components.ownership),
        ("policy", components.policy),
    ]
    .into_iter()
    .collect()
}

fn v2_components(
    components: &crate::risk_v2::RiskV2Components,
) -> BTreeMap<&'static str, Option<f64>> {
    [
        ("complexity", components.complexity),
        ("crap", components.crap),
        ("churn", components.churn),
        ("impact", components.impact),
        ("ownership", components.ownership),
        ("policy", components.policy),
    ]
    .into_iter()
    .collect()
}

fn module_rows(files: &[ProjectFile]) -> Vec<ProjectModule> {
    let mut modules: BTreeMap<String, ProjectModule> = BTreeMap::new();
    modules.insert(
        ".".to_owned(),
        ProjectModule {
            path: ".".to_owned(),
            files: 0,
            functions: 0,
            loc: 0,
            max_cognitive: 0,
            max_cyclomatic: 0,
        },
    );
    for file in files {
        let parts: Vec<&str> = file.path.split('/').collect();
        let mut prefixes = vec![".".to_owned()];
        let mut prefix = String::new();
        for part in &parts[..parts.len().saturating_sub(1)] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            prefixes.push(prefix.clone());
        }
        for module_path in prefixes {
            let module = modules.entry(module_path.clone()).or_insert(ProjectModule {
                path: module_path,
                files: 0,
                functions: 0,
                loc: 0,
                max_cognitive: 0,
                max_cyclomatic: 0,
            });
            module.files += 1;
            module.functions += file.functions;
            module.loc += file.loc;
            module.max_cognitive = module.max_cognitive.max(file.max_cognitive);
            module.max_cyclomatic = module.max_cyclomatic.max(file.max_cyclomatic);
        }
    }
    modules.into_values().collect()
}
