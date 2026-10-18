use leadline::config::DuplicationConfig;
use leadline::duplication::{GroupStatus, OccurrenceStatus, compare, detect, detect_with_limits};
use leadline::source_snapshot::SourceEntry;
use std::collections::BTreeMap;

fn entry(path: &str, source: &str) -> SourceEntry {
    SourceEntry {
        path: path.to_owned(),
        bytes: source.as_bytes().to_vec(),
    }
}

fn config(min_tokens: usize, min_lines: usize) -> DuplicationConfig {
    DuplicationConfig {
        min_tokens,
        min_lines,
        excludes: Vec::new(),
    }
}

const FIRST: &str = r#"
export function alpha(input: number): number {
  let total = 0;
  for (let index = 0; index < input; index += 1) {
    if (index % 2 === 0) {
      total += index * 3;
    } else {
      total -= index - 1;
    }
  }
  return total;
}
"#;

const SECOND: &str = r#"
export function beta(value: number): number {
  let sum = 0;
  for (let step = 0; step < value; step += 1) {
    if (step % 2 === 0) {
      sum += step * 3;
    } else {
      sum -= step - 1;
    }
  }
  return sum;
}
"#;

/// Two separated copies of a nine-token expression (`1 + 2 * 3 - 4 / 5`),
/// plus a third copy that starts on the second copy's final token and so
/// overlaps its already-recorded span.
const REPEATED: &str = r#"
function value(): number {
  return 1 + 2 * 3 - 4 / 5;
}
const limit = 10;
const total = 1 + 2 * 3 - 4 / 5 + 6 * 7 - 8 / 9 * limit;
"#;

#[test]
fn detects_type_two_clones_across_files() {
    let report = detect(
        &[entry("src/a.ts", FIRST), entry("src/b.ts", SECOND)],
        &config(12, 5),
    );
    assert!(report.complete, "{:?}", report.reason);
    assert_eq!(report.groups.len(), 1, "{:?}", report.groups);
    let group = &report.groups[0];
    assert_eq!(group.occurrences.len(), 2);
    assert!(group.occurrences[0].token_count >= 12);
    assert!(
        group
            .occurrences
            .iter()
            .all(|occurrence| occurrence.duplicated_lines >= 5)
    );
    assert!(report.duplicated_lines >= 10);
}

#[test]
fn identical_files_with_shared_content_form_one_group() {
    // Two identical files are one clone group with two occurrences; formatting
    // and identifier differences never split it.
    let report = detect(
        &[entry("src/a.ts", FIRST), entry("src/copy.ts", FIRST)],
        &config(12, 5),
    );
    assert!(report.complete);
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].occurrences.len(), 2);
    let paths: Vec<&str> = report.groups[0]
        .occurrences
        .iter()
        .map(|occurrence| occurrence.path.as_str())
        .collect();
    assert!(paths.contains(&"src/a.ts") && paths.contains(&"src/copy.ts"));
}

#[test]
fn repeated_runs_at_different_offsets_are_reported_once() {
    // Same-file repeats at different offsets share one group; the third copy
    // overlapping the second one must not add another occurrence.
    let report = detect(&[entry("src/repeat.ts", REPEATED)], &config(9, 1));
    assert!(report.complete, "{:?}", report.reason);
    assert_eq!(report.comparisons, 3, "{:?}", report.groups);
    assert_eq!(report.groups.len(), 1, "{:?}", report.groups);
    let group = &report.groups[0];
    assert_eq!(group.token_count, 9);
    assert_eq!(group.occurrences.len(), 2, "{:?}", group.occurrences);
    // The fixture's leading newline puts the copies on lines 3 and 6; the
    // third copy overlaps the second at its first token and is rejected.
    let spans: Vec<(u32, u32, usize, u32)> = group
        .occurrences
        .iter()
        .map(|occurrence| {
            (
                occurrence.start_line,
                occurrence.end_line,
                occurrence.token_count,
                occurrence.duplicated_lines,
            )
        })
        .collect();
    assert_eq!(spans, vec![(3, 3, 9, 1), (6, 6, 9, 1)]);
    assert_eq!(report.duplicated_lines, 2);
}

