//! Anonymous ownership concentration.
//!
//! Ownership answers "how concentrated is knowledge per file or module?" from
//! contributor touch counts. It never ranks developers: aggregate reports
//! carry counts and concentration only, author rows are opt-in, and
//! anonymized labels are artifact-local so they cannot correlate across
//! repositories.

use crate::history::FileTouches;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// How much contributor identity a report may expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipMode {
    /// Counts and concentration only.
    AggregateOnly,
    /// Per-file author rows with mailmap-resolved identities.
    IncludeAuthors,
    /// Per-file author rows with artifact-local `author-00N` labels.
    AnonymizeAuthors,
}

/// One contributor's touches of one file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuthorTouches {
    pub identity: String,
    pub touches: u64,
}

/// Concentration for one file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OwnershipFact {
    pub path: String,
    pub contributors: u64,
    pub concentration_percent: Option<f64>,
    pub bus_factor_50: Option<u64>,
    /// Present only in the author-bearing modes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<AuthorTouches>>,
}

/// Concentration for one directory prefix.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModuleOwnership {
    pub path: String,
    pub contributors: u64,
    pub concentration_percent: Option<f64>,
    pub bus_factor_50: Option<u64>,
}

/// Deterministic ownership report.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OwnershipReport {
    pub files: Vec<OwnershipFact>,
    pub modules: Vec<ModuleOwnership>,
}

/// Builds ownership for exactly the source paths the analysis produced.
pub fn build(
    touches: &[FileTouches],
    source_paths: &[String],
    mode: OwnershipMode,
) -> OwnershipReport {
    let by_path: BTreeMap<&str, &FileTouches> = touches
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();

    let mut labels: BTreeMap<&str, String> = BTreeMap::new();
    if mode == OwnershipMode::AnonymizeAuthors {
        let mut identities: BTreeSet<&str> = BTreeSet::new();
        for file in touches {
            for (identity, _) in &file.identities {
                identities.insert(identity.as_str());
            }
        }
        for (index, identity) in identities.into_iter().enumerate() {
            labels.insert(identity, format!("author-{:03}", index + 1));
        }
    }

    let mut files = Vec::with_capacity(source_paths.len());
    for path in source_paths {
        let identities = by_path
            .get(path.as_str())
            .map(|file| file.identities.as_slice())
            .unwrap_or(&[]);
        let (contributors, concentration, bus_factor) = concentration(identities);
        let authors = match mode {
            OwnershipMode::AggregateOnly => None,
            OwnershipMode::IncludeAuthors => Some(author_rows(identities, &BTreeMap::new())),
            OwnershipMode::AnonymizeAuthors => Some(author_rows(identities, &labels)),
        };
        files.push(OwnershipFact {
            path: path.clone(),
            contributors,
            concentration_percent: concentration,
            bus_factor_50: bus_factor,
            authors,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    OwnershipReport {
        files,
        modules: module_rows(source_paths, touches),
    }
}

fn author_rows(
    identities: &[(String, u64)],
    labels: &BTreeMap<&str, String>,
) -> Vec<AuthorTouches> {
    let mut rows: Vec<AuthorTouches> = identities
        .iter()
        .map(|(identity, touches)| AuthorTouches {
            identity: labels
                .get(identity.as_str())
                .cloned()
                .unwrap_or_else(|| identity.clone()),
            touches: *touches,
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .touches
            .cmp(&left.touches)
            .then_with(|| left.identity.cmp(&right.identity))
    });
    rows
}

/// `(contributors, concentration percent, bus factor)` for raw touch rows.
fn concentration(identities: &[(String, u64)]) -> (u64, Option<f64>, Option<u64>) {
    let total: u64 = identities.iter().map(|(_, touches)| *touches).sum();
    if total == 0 {
        return (0, None, None);
    }
    let mut rows: Vec<(&str, u64)> = identities
        .iter()
        .map(|(identity, touches)| (identity.as_str(), *touches))
        .collect();
    rows.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    let top = rows.first().map_or(0, |(_, touches)| *touches);
    let concentration = top as f64 / total as f64 * 100.0;
    let mut cumulative = 0_u64;
    let mut bus_factor = 0_u64;
    for (_, touches) in rows {
        cumulative += touches;
        bus_factor += 1;
        if cumulative * 2 >= total {
            break;
        }
    }
    let bus_factor = bus_factor.min(identities.len() as u64);
    (
        identities.len() as u64,
        Some(concentration),
        Some(bus_factor),
    )
}

fn module_rows(source_paths: &[String], touches: &[FileTouches]) -> Vec<ModuleOwnership> {
    let mut module_totals: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let by_path: BTreeMap<&str, &FileTouches> = touches
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    for path in source_paths {
        let mut prefixes = vec![".".to_owned()];
        let mut prefix = String::new();
        let parts: Vec<&str> = path.split('/').collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            prefixes.push(prefix.clone());
        }
        let identities = by_path
            .get(path.as_str())
            .map(|file| file.identities.as_slice())
            .unwrap_or(&[]);
        for module in prefixes {
            let totals = module_totals.entry(module).or_default();
            for (identity, count) in identities {
                *totals.entry(identity.clone()).or_default() += count;
            }
        }
    }
    module_totals
        .into_iter()
        .map(|(path, totals)| {
            let rows: Vec<(String, u64)> = totals.into_iter().collect();
            let (contributors, concentration_percent, bus_factor_50) = concentration(&rows);
            ModuleOwnership {
                path,
                contributors,
                concentration_percent,
                bus_factor_50,
            }
        })
        .collect()
}
