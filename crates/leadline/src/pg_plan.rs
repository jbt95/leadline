//! Bounded PostgreSQL `EXPLAIN (FORMAT JSON)` directory loader.
//!
//! Reads raw plan artifacts only. Leadline never connects to PostgreSQL,
//! executes SQL, or shells out to `psql`.

use crate::external::{INPUT_BYTES_LIMIT, InputBudget, read_bounded, validate_json_depth};
use serde::Serialize;
use std::path::Path;

/// Maximum plan files per directory.
const MAX_PLAN_FILES: usize = 200;
/// Maximum plan-tree nesting depth.
const MAX_PLAN_DEPTH: usize = 128;

/// One raw plan node with only the facts later stages need.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PgPlanNode {
    pub node_type: String,
    pub relation_name: Option<String>,
    pub index_name: Option<String>,
    pub total_cost: Option<f64>,
    pub plan_rows: Option<u64>,
    pub actual_rows: Option<u64>,
    pub actual_loops: Option<u64>,
    pub children: Vec<PgPlanNode>,
}

/// One query plan keyed by its filename-derived query ID.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QueryPlan {
    pub query_id: String,
    pub plan: PgPlanNode,
}

/// Load a flat directory of raw `EXPLAIN (FORMAT JSON)` files.
///
/// The query ID is the UTF-8 filename without `.json`. Non-`.json` regular
/// files are ignored. Symlinks, nested directories, non-file entries,
/// non-UTF-8 names, and empty or control-character stems are rejected.
pub fn load_plan_directory(path: &Path) -> crate::Result<Vec<QueryPlan>> {
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("cannot list plan directory {}: {error}", path.display()))?;
    let mut budget = InputBudget::new();
    let mut plans = Vec::new();
    let mut json_files = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read plan entry: {error}"))?;
        let file_name = entry.file_name();
        let name = file_name.to_str().ok_or_else(|| {
            format!(
                "plan filename is not valid UTF-8: {}",
                entry.path().display()
            )
        })?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot stat plan entry {name:?}: {error}"))?;
        if file_type.is_symlink() {
            return Err(format!("plan entry is a symlink: {name:?}").into());
        }
        if file_type.is_dir() {
            return Err(format!("plan directory must be flat: {name:?}").into());
        }
        if !file_type.is_file() {
            return Err(format!("plan entry is not a file: {name:?}").into());
        }
        if !name.ends_with(".json") {
            continue;
        }
        let stem = name.strip_suffix(".json").unwrap_or("");
        if stem.is_empty() || stem.chars().any(|c| c.is_control()) {
            return Err(format!("invalid plan filename: {name:?}").into());
        }
        json_files += 1;
        if json_files > MAX_PLAN_FILES {
            return Err(format!("too many plan files (max {MAX_PLAN_FILES})").into());
        }
        let bytes = read_bounded(&entry.path(), INPUT_BYTES_LIMIT, &mut budget)?;
        validate_json_depth(&bytes)?;
        let plan = parse_explain(&bytes, stem)?;
        plans.push(plan);
    }
    plans.sort_by(|a, b| a.query_id.cmp(&b.query_id));
    Ok(plans)
}

fn parse_explain(bytes: &[u8], query_id: &str) -> crate::Result<QueryPlan> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid plan JSON {query_id:?}: {error}"))?;
    let statements = value.as_array().ok_or_else(|| {
        format!("invalid plan JSON {query_id:?}: top level must be a one-element array")
    })?;
    if statements.len() != 1 {
        return Err(format!(
            "invalid plan JSON {query_id:?}: expected one statement, found {}",
            statements.len()
        )
        .into());
    }
    let plan_value = statements[0]
        .get("Plan")
        .ok_or_else(|| format!("invalid plan JSON {query_id:?}: missing \"Plan\" object"))?;
    Ok(QueryPlan {
        query_id: query_id.to_owned(),
        plan: parse_node(plan_value, query_id, 0)?,
    })
}

