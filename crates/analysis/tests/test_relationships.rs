use analysis::test_relationships::{TEST_RELATIONSHIP_SCHEMA_VERSION, ingest};
use model::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE,
    MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use parse::external::InputBudget;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-testmap-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn write(root: &Path, name: &str, contents: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

fn metrics() -> FunctionMetrics {
    FunctionMetrics {
        loc: 10,
        logical_loc: 5,
        function_length: 10,
        parameters: 0,
        max_nesting: 1,
        cyclomatic: 1,
        cognitive: 1,
        halstead_n1: 0,
        halstead_n2: 0,
        halstead_total_operators: 0,
        halstead_total_operands: 0,
        halstead_vocabulary: 0,
        halstead_length: 0,
        halstead_volume: 0.0,
        halstead_difficulty: 0.0,
        halstead_effort: 0.0,
        maintainability_index: 50.0,
        coverage: None,
        crap: None,
    }
}

fn function(name: &str, symbol_key: &str, start: u32) -> FunctionAnalysis {
    FunctionAnalysis {
        name: name.to_owned(),
        id: format!("{name}@{start}"),
        kind: FunctionKind::Function,
        start_line: start,
        end_line: start + 10,
        start_byte: 0,
        end_byte: 100,
        metrics: metrics(),
        contributions: vec![],
        suppressed_metrics: vec![],
        symbol_key: symbol_key.to_owned(),
        source_fingerprint: 0,
    }
}

fn analysis() -> AnalysisReport {
    AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: "test",
        metric_specs: MetricSpecs::default(),
        files: vec![
            FileAnalysis {
                path: "src/payment.ts".to_owned(),
                language: model::core::Language::TypeScript,
                functions: vec![
                    function("charge", "src/payment.ts:charge(int)", 10),
                    function("charge", "src/payment.ts:charge(string)", 30),
                ],
                parse_errors: vec![],
            },
            FileAnalysis {
                path: "tests/payment.test.ts".to_owned(),
                language: model::core::Language::TypeScript,
                functions: vec![function("payment", "tests/payment.test.ts:payment()", 1)],
                parse_errors: vec![],
            },
        ],
    }
}

const MAP: &str = r#"{
  "schema_version": 2,
  "recorder": "pytest",
  "recorded_at": "a1b2c3d",
  "relationships": [
    {"test_id": "tests/payment.test.ts::charges once", "test_path": "tests/payment.test.ts", "symbol_key": "src/payment.ts:charge(int)", "target_path": "src/payment.ts"},
    {"test_id": "tests/payment.test.ts::refunds", "test_path": "tests/payment.test.ts", "symbol_key": "src/payment.ts:charge(string)", "target_path": "src/payment.ts"},
    {"test_id": "tests/missing.test.ts::gone", "test_path": "tests/missing.test.ts", "symbol_key": "src/missing.ts:gone()", "target_path": "src/missing.ts"},
    {"test_id": "../escape.test.ts::escape", "test_path": "../escape.test.ts", "symbol_key": "src/payment.ts:charge(int)", "target_path": "src/payment.ts"}
  ]
}"#;

