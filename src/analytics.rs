//! Orchestration: load snapshots and build Project/debt/trend artifacts.
//!
//! Adapters and joins stay pure; this module owns the I/O order (snapshot,
//! analysis, graph, Git analytics, ownership, external reports, risk) so CLI
//! and future frontends share one pipeline.

use crate::Result;
use crate::debt::{
    DEBT_SCHEMA_VERSION, DebtReport, RISK_CHANGE_MODEL, classify, compare_risks, summarize,
};
use crate::duplication::detect;
use crate::external::InputBudget;
use crate::graph::analyze_dependencies_from_sources;
use crate::history::{HistoryWindow, analyze_git_at_with_mailmap};
use crate::mutation::{MutationInput, ingest as ingest_mutation};
use crate::ownership::{OwnershipMode, OwnershipReport, build as build_ownership};
use crate::policy::{PolicyReport, evaluate as evaluate_policy};
use crate::project::{Project, ProjectInputs, build as build_project};
use crate::risk::build as build_risk;
use crate::snapshots::{SnapshotOutcome, TrendPoint, TrendStore};
use crate::source_snapshot::{SnapshotContext, SnapshotTarget, SourceSnapshot, load};
use crate::test_relationships::ingest as ingest_test_relationships;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Requests one Project build from a repository state.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectRequest {
    pub path: PathBuf,
    pub target: SnapshotTarget,
    pub window: HistoryWindow,
    pub mutation_inputs: Vec<MutationInput>,
    pub test_maps: Vec<PathBuf>,
    pub ownership_mode: OwnershipMode,
    pub snapshots_path: Option<PathBuf>,
    pub coverage: Option<crate::coverage::CoverageMap>,
}

/// Loads a source snapshot and builds the canonical Project.
pub fn build(request: &ProjectRequest) -> Result<Project> {
    let (context, snapshot) = load(&request.path, request.target.clone())?;
    let analysis = crate::analyze_sources(&snapshot.entries, request.coverage.as_ref())?;
    let graph = analyze_dependencies_from_sources(&snapshot.entries)?;
    let mut budget = InputBudget::new();
    build_from_parts(&context, &snapshot, &analysis, &graph, request, &mut budget)
}

fn build_from_parts(
    context: &SnapshotContext,
    snapshot: &SourceSnapshot,
    analysis: &crate::core::AnalysisReport,
    graph: &crate::graph::DependencyReport,
    request: &ProjectRequest,
    budget: &mut InputBudget,
) -> Result<Project> {
    let revision = snapshot.commit.clone().unwrap_or_else(|| "HEAD".to_owned());
    let git = match context.repo_root {
        Some(_) => Some(analyze_git_at_with_mailmap(
            context,
            &revision,
            snapshot.mailmap_bytes.as_deref(),
        )?),
        None => None,
    };
    let source_paths: Vec<String> = analysis
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    let ownership = git
        .as_ref()
        .map(|git| build_ownership(&git.touches, &source_paths, request.ownership_mode));
    let mutation = if request.mutation_inputs.is_empty() {
        None
    } else {
        Some(ingest_mutation(
            &request.mutation_inputs,
            &snapshot.entries,
            analysis,
            budget,
        )?)
    };
    let test_relationships = if request.test_maps.is_empty() {
        None
    } else {
        Some(ingest_test_relationships(
            &request.test_maps,
            analysis,
            budget,
        )?)
    };
    let duplication = detect(&snapshot.entries, &context.config.duplication);
    let policy = evaluate_policy(graph, &context.config.architecture_rules);
    let no_ownership = OwnershipReport {
        files: Vec::new(),
        modules: Vec::new(),
    };
    let ownership_ref = ownership.as_ref().unwrap_or(&no_ownership);
    let unavailable = unavailable_history();
    let history_ref = git.as_ref().map(|git| &git.history).unwrap_or(&unavailable);
    let risk = build_risk(
        analysis,
        history_ref,
        graph,
        ownership_ref,
        &policy,
        request.window,
    );
    let snapshots = match &request.snapshots_path {
        Some(path) => Some(TrendStore::read(path)?.points),
        None => None,
    };
    Ok(build_project(ProjectInputs {
        analysis,
        generated_from: snapshot_label(&request.target),
        git: git.as_ref(),
        graph,
        ownership: ownership.as_ref(),
        mutation: mutation.as_ref(),
        test_relationships: test_relationships.as_ref(),
        duplication: &duplication,
        policy: &policy,
        risk: &risk,
        snapshots,
    }))
}