#[test]
fn near_misses_and_short_runs_do_not_group() {
    let report = detect(
        &[entry("src/a.ts", FIRST), entry("src/b.ts", SECOND)],
        &config(200, 5),
    );
    assert!(report.groups.is_empty());

    let small = detect(
        &[entry("src/a.ts", FIRST), entry("src/b.ts", SECOND)],
        &config(12, 40),
    );
    assert!(
        small.groups.is_empty(),
        "min_lines filters short spans: {:?}",
        small.groups
    );
}

#[test]
fn comments_and_parse_errors_are_excluded() {
    // Comments never change token streams.
    let commented = FIRST.replace("let total = 0;", "let total = 0; // comment");
    let report = detect(
        &[entry("src/a.ts", FIRST), entry("src/b.ts", &commented)],
        &config(12, 5),
    );
    assert_eq!(report.groups.len(), 1);

    // Parse errors exclude the file from candidates with a diagnostic.
    let broken = detect(
        &[
            entry("src/a.ts", FIRST),
            entry("src/bad.ts", "function ( {"),
        ],
        &config(12, 5),
    );
    assert!(broken.groups.is_empty());
    assert_eq!(broken.diagnostics.len(), 1);
    assert_eq!(broken.diagnostics[0].path, "src/bad.ts");
}

#[test]
fn ceilings_mark_reports_incomplete_instead_of_truncating() {
    let report = detect_with_limits(
        &[entry("src/a.ts", FIRST), entry("src/b.ts", SECOND)],
        &config(12, 5),
        1,
        1_000,
    );
    assert!(!report.complete);
    assert_eq!(report.reason.as_deref(), Some("token_ceiling_exceeded"));
    assert!(report.groups.is_empty());

    let comparisons = detect_with_limits(
        &[entry("src/a.ts", FIRST), entry("src/b.ts", SECOND)],
        &config(12, 5),
        1_000_000,
        1,
    );
    assert!(!comparisons.complete);
    assert_eq!(
        comparisons.reason.as_deref(),
        Some("comparison_ceiling_exceeded")
    );
}

#[test]
fn drift_ignores_line_shifts_and_applies_renames() {
    let before = detect(
        &[entry("src/a.ts", FIRST), entry("src/b.ts", SECOND)],
        &config(12, 5),
    );
    let shifted = format!("\n\n{FIRST}");
    let after = detect(
        &[entry("src/a.ts", &shifted), entry("src/b.ts", SECOND)],
        &config(12, 5),
    );
    let drift = compare(&before, &after, &BTreeMap::new());
    assert!(drift.complete);
    assert_eq!(drift.added_occurrences, 0, "line shifts are not new clones");
    assert_eq!(drift.removed_occurrences, 0);
    assert!(
        drift
            .groups
            .iter()
            .all(|group| group.status == Some(GroupStatus::Existing))
    );

    let mut renames = BTreeMap::new();
    renames.insert("src/a.ts".to_owned(), "src/renamed.ts".to_owned());
    let renamed = detect(
        &[entry("src/renamed.ts", FIRST), entry("src/b.ts", SECOND)],
        &config(12, 5),
    );
    let drift = compare(&before, &renamed, &renames);
    assert_eq!(drift.added_occurrences, 0);
    assert_eq!(drift.removed_occurrences, 0);
    assert!(
        drift
            .groups
            .iter()
            .flat_map(|group| &group.occurrences)
            .all(|occurrence| occurrence.path != "src/a.ts"
                || occurrence.status == Some(OccurrenceStatus::Existing))
    );

    let removed = compare(
        &before,
        &detect(&[entry("src/a.ts", FIRST)], &config(12, 5)),
        &BTreeMap::new(),
    );
    assert_eq!(removed.removed_occurrences, 2);
    assert!(
        removed
            .groups
            .iter()
            .any(|group| group.status == Some(GroupStatus::Resolved))
    );
}

#[test]
fn detection_is_deterministic() {
    let entries = [entry("src/a.ts", FIRST), entry("src/b.ts", SECOND)];
    let first = detect(&entries, &config(12, 5));
    let second = detect(&entries, &config(12, 5));
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
}