#[test]
fn resolves_symbol_keys_and_marks_unresolved_rows() {
    let root = temporary_directory();
    let map = write(&root, "map.json", MAP);
    let report = ingest(&[map], &analysis(), &mut InputBudget::new()).unwrap();
    assert_eq!(report.schema_version, TEST_RELATIONSHIP_SCHEMA_VERSION);
    assert_eq!(report.recorder, "pytest");
    assert_eq!(report.recorded_at, "a1b2c3d");
    assert_eq!(report.relationships.len(), 4);
    assert_eq!(report.unresolved, 2);

    let keyed = report
        .relationships
        .iter()
        .find(|row| row.test_id == "tests/payment.test.ts::charges once")
        .unwrap();
    assert!(keyed.resolved);
    assert_eq!(keyed.reason, None);
    assert_eq!(keyed.symbol_key, "src/payment.ts:charge(int)");
    assert_eq!(keyed.target_path.as_deref(), Some("src/payment.ts"));

    let other = report
        .relationships
        .iter()
        .find(|row| row.test_id == "tests/payment.test.ts::refunds")
        .unwrap();
    assert!(other.resolved, "each key resolves to its own overload");

    let missing = report
        .relationships
        .iter()
        .find(|row| row.test_id == "tests/missing.test.ts::gone")
        .unwrap();
    assert!(!missing.resolved);
    assert_eq!(missing.reason.as_deref(), Some("unknown test file"));
    assert_eq!(missing.test_path.as_deref(), Some("tests/missing.test.ts"));
    assert_eq!(missing.target_path.as_deref(), Some("src/missing.ts"));

    let escape = report
        .relationships
        .iter()
        .find(|row| row.test_id == "../escape.test.ts::escape")
        .unwrap();
    assert!(!escape.resolved, "a path escaping the root never resolves");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unmatched_symbol_keys_stay_with_a_stable_reason() {
    let root = temporary_directory();
    let map = write(
        &root,
        "map.json",
        r#"{"schema_version": 2, "recorder": "pytest", "recorded_at": "a1b2c3d",
            "relationships": [
              {"test_id": "tests/payment.test.ts::vanished", "test_path": "tests/payment.test.ts", "symbol_key": "src/payment.ts:vanished(int)", "target_path": "src/payment.ts"},
              {"test_id": "tests/payment.test.ts::moved", "test_path": "tests/payment.test.ts", "symbol_key": "src/payment.ts:charge(int)", "target_path": "src/elsewhere.ts"}
            ]}"#,
    );
    let report = ingest(
        std::slice::from_ref(&map),
        &analysis(),
        &mut InputBudget::new(),
    )
    .unwrap();
    assert_eq!(report.relationships.len(), 2, "no row is dropped");

    let vanished = report
        .relationships
        .iter()
        .find(|row| row.test_id == "tests/payment.test.ts::vanished")
        .unwrap();
    assert!(!vanished.resolved);
    assert_eq!(vanished.reason.as_deref(), Some("unknown symbol key"));

    let moved = report
        .relationships
        .iter()
        .find(|row| row.test_id == "tests/payment.test.ts::moved")
        .unwrap();
    assert!(
        moved.resolved,
        "the symbol key is authoritative, so a stale target_path does not unresolve the row"
    );

    let mut collided = analysis();
    collided.files[0].functions[1].symbol_key = "src/payment.ts:charge(int)".to_owned();
    let report = ingest(
        std::slice::from_ref(&map),
        &collided,
        &mut InputBudget::new(),
    )
    .unwrap();
    let ambiguous = report
        .relationships
        .iter()
        .find(|row| row.test_id == "tests/payment.test.ts::moved")
        .unwrap();
    assert!(!ambiguous.resolved);
    assert_eq!(ambiguous.reason.as_deref(), Some("ambiguous symbol key"));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_version_one_maps_naming_the_version() {
    let root = temporary_directory();
    let v1 = write(
        &root,
        "v1.json",
        r#"{"schema_version": 1, "relationships": [
            {"test_path": "tests/payment.test.ts", "target_path": "src/payment.ts", "target_function": "charge"}
        ]}"#,
    );
    let error = ingest(&[v1], &analysis(), &mut InputBudget::new())
        .expect_err("a v1 map must not be read as v2");
    let message = error.to_string();
    assert!(
        message.contains("unsupported schema_version 1"),
        "the error names the version the file carried: {message}"
    );
    assert!(message.contains("v1.json"), "{message}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn duplicate_rows_collapse_but_distinct_test_ids_do_not() {
    let root = temporary_directory();
    let map = write(
        &root,
        "map.json",
        r#"{"schema_version": 2, "recorder": "pytest", "recorded_at": "a1b2c3d",
            "relationships": [
              {"test_id": "tests/payment.test.ts::charges", "test_path": "tests/payment.test.ts", "symbol_key": "src/payment.ts:charge(int)", "target_path": "src/payment.ts"},
              {"test_id": "tests/payment.test.ts::charges", "test_path": "tests/payment.test.ts", "symbol_key": "src/payment.ts:charge(int)", "target_path": "src/payment.ts"},
              {"test_id": "tests/payment.test.ts::refunds", "test_path": "tests/payment.test.ts", "symbol_key": "src/payment.ts:charge(int)", "target_path": "src/payment.ts"}
            ]}"#,
    );
    let report = ingest(&[map], &analysis(), &mut InputBudget::new()).unwrap();
    assert_eq!(report.relationships.len(), 2, "one row per test id");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_unknown_schemas_malformed_rows_and_missing_arrays() {
    let root = temporary_directory();
    let newer = write(
        &root,
        "newer.json",
        r#"{"schema_version": 3, "recorder": "pytest", "recorded_at": "a1b2c3d", "relationships": []}"#,
    );
    assert!(ingest(&[newer], &analysis(), &mut InputBudget::new()).is_err());
    let missing = write(
        &root,
        "missing.json",
        r#"{"schema_version": 2, "recorder": "pytest", "recorded_at": "a1b2c3d"}"#,
    );
    assert!(ingest(&[missing], &analysis(), &mut InputBudget::new()).is_err());
    let no_id = write(
        &root,
        "no-id.json",
        r#"{"schema_version": 2, "recorder": "pytest", "recorded_at": "a1b2c3d", "relationships": [
            {"test_path": "tests/payment.test.ts", "symbol_key": "src/payment.ts:charge(int)"}
        ]}"#,
    );
    let error = ingest(&[no_id], &analysis(), &mut InputBudget::new()).unwrap_err();
    assert!(error.to_string().contains("missing test_id"), "{error}");
    let no_key = write(
        &root,
        "no-key.json",
        r#"{"schema_version": 2, "recorder": "pytest", "recorded_at": "a1b2c3d", "relationships": [
            {"test_id": "tests/payment.test.ts::charges", "test_path": "tests/payment.test.ts"}
        ]}"#,
    );
    let error = ingest(&[no_key], &analysis(), &mut InputBudget::new()).unwrap_err();
    assert!(error.to_string().contains("missing symbol_key"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_maps_recorded_at_different_commits() {
    let root = temporary_directory();
    let first = write(&root, "first.json", MAP);
    let second = write(
        &root,
        "second.json",
        r#"{"schema_version": 2, "recorder": "pytest", "recorded_at": "d4e5f6a", "relationships": []}"#,
    );
    let error = ingest(&[first, second], &analysis(), &mut InputBudget::new()).unwrap_err();
    assert!(error.to_string().contains("d4e5f6a"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}
