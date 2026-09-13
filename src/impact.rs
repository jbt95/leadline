//! Transitive impact analysis over a resolved dependency graph.

use crate::graph::DependencyReport;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const IMPACT_SCHEMA_VERSION: u32 = 1;
pub const IMPACT_MODEL: &str = "impact";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImpactedFile {
    pub path: String,
    pub distance: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImpactReport {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub model: &'static str,
    pub target: String,
    pub files_analyzed: usize,
    pub fan_in: usize,
    pub fan_out: usize,
    pub direct_dependents: usize,
    pub blast_radius: usize,
    pub blast_radius_percent: f64,
    pub dependents: Vec<ImpactedFile>,
    pub cycles: Vec<Vec<String>>,
    pub truncated: bool,
}

/// Counts-only whole-scope impact over one shared reverse map.
///
/// `analyze_impact` rebuilds this map on every call; prefer this helper when
/// scoring every file in a scope so the map is built once.
pub fn impact_counts(graph: &DependencyReport) -> BTreeMap<String, ImpactCounts> {
    let reverse = reverse_adjacency(graph);
    let files_analyzed = graph.files.len();
    graph
        .files
        .iter()
        .map(|file| {
            let distances = reachable_distances(&reverse, file.path.as_str());
            let blast_radius = distances.len();
            let direct_dependents = distances
                .values()
                .filter(|&&distance| distance == 1)
                .count();
            let blast_radius_percent = if files_analyzed <= 1 {
                0.0
            } else {
                blast_radius as f64 / (files_analyzed - 1) as f64 * 100.0
            };
            (
                file.path.clone(),
                ImpactCounts {
                    direct_dependents,
                    blast_radius,
                    blast_radius_percent,
                },
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactCounts {
    pub direct_dependents: usize,
    pub blast_radius: usize,
    pub blast_radius_percent: f64,
}

fn reverse_adjacency(graph: &DependencyReport) -> BTreeMap<&str, Vec<&str>> {
    let mut reverse: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &graph.edges {
        reverse
            .entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }
    for dependents in reverse.values_mut() {
        dependents.sort_unstable();
        dependents.dedup();
    }
    reverse
}

fn reachable_distances<'graph>(
    reverse: &BTreeMap<&'graph str, Vec<&'graph str>>,
    target: &'graph str,
) -> BTreeMap<&'graph str, usize> {
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    visited.insert(target);
    let mut distances: BTreeMap<&str, usize> = BTreeMap::new();
    let mut queue: VecDeque<(&str, usize)> = VecDeque::from([(target, 0)]);
    while let Some((node, distance)) = queue.pop_front() {
        if let Some(dependents) = reverse.get(node) {
            for &dependent in dependents {
                if visited.insert(dependent) {
                    distances.insert(dependent, distance + 1);
                    queue.push_back((dependent, distance + 1));
                }
            }
        }
    }
    distances
}

/// Reports the transitive dependents of `target` by following reverse edges.
///
/// Returns `None` when `target` is not a file in `graph`.
pub fn analyze_impact(
    graph: &DependencyReport,
    target: &str,
    limit: usize,
) -> Option<ImpactReport> {
    let target_file = graph.files.iter().find(|file| file.path == target)?;
    let files_analyzed = graph.files.len();
    let reverse = reverse_adjacency(graph);
    let distances = reachable_distances(&reverse, target);

    let mut dependents: Vec<ImpactedFile> = distances
        .into_iter()
        .map(|(path, distance)| ImpactedFile {
            path: path.to_owned(),
            distance,
        })
        .collect();
    dependents
        .sort_by(|left, right| (left.distance, &left.path).cmp(&(right.distance, &right.path)));

    let blast_radius = dependents.len();
    let direct_dependents = dependents.iter().filter(|row| row.distance == 1).count();
    let blast_radius_percent = if files_analyzed <= 1 {
        0.0
    } else {
        blast_radius as f64 / (files_analyzed - 1) as f64 * 100.0
    };
    let truncated = dependents.len() > limit;
    dependents.truncate(limit);
    let cycles: Vec<Vec<String>> = graph
        .cycles
        .iter()
        .filter(|cycle| cycle.files.iter().any(|file| file == target))
        .map(|cycle| cycle.files.clone())
        .collect();

    Some(ImpactReport {
        schema_version: IMPACT_SCHEMA_VERSION,
        metric_profile: graph.metric_profile,
        analyzer_version: graph.analyzer_version,
        model: IMPACT_MODEL,
        target: target.to_owned(),
        files_analyzed,
        fan_in: target_file.fan_in,
        fan_out: target_file.fan_out,
        direct_dependents,
        blast_radius,
        blast_radius_percent,
        dependents,
        cycles,
        truncated,
    })
}