fn parse_node(
    value: &serde_json::Value,
    query_id: &str,
    depth: usize,
) -> crate::Result<PgPlanNode> {
    if depth > MAX_PLAN_DEPTH {
        return Err(format!("plan nesting exceeds {MAX_PLAN_DEPTH} levels: {query_id:?}").into());
    }
    let object = value
        .as_object()
        .ok_or_else(|| format!("invalid plan node in {query_id:?}: expected an object"))?;
    let node_type = object
        .get("Node Type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("invalid plan node in {query_id:?}: missing \"Node Type\""))?;
    if node_type.is_empty() {
        return Err(format!("invalid plan node in {query_id:?}: empty \"Node Type\"").into());
    }
    let mut children = Vec::new();
    if let Some(plans) = object.get("Plans") {
        let list = plans.as_array().ok_or_else(|| {
            format!("invalid plan node in {query_id:?}: \"Plans\" must be an array")
        })?;
        for child in list {
            children.push(parse_node(child, query_id, depth + 1)?);
        }
    }
    Ok(PgPlanNode {
        node_type: node_type.to_owned(),
        relation_name: qualified_relation(object),
        index_name: optional_name(object, "Index Name"),
        total_cost: optional_cost(object, query_id)?,
        plan_rows: optional_count(object, "Plan Rows", query_id)?,
        actual_rows: optional_count(object, "Actual Rows", query_id)?,
        actual_loops: optional_count(object, "Actual Loops", query_id)?,
        children,
    })
}

fn optional_name(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    object.get(key).and_then(|v| v.as_str()).map(str::to_owned)
}

/// `schema.relation` when EXPLAIN carries a schema, else the bare relation.
///
/// Same-named tables in different schemas must stay distinct scan identities.
fn qualified_relation(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let relation = optional_name(object, "Relation Name")?;
    match optional_name(object, "Schema Name") {
        Some(schema) if !schema.is_empty() => Some(format!("{schema}.{relation}")),
        _ => Some(relation),
    }
}

fn optional_cost(
    object: &serde_json::Map<String, serde_json::Value>,
    query_id: &str,
) -> crate::Result<Option<f64>> {
    let Some(value) = object.get("Total Cost") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let cost = value.as_f64().ok_or_else(|| {
        format!("invalid plan JSON {query_id:?}: \"Total Cost\" must be a number")
    })?;
    if !cost.is_finite() || cost < 0.0 {
        return Err(format!(
            "invalid plan JSON {query_id:?}: \"Total Cost\" must be finite and non-negative"
        )
        .into());
    }
    Ok(Some(cost))
}

fn optional_count(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    query_id: &str,
) -> crate::Result<Option<u64>> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(n) = value.as_u64() {
        return Ok(Some(n));
    }
    if let Some(n) = value.as_i64()
        && n >= 0
    {
        return Ok(Some(n as u64));
    }
    if let Some(f) = value.as_f64()
        && f.is_finite()
        && f >= 0.0
        && f.fract() == 0.0
        && f <= u64::MAX as f64
    {
        return Ok(Some(f as u64));
    }
    Err(format!("invalid plan JSON {query_id:?}: {key:?} must be a non-negative integer").into())
}

/// Scan strategy observed for one relation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanKind {
    Sequential,
    Index,
    IndexOnly,
    Bitmap,
}

/// Stable facts reduced from one plan tree.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PlanFacts {
    pub total_cost: Option<f64>,
    pub plan_rows: Option<u64>,
    pub sort_nodes: u64,
    pub join_counts: std::collections::BTreeMap<String, u64>,
    pub relation_scans: std::collections::BTreeMap<String, std::collections::BTreeSet<ScanKind>>,
    pub max_estimate_error_ratio: Option<f64>,
    pub estimate_error_unbounded: bool,
}