fn unavailable_history() -> crate::history::HistoryReport {
    crate::history::HistoryReport {
        schema_version: crate::history::HISTORY_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        available: false,
        reference: "head-commit-time",
        head_commit: None,
        head_timestamp: None,
        files: Vec::new(),
    }
}

fn snapshot_label(target: &SnapshotTarget) -> String {
    match target {
        SnapshotTarget::Worktree => "worktree".to_owned(),
        SnapshotTarget::Index => "index".to_owned(),
        SnapshotTarget::Revision(revision) => revision.clone(),
    }
}

/// Captures one trend point from the resolved HEAD tree.
pub fn capture_trend(
    path: &Path,
    output: &Path,
    window: HistoryWindow,
    mutation_inputs: &[MutationInput],
    replace: bool,
) -> Result<SnapshotOutcome> {
    let request = ProjectRequest {
        path: path.to_path_buf(),
        target: SnapshotTarget::Revision("HEAD".to_owned()),
        window,
        mutation_inputs: mutation_inputs.to_vec(),
        test_maps: Vec::new(),
        ownership_mode: OwnershipMode::AggregateOnly,
        snapshots_path: None,
        coverage: None,
    };
    let (context, snapshot) = load(&request.path, request.target.clone())?;
    let Some(commit) = snapshot.commit.clone() else {
        return Err("snapshot: HEAD must resolve to a commit".into());
    };
    let analysis = crate::analyze_sources(&snapshot.entries, None)?;
    let graph = analyze_dependencies_from_sources(&snapshot.entries)?;
    let mut budget = InputBudget::new();
    let project = build_from_parts(
        &context,
        &snapshot,
        &analysis,
        &graph,
        &request,
        &mut budget,
    )?;

    let source_hash = hash_source_entries(&snapshot.entries);
    let config_hash = hash_bytes(snapshot.config_bytes.as_deref().unwrap_or_default());
    let mailmap_hash = snapshot.mailmap_bytes.as_deref().map(hash_bytes);
    let input_fingerprint = hash_json(&serde_json::json!({
        "target": snapshot_label(&request.target),
        "commit": commit,
        "scope": scope_label(&context),
        "config": config_hash,
        "mailmap": mailmap_hash,
        "analyzer": env!("CARGO_PKG_VERSION"),
        "risk": project.risk.model,
        "duplication": project.duplication.profile,
        "mutation": project
            .mutation
            .as_ref()
            .map(|mutation| mutation.model)
            .unwrap_or("none"),
    }));

    let mut store = TrendStore::read(output)?;
    let point = TrendPoint::from_project(
        &project,
        scope_label(&context),
        config_hash,
        mailmap_hash,
        source_hash,
        input_fingerprint,
    );
    let outcome = store.append(point, replace)?;
    if outcome != SnapshotOutcome::Unchanged {
        store.write(output)?;
    }
    Ok(outcome)
}

