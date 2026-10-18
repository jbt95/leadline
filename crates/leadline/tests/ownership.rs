use leadline::history::FileTouches;
use leadline::ownership::{OwnershipMode, build};

fn touches(path: &str, rows: &[(&str, u64)]) -> FileTouches {
    FileTouches {
        path: path.to_owned(),
        identities: rows
            .iter()
            .map(|(identity, count)| ((*identity).to_owned(), *count))
            .collect(),
    }
}

fn paths(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn concentration_and_bus_factor_use_touch_distribution() {
    let report = build(
        &[touches("src/a.ts", &[("a", 6), ("b", 3), ("c", 1)])],
        &paths(&["src/a.ts"]),
        OwnershipMode::AggregateOnly,
    );
    let file = &report.files[0];
    assert_eq!(file.contributors, 3);
    assert_eq!(file.concentration_percent, Some(60.0));
    assert_eq!(file.bus_factor_50, Some(1));
    assert!(file.authors.is_none());
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("\"a\""), "aggregate output leaks identities");
}

#[test]
fn zero_touch_files_report_unknown_concentration() {
    let report = build(&[], &paths(&["src/new.ts"]), OwnershipMode::AggregateOnly);
    let file = &report.files[0];
    assert_eq!(file.contributors, 0);
    assert_eq!(file.concentration_percent, None);
    assert_eq!(file.bus_factor_50, None);
}

#[test]
fn bus_factor_handles_exact_half_split() {
    let report = build(
        &[touches("src/a.ts", &[("a", 5), ("b", 5)])],
        &paths(&["src/a.ts"]),
        OwnershipMode::AggregateOnly,
    );
    assert_eq!(report.files[0].bus_factor_50, Some(1));
    assert_eq!(report.files[0].concentration_percent, Some(50.0));
}

#[test]
fn modules_sum_touches_instead_of_averaging_percentages() {
    let report = build(
        &[
            touches("src/a.ts", &[("a", 100)]),
            touches("src/b.ts", &[("b", 1), ("c", 1)]),
        ],
        &paths(&["src/a.ts", "src/b.ts"]),
        OwnershipMode::AggregateOnly,
    );
    let root = report
        .modules
        .iter()
        .find(|module| module.path == ".")
        .unwrap();
    assert_eq!(root.contributors, 3);
    let expected = 100.0 / 102.0 * 100.0;
    assert!(
        (root.concentration_percent.unwrap() - expected).abs() < 1e-9,
        "root concentration must sum touches, got {root:?}"
    );
    let src = report
        .modules
        .iter()
        .find(|module| module.path == "src")
        .unwrap();
    assert_eq!(
        (
            src.contributors,
            src.concentration_percent,
            src.bus_factor_50
        ),
        (
            root.contributors,
            root.concentration_percent,
            root.bus_factor_50
        ),
        "one shared directory mirrors the root totals"
    );
}

#[test]
fn author_modes_are_opt_in_and_anonymized_labels_do_not_correlate() {
    let fixture = [touches("src/a.ts", &[("b@x", 2), ("a@x", 3)])];
    let aggregate = build(
        &fixture,
        &paths(&["src/a.ts"]),
        OwnershipMode::AggregateOnly,
    );
    assert!(aggregate.files[0].authors.is_none());

    let named = build(
        &fixture,
        &paths(&["src/a.ts"]),
        OwnershipMode::IncludeAuthors,
    );
    let rows = named.files[0].authors.as_ref().unwrap();
    assert_eq!(rows[0].identity, "a@x");
    assert_eq!(rows[0].touches, 3);
    assert_eq!(rows[1].identity, "b@x");

    let anonymous = build(
        &fixture,
        &paths(&["src/a.ts"]),
        OwnershipMode::AnonymizeAuthors,
    );
    let again = build(
        &fixture,
        &paths(&["src/a.ts"]),
        OwnershipMode::AnonymizeAuthors,
    );
    assert_eq!(anonymous, again);
    let rows = anonymous.files[0].authors.as_ref().unwrap();
    assert_eq!(rows[0].identity, "author-001");
    assert_eq!(rows[0].touches, 3);
    assert_eq!(rows[1].identity, "author-002");
    assert_eq!(rows[1].touches, 2);
    assert!(!serde_json::to_string(&anonymous).unwrap().contains('@'));
}

#[test]
fn reports_are_deterministic_and_path_sorted() {
    let fixture = [
        touches("src/z.ts", &[("b", 1)]),
        touches("src/a.ts", &[("a", 1)]),
    ];
    let first = build(
        &fixture,
        &paths(&["src/z.ts", "src/a.ts"]),
        OwnershipMode::AggregateOnly,
    );
    let second = build(
        &fixture,
        &paths(&["src/a.ts", "src/z.ts"]),
        OwnershipMode::AggregateOnly,
    );
    assert_eq!(first, second, "input order never changes the report");
    assert_eq!(
        first
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["src/a.ts", "src/z.ts"]
    );
    let modules: Vec<&str> = first.modules.iter().map(|m| m.path.as_str()).collect();
    let mut sorted = modules.clone();
    sorted.sort_unstable();
    assert_eq!(modules, sorted);
}