/// One relation with its observed scan strategies, for report projections.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RelationScan {
    pub relation: String,
    pub scans: std::collections::BTreeSet<ScanKind>,
}

impl PlanFacts {
    /// Deterministic per-relation scan rows sorted by relation name.
    pub fn relation_scan_rows(&self) -> Vec<RelationScan> {
        self.relation_scans
            .iter()
            .map(|(relation, scans)| RelationScan {
                relation: relation.clone(),
                scans: scans.clone(),
            })
            .collect()
    }
}

/// Reduce one plan tree to stable comparison facts with one iterative walk.
///
/// Top-level cost and rows come only from the root node.
pub fn extract_plan_facts(plan: &QueryPlan) -> PlanFacts {
    let mut facts = PlanFacts {
        total_cost: plan.plan.total_cost,
        plan_rows: plan.plan.plan_rows,
        ..PlanFacts::default()
    };
    let mut stack = vec![&plan.plan];
    while let Some(node) = stack.pop() {
        match node.node_type.as_str() {
            "Sort" | "Incremental Sort" => facts.sort_nodes += 1,
            "Nested Loop" | "Hash Join" | "Merge Join" => {
                *facts.join_counts.entry(node.node_type.clone()).or_default() += 1;
            }
            _ => {}
        }
        if let (Some(relation), Some(kind)) =
            (node.relation_name.as_deref(), scan_kind(&node.node_type))
        {
            facts
                .relation_scans
                .entry(relation.to_owned())
                .or_default()
                .insert(kind);
        }
        if let (Some(estimated), Some(actual_rows)) = (node.plan_rows, node.actual_rows) {
            // `Plan Rows` and `Actual Rows` are both per-execution values:
            // PostgreSQL reports actual rows averaged over `Actual Loops`,
            // so equal-loop nodes stay comparable. Multiplying by loops
            // would inflate nested-loop estimates and fabricate violations.
            let estimated = estimated as f64;
            let actual_total = actual_rows as f64;
            if estimated == 0.0 && actual_total == 0.0 {
                facts.max_estimate_error_ratio =
                    Some(facts.max_estimate_error_ratio.unwrap_or(1.0).max(1.0));
            } else if estimated == 0.0 || actual_total == 0.0 {
                facts.estimate_error_unbounded = true;
            } else {
                let ratio = (actual_total / estimated).max(estimated / actual_total);
                facts.max_estimate_error_ratio =
                    Some(facts.max_estimate_error_ratio.unwrap_or(ratio).max(ratio));
            }
        }
        stack.extend(node.children.iter());
    }
    facts
}

fn scan_kind(node_type: &str) -> Option<ScanKind> {
    match node_type {
        "Seq Scan" => Some(ScanKind::Sequential),
        "Index Scan" => Some(ScanKind::Index),
        "Index Only Scan" => Some(ScanKind::IndexOnly),
        name if name.starts_with("Bitmap") => Some(ScanKind::Bitmap),
        _ => None,
    }
}

/// Calibrated gate limits. `None` disables a dimension.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanLimits {
    pub max_cost_increase_percent: Option<f64>,
    pub max_plan_rows_ratio: Option<f64>,
    pub max_estimate_error_ratio: Option<f64>,
}

/// Typed plan change. Declaration order is the stable violation rank.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRegressionKind {
    IndexToSequentialScan,
    CostIncrease,
    RowGrowth,
    EstimateError,
    AddedSort,
    JoinStrategyChange,
}

/// Match status of one query ID across the two directories.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryPlanStatus {
    Unchanged,
    Changed,
    New,
    Removed,
}

/// One typed change for one query.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QueryPlanChange {
    pub query_id: String,
    pub kind: PlanRegressionKind,
    pub relation: Option<String>,
    pub detail: Option<String>,
    pub baseline: Option<f64>,
    pub current: Option<f64>,
    pub violates_gate: bool,
}

