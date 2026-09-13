//! Architecture policy: dependency rules with drift classification.
//!
//! Rules are evaluated in declaration order over high-confidence resolved
//! graph edges. A violation affects the edge's source endpoint for scoring.

use crate::config::{ArchitectureRule, Severity};
use crate::graph::DependencyReport;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const POLICY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyStatus {
    New,
    Existing,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PolicyViolation {
    pub rule: String,
    pub severity: Severity,
    pub source: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PolicyStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PolicyReport {
    pub schema_version: u32,
    pub analyzer_version: &'static str,
    pub rules: usize,
    pub info: u64,
    pub warning: u64,
    pub error: u64,
    pub violations: Vec<PolicyViolation>,
}

/// Evaluates every rule against one dependency graph.
pub fn evaluate(graph: &DependencyReport, rules: &[ArchitectureRule]) -> PolicyReport {
    let matchers = rules
        .iter()
        .map(|rule| CompiledRule {
            name: rule.name.clone(),
            severity: rule.severity,
            source: compile(&rule.source),
            deny: rule.deny.iter().map(|pattern| compile(pattern)).collect(),
        })
        .collect::<Vec<_>>();

    let mut violations = Vec::new();
    for edge in &graph.edges {
        if edge.confidence != "high" {
            continue;
        }
        for rule in &matchers {
            if !rule.source.matches(&edge.source) {
                continue;
            }
            if !rule.deny.iter().any(|deny| deny.matches(&edge.target)) {
                continue;
            }
            violations.push(PolicyViolation {
                rule: rule.name.clone(),
                severity: rule.severity,
                source: edge.source.clone(),
                target: edge.target.clone(),
                status: None,
            });
        }
    }
    finalize(violations, matchers.len())
}

/// Evaluates both sides with one rule set and classifies drift.
///
/// `renames` maps a base path to its target path; both edge endpoints pass
/// through it before identity comparison, so a pure rename stays `existing`.
pub fn compare(
    before: &DependencyReport,
    after: &DependencyReport,
    rules: &[ArchitectureRule],
    renames: &BTreeMap<String, String>,
) -> PolicyReport {
    let before_report = evaluate(before, rules);
    let mut after_report = evaluate(after, rules);
    let before_keys: std::collections::BTreeSet<(String, String, String)> = before_report
        .violations
        .iter()
        .map(|violation| {
            (
                violation.rule.clone(),
                mapped_path(renames, &violation.source),
                mapped_path(renames, &violation.target),
            )
        })
        .collect();
    let after_keys: std::collections::BTreeSet<(String, String, String)> = after_report
        .violations
        .iter()
        .map(|violation| {
            (
                violation.rule.clone(),
                violation.source.clone(),
                violation.target.clone(),
            )
        })
        .collect();
    let mut statuses: BTreeMap<(String, String, String), PolicyStatus> = BTreeMap::new();
    for key in &after_keys {
        statuses.insert(
            key.clone(),
            if before_keys.contains(key) {
                PolicyStatus::Existing
            } else {
                PolicyStatus::New
            },
        );
    }
    for key in &before_keys {
        statuses
            .entry(key.clone())
            .or_insert(PolicyStatus::Resolved);
    }
    for violation in &mut after_report.violations {
        violation.status = statuses
            .get(&(
                violation.rule.clone(),
                violation.source.clone(),
                violation.target.clone(),
            ))
            .copied();
    }
    let leftovers: Vec<PolicyViolation> = before_keys
        .into_iter()
        .filter(|key| !after_keys.contains(key))
        .map(|(rule, source, target)| PolicyViolation {
            severity: rules
                .iter()
                .find(|candidate| candidate.name == rule)
                .map(|candidate| candidate.severity)
                .unwrap_or(Severity::Info),
            rule,
            source,
            target,
            status: Some(PolicyStatus::Resolved),
        })
        .collect();
    after_report.violations.extend(leftovers);
    after_report.violations.sort_by(|left, right| {
        left.rule
            .cmp(&right.rule)
            .then(left.source.cmp(&right.source))
            .then(left.target.cmp(&right.target))
    });
    finalize_with_status(after_report, rules.len())
}

fn mapped_path(renames: &BTreeMap<String, String>, path: &str) -> String {
    renames
        .get(path)
        .cloned()
        .unwrap_or_else(|| path.to_owned())
}

fn finalize(mut violations: Vec<PolicyViolation>, rules: usize) -> PolicyReport {
    violations.sort_by(|left, right| {
        left.rule
            .cmp(&right.rule)
            .then(left.source.cmp(&right.source))
            .then(left.target.cmp(&right.target))
    });
    let (info, warning, error) = count_severities(&violations);
    PolicyReport {
        schema_version: POLICY_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        rules,
        info,
        warning,
        error,
        violations,
    }
}

fn finalize_with_status(mut report: PolicyReport, rules: usize) -> PolicyReport {
    let counts = count_severities(&report.violations);
    report.rules = rules;
    report.info = counts.0;
    report.warning = counts.1;
    report.error = counts.2;
    report
}

fn count_severities(violations: &[PolicyViolation]) -> (u64, u64, u64) {
    let mut info = 0;
    let mut warning = 0;
    let mut error = 0;
    for violation in violations {
        match violation.severity {
            Severity::Info => info += 1,
            Severity::Warning => warning += 1,
            Severity::Error => error += 1,
        }
    }
    (info, warning, error)
}

struct CompiledRule {
    name: String,
    severity: Severity,
    source: Matcher,
    deny: Vec<Matcher>,
}

struct Matcher {
    matcher: ignore::gitignore::Gitignore,
}

impl Matcher {
    fn matches(&self, path: &str) -> bool {
        self.matcher
            .matched_path_or_any_parents(Path::new(path), false)
            .is_ignore()
    }
}

fn compile(pattern: &str) -> Matcher {
    let mut builder = ignore::gitignore::GitignoreBuilder::new("");
    let _ = builder.add_line(None, pattern);
    let matcher = builder.build().unwrap_or_else(|_| {
        let mut empty = ignore::gitignore::GitignoreBuilder::new("");
        empty.build().expect("empty matcher")
    });
    Matcher { matcher }
}
