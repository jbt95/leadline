use leadline::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE,
    MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use leadline::external::InputBudget;
use leadline::test_relationships::ingest;
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

fn function(name: &str, start: u32) -> FunctionAnalysis {
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
                language: leadline::core::Language::TypeScript,
                functions: vec![function("charge", 10), function("charge", 30)],
                parse_errors: vec![],
            },
            FileAnalysis {
                path: "tests/payment.test.ts".to_owned(),
                language: leadline::core::Language::TypeScript,
                functions: vec![function("payment", 1)],
                parse_errors: vec![],
            },
            FileAnalysis {
                path: "tests/charge.test.ts".to_owned(),
                language: leadline::core::Language::TypeScript,
                functions: vec![function("chargeTest", 1)],
                parse_errors: vec![],
            },
        ],
    }
}

const MAP: &str = r#"{
  "schema_version": 1,
  "relationships": [
    {"test_path": "tests/payment.test.ts", "target_path": "src/payment.ts"},
    {"test_path": "tests/charge.test.ts", "target_path": "src/payment.ts", "target_function": "charge"},
    {"test_path": "tests/missing.test.ts", "target_path": "src/missing.ts"},
    {"test_path": "../escape.test.ts", "target_path": "src/payment.ts"}
  ]
}"#;

#[test]
fn resolves_relationships_and_marks_unresolved_rows() {
    let root = temporary_directory();
    let map = write(&root, "map.json", MAP);
    let report = ingest(&[map], &analysis(), &mut InputBudget::new()).unwrap();
    assert_eq!(report.relationships.len(), 4);
    assert_eq!(report.unresolved, 3);

    let plain = report
        .relationships
        .iter()
        .find(|row| row.test_path.as_deref() == Some("tests/payment.test.ts"))
        .unwrap();
    assert!(plain.resolved);
    assert_eq!(plain.target_path.as_deref(), Some("src/payment.ts"));
    assert_eq!(plain.target_function_id, None);

    let ambiguous = report
        .relationships
        .iter()
        .find(|row| row.test_path.as_deref() == Some("tests/charge.test.ts"))
        .unwrap();
    assert!(!ambiguous.resolved);
    assert_eq!(ambiguous.target_function_id, None);
    assert!(ambiguous.reason.as_deref().unwrap().contains("ambiguous"));

    let missing = report
        .relationships
        .iter()
        .find(|row| row.test_path.as_deref() == Some("tests/missing.test.ts"))
        .unwrap();
    assert!(!missing.resolved);
    assert_eq!(missing.target_path.as_deref(), Some("src/missing.ts"));

    let escape = report
        .relationships
        .iter()
        .find(|row| row.test_path.as_deref() == Some("../escape.test.ts"))
        .unwrap();
    assert!(!escape.resolved);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unique_function_names_and_duplicate_rows_collapse() {
    let root = temporary_directory();
    let map = write(
        &root,
        "map.json",
        r#"{"schema_version": 1, "relationships": [
            {"test_path": "tests/payment.test.ts", "target_path": "src/payment.ts", "target_function": "charge"},
            {"test_path": "tests/payment.test.ts", "target_path": "src/payment.ts", "target_function": "charge"}
        ]}"#,
    );
    let report = ingest(&[map], &analysis(), &mut InputBudget::new()).unwrap();
    assert_eq!(report.relationships.len(), 1, "exact duplicates collapse");
    assert!(
        !report.relationships[0].resolved,
        "ambiguous name stays unresolved"
    );

    let unique = write(
        &root,
        "unique.json",
        r#"{"schema_version": 1, "relationships": [
            {"test_path": "tests/payment.test.ts", "target_path": "src/payment.ts", "target_function": "charge"}
        ]}"#,
    );
    let mut single = analysis();
    single.files[0].functions.truncate(1);
    let report = ingest(&[unique], &single, &mut InputBudget::new()).unwrap();
    assert!(report.relationships[0].resolved);
    assert_eq!(
        report.relationships[0].target_function_id.as_deref(),
        Some("charge@10")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_unknown_schemas_and_missing_arrays() {
    let root = temporary_directory();
    let bad = write(
        &root,
        "bad.json",
        r#"{"schema_version": 2, "relationships": []}"#,
    );
    assert!(ingest(&[bad], &analysis(), &mut InputBudget::new()).is_err());
    let missing = write(&root, "missing.json", r#"{"schema_version": 1}"#);
    assert!(ingest(&[missing], &analysis(), &mut InputBudget::new()).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