/// One query row in the comparison report.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QueryPlanEntry {
    pub query_id: String,
    pub status: QueryPlanStatus,
    pub changes: Vec<QueryPlanChange>,
}

/// Full comparison report: per-query rows plus flat gate violations.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PgPlanReport {
    pub queries: Vec<QueryPlanEntry>,
    pub violations: Vec<QueryPlanChange>,
}

/// Compare current and baseline plan directories query by query.
///
/// New and removed queries are informational and never violate. Sort and
/// join-strategy changes violate only when the same query also violates an
/// enabled numeric limit; index-to-sequential scans always violate.
pub fn compare_plan_directories(
    current: &Path,
    baseline: &Path,
    limits: &PlanLimits,
) -> crate::Result<PgPlanReport> {
    let current_plans = load_plan_directory(current)?;
    let baseline_plans = load_plan_directory(baseline)?;
    let baseline_by_id: std::collections::BTreeMap<&str, &QueryPlan> = baseline_plans
        .iter()
        .map(|plan| (plan.query_id.as_str(), plan))
        .collect();
    let current_ids: std::collections::BTreeSet<&str> = current_plans
        .iter()
        .map(|plan| plan.query_id.as_str())
        .collect();
    let mut report = PgPlanReport {
        queries: Vec::new(),
        violations: Vec::new(),
    };
    for plan in &current_plans {
        match baseline_by_id.get(plan.query_id.as_str()) {
            None => report.queries.push(QueryPlanEntry {
                query_id: plan.query_id.clone(),
                status: QueryPlanStatus::New,
                changes: Vec::new(),
            }),
            Some(base) => {
                let changes = compare_query(plan, base, limits);
                let status = if changes.is_empty() {
                    QueryPlanStatus::Unchanged
                } else {
                    QueryPlanStatus::Changed
                };
                report
                    .violations
                    .extend(changes.iter().filter(|c| c.violates_gate).cloned());
                report.queries.push(QueryPlanEntry {
                    query_id: plan.query_id.clone(),
                    status,
                    changes,
                });
            }
        }
    }
    for plan in &baseline_plans {
        if !current_ids.contains(plan.query_id.as_str()) {
            report.queries.push(QueryPlanEntry {
                query_id: plan.query_id.clone(),
                status: QueryPlanStatus::Removed,
                changes: Vec::new(),
            });
        }
    }
    report.queries.sort_by(|a, b| a.query_id.cmp(&b.query_id));
    report.violations.sort_by(|a, b| {
        a.query_id
            .cmp(&b.query_id)
            .then(a.kind.cmp(&b.kind))
            .then(a.relation.cmp(&b.relation))
            .then(a.detail.cmp(&b.detail))
    });
    Ok(report)
}