fn scope_label(context: &SnapshotContext) -> String {
    let trimmed = context.scope_prefix.trim_end_matches('/');
    if trimmed.is_empty() {
        ".".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn hash_json(value: &serde_json::Value) -> String {
    hash_bytes(serde_json::to_vec(value).unwrap_or_default().as_slice())
}

fn hash_source_entries(entries: &[crate::source_snapshot::SourceEntry]) -> String {
    let mut hasher = blake3::Hasher::new();
    for entry in entries {
        hasher.update(entry.path.as_bytes());
        hasher.update(b"\x00");
        hasher.update(&(entry.bytes.len() as u64).to_be_bytes());
        hasher.update(&entry.bytes);
    }
    hasher.finalize().to_hex().to_string()
}

/// Full-state debt comparison between a base revision and a target state.
pub struct DebtRequest {
    pub path: PathBuf,
    pub base: String,
    pub target: SnapshotTarget,
    pub detect_renames: bool,
    pub window: HistoryWindow,
    pub fail_on_regression: bool,
}

/// Compares complete before/after states with the current configuration.
pub fn analyze_debt(request: &DebtRequest) -> Result<DebtReport> {
    let before = build(&ProjectRequest {
        path: request.path.clone(),
        target: SnapshotTarget::Revision(request.base.clone()),
        window: request.window,
        mutation_inputs: Vec::new(),
        test_maps: Vec::new(),
        ownership_mode: OwnershipMode::AggregateOnly,
        snapshots_path: None,
        coverage: None,
    })?;
    let after = build(&ProjectRequest {
        path: request.path.clone(),
        target: request.target.clone(),
        window: request.window,
        mutation_inputs: Vec::new(),
        test_maps: Vec::new(),
        ownership_mode: OwnershipMode::AggregateOnly,
        snapshots_path: None,
        coverage: None,
    })?;
    let renames = if request.detect_renames {
        detect_renames(&request.path, &request.base)?
    } else {
        BTreeMap::new()
    };

    let (before_context, before_snapshot) = load(
        &request.path,
        SnapshotTarget::Revision(request.base.clone()),
    )?;
    let (after_context, after_snapshot) = load(&request.path, request.target.clone())?;
    let before_analysis = crate::analyze_sources(&before_snapshot.entries, None)?;
    let after_analysis = crate::analyze_sources(&after_snapshot.entries, None)?;
    let before_files: Vec<&crate::core::FileAnalysis> = before_analysis.files.iter().collect();
    let after_files: Vec<&crate::core::FileAnalysis> = after_analysis.files.iter().collect();
    let thresholds = after_context.config.thresholds.clone();
    let (findings, unknown) = classify(&before_files, &after_files, &thresholds);

    // ponytail: both sides are loaded twice (Project build + threshold
    // classification); reuse one pair of analyses if this dominates profiles.
    let before_entries = project_risk_entries(&before);
    let after_entries = project_risk_entries(&after);
    let risk_changes = compare_risks(&before_entries, &after_entries, &renames);
    let summary = summarize(&findings, unknown, &risk_changes);
    let _ = (
        before_context,
        after_context,
        before_snapshot,
        after_snapshot,
    );
    Ok(DebtReport {
        schema_version: DEBT_SCHEMA_VERSION,
        metric_profile: after_analysis.metric_profile,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        model: RISK_CHANGE_MODEL,
        base: request.base.clone(),
        base_commit: before.meta.head_commit.clone(),
        target_commit: after.meta.head_commit.clone(),
        summary,
        findings,
        risk_changes,
    })
}

fn project_risk_entries(project: &Project) -> Vec<crate::debt::RiskSideEntry> {
    project
        .risk
        .rows
        .iter()
        .map(|row| crate::debt::RiskSideEntry {
            path: row.path.clone(),
            score: Some(row.score),
            components: row
                .components
                .iter()
                .map(|(name, value)| (*name, *value))
                .collect(),
        })
        .collect()
}

fn detect_renames(path: &Path, base: &str) -> Result<BTreeMap<String, String>> {
    let (context, _) = load(path, SnapshotTarget::Worktree)?;
    let Some(root) = context.repo_root.as_deref() else {
        return Ok(BTreeMap::new());
    };
    let output = crate::git::run(root, &["diff", "-M", "--name-status", "-z", base])?;
    parse_renames(&output.stdout)
}

fn parse_renames(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    let fields: Vec<String> = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| Ok(String::from_utf8(field.to_vec())?))
        .collect::<Result<_>>()?;
    let mut renames = BTreeMap::new();
    let mut index = 0;
    while index < fields.len() {
        if fields[index].starts_with('R') {
            let old = fields.get(index + 1).ok_or("malformed rename status")?;
            let new = fields.get(index + 2).ok_or("malformed rename status")?;
            renames.insert(old.clone(), new.clone());
            index += 3;
        } else {
            index += 2;
        }
    }
    Ok(renames)
}

/// Evaluates policy for one request's current state.
pub fn policy_for(request: &ProjectRequest) -> Result<PolicyReport> {
    let (context, snapshot) = load(&request.path, request.target.clone())?;
    let graph = analyze_dependencies_from_sources(&snapshot.entries)?;
    Ok(evaluate_policy(&graph, &context.config.architecture_rules))
}

/// Evaluates policy for one state, or `new`/`existing`/`resolved` drift when
/// `base` is given.
pub fn analyze_policy(
    path: &Path,
    target: SnapshotTarget,
    base: Option<&str>,
) -> Result<PolicyReport> {
    let (context, snapshot) = load(path, target.clone())?;
    let graph = analyze_dependencies_from_sources(&snapshot.entries)?;
    let rules = &context.config.architecture_rules;
    match base {
        Some(base) => {
            let (_, base_snapshot) = load(path, SnapshotTarget::Revision(base.to_owned()))?;
            let base_graph = analyze_dependencies_from_sources(&base_snapshot.entries)?;
            let renames = detect_renames(path, base).unwrap_or_default();
            Ok(crate::policy::compare(&base_graph, &graph, rules, &renames))
        }
        None => Ok(evaluate_policy(&graph, rules)),
    }
}

/// One duplication run: a current-state report or a drift comparison.
#[derive(Clone, Debug, PartialEq)]
pub enum DuplicationOutcome {
    Single(crate::duplication::DuplicationReport),
    Drift(crate::duplication::DuplicationDriftReport),
}

/// Detects clones for one state, or compares occurrences against `base`.
pub fn analyze_duplication(
    path: &Path,
    target: SnapshotTarget,
    base: Option<&str>,
) -> Result<DuplicationOutcome> {
    let (context, snapshot) = load(path, target.clone())?;
    let after = detect(&snapshot.entries, &context.config.duplication);
    match base {
        Some(base) => {
            let (base_context, base_snapshot) =
                load(path, SnapshotTarget::Revision(base.to_owned()))?;
            let before = detect(&base_snapshot.entries, &base_context.config.duplication);
            let renames = detect_renames(path, base).unwrap_or_default();
            Ok(DuplicationOutcome::Drift(crate::duplication::compare(
                &before, &after, &renames,
            )))
        }
        None => Ok(DuplicationOutcome::Single(after)),
    }
}

/// Normalizes every external mutation input plus optional test maps.
pub fn analyze_mutation(
    path: &Path,
    target: SnapshotTarget,
    mutation_inputs: &[MutationInput],
    test_maps: &[PathBuf],
) -> Result<(
    crate::mutation::MutationReport,
    Option<crate::test_relationships::TestRelationshipReport>,
)> {
    let (_, snapshot) = load(path, target)?;
    let analysis = crate::analyze_sources(&snapshot.entries, None)?;
    let mut budget = InputBudget::new();
    let mutation = ingest_mutation(mutation_inputs, &snapshot.entries, &analysis, &mut budget)?;
    let relationships = if test_maps.is_empty() {
        None
    } else {
        Some(ingest_test_relationships(
            test_maps,
            &analysis,
            &mut budget,
        )?)
    };
    Ok((mutation, relationships))
}