fn compare_query(
    current: &QueryPlan,
    baseline: &QueryPlan,
    limits: &PlanLimits,
) -> Vec<QueryPlanChange> {
    let id = current.query_id.as_str();
    let current_facts = extract_plan_facts(current);
    let baseline_facts = extract_plan_facts(baseline);
    let mut changes = Vec::new();
    // Index-to-sequential regressions always fail.
    let mut relations: std::collections::BTreeSet<&str> = baseline_facts
        .relation_scans
        .keys()
        .map(String::as_str)
        .collect();
    relations.extend(current_facts.relation_scans.keys().map(String::as_str));
    for relation in relations {
        let fast = |kinds: &std::collections::BTreeSet<ScanKind>| {
            kinds.contains(&ScanKind::Index)
                || kinds.contains(&ScanKind::IndexOnly)
                || kinds.contains(&ScanKind::Bitmap)
        };
        let was_fast = baseline_facts
            .relation_scans
            .get(relation)
            .is_some_and(fast);
        let now_seq_only = current_facts
            .relation_scans
            .get(relation)
            .is_some_and(|kinds| {
                *kinds == std::collections::BTreeSet::from([ScanKind::Sequential])
            });
        if was_fast && now_seq_only {
            changes.push(QueryPlanChange {
                query_id: id.to_owned(),
                kind: PlanRegressionKind::IndexToSequentialScan,
                relation: Some(relation.to_owned()),
                detail: Some("index scan replaced by sequential scan".to_owned()),
                baseline: None,
                current: None,
                violates_gate: true,
            });
        }
    }
    // Total cost increases fail past the configured percent.
    if let (Some(base), Some(now)) = (baseline_facts.total_cost, current_facts.total_cost) {
        if now > base {
            let unbounded = base == 0.0;
            let percent = if unbounded {
                f64::INFINITY
            } else {
                (now - base) / base * 100.0
            };
            changes.push(QueryPlanChange {
                query_id: id.to_owned(),
                kind: PlanRegressionKind::CostIncrease,
                relation: None,
                detail: Some(if unbounded {
                    "unbounded increase from zero baseline".to_owned()
                } else {
                    format!("+{percent:.1}%")
                }),
                baseline: Some(base),
                current: Some(now),
                violates_gate: limits
                    .max_cost_increase_percent
                    .is_some_and(|limit| unbounded || percent > limit),
            });
        } else if now < base {
            changes.push(QueryPlanChange {
                query_id: id.to_owned(),
                kind: PlanRegressionKind::CostIncrease,
                relation: None,
                detail: Some("cost decreased".to_owned()),
                baseline: Some(base),
                current: Some(now),
                violates_gate: false,
            });
        }
    }
    // Plan-row growth fails past the configured ratio.
    if let (Some(base), Some(now)) = (baseline_facts.plan_rows, current_facts.plan_rows)
        && now != base
    {
        let unbounded = base == 0;
        let ratio = if unbounded {
            f64::INFINITY
        } else {
            now as f64 / base as f64
        };
        let fails = now > base
            && limits
                .max_plan_rows_ratio
                .is_some_and(|limit| unbounded || ratio > limit);
        changes.push(QueryPlanChange {
            query_id: id.to_owned(),
            kind: PlanRegressionKind::RowGrowth,
            relation: None,
            detail: Some(if unbounded {
                "unbounded growth from zero baseline".to_owned()
            } else if now < base {
                format!("decreased to {ratio:.2}x")
            } else {
                format!("{ratio:.2}x")
            }),
            baseline: Some(base as f64),
            current: Some(now as f64),
            violates_gate: fails,
        });
    }
    // Estimate error is absolute against the current plan, never the baseline.
    if let Some(limit) = limits.max_estimate_error_ratio {
        if current_facts.estimate_error_unbounded {
            changes.push(QueryPlanChange {
                query_id: id.to_owned(),
                kind: PlanRegressionKind::EstimateError,
                relation: None,
                detail: Some("unbounded estimate error".to_owned()),
                baseline: None,
                current: None,
                violates_gate: true,
            });
        } else if current_facts
            .max_estimate_error_ratio
            .is_some_and(|ratio| ratio > limit)
        {
            let ratio = current_facts.max_estimate_error_ratio.unwrap_or(1.0);
            changes.push(QueryPlanChange {
                query_id: id.to_owned(),
                kind: PlanRegressionKind::EstimateError,
                relation: None,
                detail: Some(format!("{ratio:.1}x")),
                baseline: None,
                current: Some(ratio),
                violates_gate: true,
            });
        }
    }
    // Added sorts are findings; they fail only beside a numeric violation.
    if current_facts.sort_nodes > baseline_facts.sort_nodes {
        changes.push(QueryPlanChange {
            query_id: id.to_owned(),
            kind: PlanRegressionKind::AddedSort,
            relation: None,
            detail: Some(format!(
                "sort nodes {} -> {}",
                baseline_facts.sort_nodes, current_facts.sort_nodes
            )),
            baseline: Some(baseline_facts.sort_nodes as f64),
            current: Some(current_facts.sort_nodes as f64),
            violates_gate: false,
        });
    }
    // Join strategy transitions behave like added sorts.
    let mut joins: std::collections::BTreeSet<&str> = baseline_facts
        .join_counts
        .keys()
        .map(String::as_str)
        .collect();
    joins.extend(current_facts.join_counts.keys().map(String::as_str));
    for join in joins {
        let base = baseline_facts.join_counts.get(join).copied().unwrap_or(0);
        let now = current_facts.join_counts.get(join).copied().unwrap_or(0);
        if base != now {
            changes.push(QueryPlanChange {
                query_id: id.to_owned(),
                kind: PlanRegressionKind::JoinStrategyChange,
                relation: None,
                detail: Some(format!("{join}: {base} -> {now}")),
                baseline: Some(base as f64),
                current: Some(now as f64),
                violates_gate: false,
            });
        }
    }
    // Structural changes ride along with numeric failures in the same query.
    let numeric_fails = changes.iter().any(|change| {
        change.violates_gate
            && matches!(
                change.kind,
                PlanRegressionKind::CostIncrease
                    | PlanRegressionKind::RowGrowth
                    | PlanRegressionKind::EstimateError
            )
    });
    if numeric_fails {
        for change in &mut changes {
            if matches!(
                change.kind,
                PlanRegressionKind::AddedSort | PlanRegressionKind::JoinStrategyChange
            ) {
                change.violates_gate = true;
            }
        }
    }
    changes.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(a.relation.cmp(&b.relation))
            .then(a.detail.cmp(&b.detail))
    });
    changes
}

impl PlanRegressionKind {
    /// Stable snake_case identifier shared by JSON, SARIF, and terminal output.
    pub fn as_str(self) -> &'static str {
        match self {
            PlanRegressionKind::IndexToSequentialScan => "index_to_sequential_scan",
            PlanRegressionKind::CostIncrease => "cost_increase",
            PlanRegressionKind::RowGrowth => "row_growth",
            PlanRegressionKind::EstimateError => "estimate_error",
            PlanRegressionKind::AddedSort => "added_sort",
            PlanRegressionKind::JoinStrategyChange => "join_strategy_change",
        }
    }
}

/// Bounded agent projection: non-unchanged queries capped at `top`.
pub fn agent_json(report: &PgPlanReport, top: usize) -> serde_json::Value {
    let notable: Vec<&QueryPlanEntry> = report
        .queries
        .iter()
        .filter(|entry| entry.status != QueryPlanStatus::Unchanged)
        .collect();
    let truncated = notable.len() > top;
    serde_json::json!({
        "queries": notable.into_iter().take(top).collect::<Vec<_>>(),
        "violations": report.violations,
        "truncated": truncated,
    })
}

/// Human-readable terminal rendering of changed queries and violations.
pub fn terminal_text(report: &PgPlanReport) -> String {
    let mut out = String::new();
    for entry in &report.queries {
        if entry.status == QueryPlanStatus::Unchanged {
            continue;
        }
        let status = match entry.status {
            QueryPlanStatus::Changed => "changed",
            QueryPlanStatus::New => "new",
            QueryPlanStatus::Removed => "removed",
            QueryPlanStatus::Unchanged => "unchanged",
        };
        out.push_str(&format!(
            "{} ({})
",
            entry.query_id, status
        ));
        for change in &entry.changes {
            let marker = if change.violates_gate { "!" } else { "-" };
            let kind = change.kind.as_str();
            match &change.detail {
                Some(detail) => out.push_str(&format!(
                    "  {marker} {kind}: {detail}
"
                )),
                None => out.push_str(&format!(
                    "  {marker} {kind}
"
                )),
            }
        }
    }
    if out.is_empty() {
        out.push_str(
            "No plan changes found.
",
        );
    }
    out
}
