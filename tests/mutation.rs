use leadline::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE,
    MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use leadline::external::InputBudget;
use leadline::mutation::{MutationInput, MutationStatus, ingest};
use leadline::source_snapshot::SourceEntry;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-mutation-{}-{id}", std::process::id()));
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

fn function(name: &str, start: u32, end: u32) -> FunctionAnalysis {
    FunctionAnalysis {
        name: name.to_owned(),
        id: format!("{name}@{start}"),
        kind: FunctionKind::Function,
        start_line: start,
        end_line: end,
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
        files: vec![FileAnalysis {
            path: "src/a.ts".to_owned(),
            language: leadline::core::Language::TypeScript,
            functions: vec![function("outer", 1, 40), function("inner", 10, 20)],
            parse_errors: vec![],
        }],
    }
}

fn entries() -> Vec<SourceEntry> {
    vec![SourceEntry {
        path: "src/a.ts".to_owned(),
        bytes: Vec::new(),
    }]
}

fn budget() -> InputBudget {
    InputBudget::new()
}

const STRYER: &str = r#"{
  "schemaVersion": "1.0",
  "files": {
    "src/a.ts": {
      "mutants": [
        {"id": "0", "mutatorName": "ArithmeticOperator", "status": "Killed",
         "location": {"start": {"line": 12, "column": 4}, "end": {"line": 12, "column": 9}}},
        {"id": "1", "mutatorName": "ArithmeticOperator", "status": "Survived",
         "location": {"start": {"line": 12, "column": 4}, "end": {"line": 12, "column": 9}}},
        {"id": "2", "mutatorName": "BooleanLiteral", "status": "NoCoverage",
         "location": {"start": {"line": 30, "column": 0}, "end": {"line": 30, "column": 5}}},
        {"id": "3", "mutatorName": "StringLiteral", "status": "TimedOut",
         "location": {"start": {"line": 30, "column": 0}, "end": {"line": 30, "column": 5}}},
        {"id": "4", "mutatorName": "BlockStatement", "status": "Ignored",
         "location": {"start": {"line": 30, "column": 0}, "end": {"line": 30, "column": 5}}},
        {"id": "5", "mutatorName": "ConditionalExpression", "status": "CompileError",
         "location": {"start": {"line": 30, "column": 0}, "end": {"line": 30, "column": 5}}}
      ]
    },
    "src/missing.ts": {
      "mutants": [
        {"id": "6", "mutatorName": "ArithmeticOperator", "status": "Killed",
         "location": {"start": {"line": 1, "column": 0}, "end": {"line": 1, "column": 3}}}
      ]
    }
  }
}"#;

#[test]
fn stryker_normalizes_status_score_and_coordinates() {
    let root = temporary_directory();
    let report_path = write(&root, "stryker.json", STRYER);
    let report = ingest(
        &[MutationInput::Stryker(report_path)],
        &entries(),
        &analysis(),
        &mut budget(),
    )
    .unwrap();

    assert_eq!(report.summary.total, 7);
    assert_eq!(report.summary.killed, 2);
    assert_eq!(report.summary.timed_out, 1);
    assert_eq!(report.summary.survived, 1);
    assert_eq!(report.summary.no_coverage, 1);
    assert_eq!(report.summary.ignored, 1);
    assert_eq!(report.summary.compile_error, 1);
    assert_eq!(report.summary.scored_mutants, 5);
    assert_eq!(report.summary.score, Some(60.0));
    assert_eq!(report.summary.unresolved, 1);

    let inner = report
        .mutants
        .iter()
        .find(|row| row.native_id.as_deref() == Some("0"))
        .unwrap();
    assert_eq!(inner.start_line, Some(12));
    assert_eq!(inner.start_column, Some(5), "schema columns are 0-based");
    assert_eq!(inner.end_column, Some(10));
    assert_eq!(inner.function_id.as_deref(), Some("inner@10"));

    let unresolved = report
        .mutants
        .iter()
        .find(|row| row.native_id.as_deref() == Some("6"))
        .unwrap();
    assert_eq!(unresolved.path, None);
    assert!(unresolved.reason.is_some());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stryker_rejects_unsupported_schemas_and_reports_unknown_status() {
    let root = temporary_directory();
    let bad = write(
        &root,
        "bad.json",
        r#"{"schemaVersion": "2.0", "files": {}}"#,
    );
    assert!(
        ingest(
            &[MutationInput::Stryker(bad)],
            &entries(),
            &analysis(),
            &mut budget()
        )
        .is_err()
    );
    let missing = write(&root, "missing.json", r#"{"files": {}}"#);
    assert!(
        ingest(
            &[MutationInput::Stryker(missing)],
            &entries(),
            &analysis(),
            &mut budget()
        )
        .is_err()
    );
    assert_eq!(
        MutationStatus::parse("runtime_error"),
        MutationStatus::Error
    );
    assert_eq!(MutationStatus::parse("MEMORY-ERROR"), MutationStatus::Error);
    assert_eq!(
        MutationStatus::parse("something-new"),
        MutationStatus::Unknown
    );
    std::fs::remove_dir_all(root).unwrap();
}

const PIT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<mutations>
  <mutation detected="true" status="KILLED">
    <sourceFile>a.ts</sourceFile>
    <mutatedClass>com.example.A</mutatedClass>
    <mutatedMethod>outer</mutatedMethod>
    <methodDescription>()V</methodDescription>
    <lineNumber>15</lineNumber>
    <mutator>MathMutator</mutator>
    <indexes><index>3</index></indexes>
    <blocks><block>0</block></blocks>
    <description>replaced return</description>
  </mutation>
  <mutation detected="false" status="SURVIVED">
    <sourceFile>a.ts</sourceFile>
    <mutatedClass>com.example.A</mutatedClass>
    <mutatedMethod>outer</mutatedMethod>
    <methodDescription>()V</methodDescription>
    <lineNumber>15</lineNumber>
    <mutator>MathMutator</mutator>
    <indexes><index>4</index></indexes>
    <blocks><block>0</block></blocks>
  </mutation>
</mutations>"#;

#[test]
fn pit_resolves_unique_suffix_and_keeps_distinct_same_line_mutants() {
    let root = temporary_directory();
    let report_path = write(&root, "pit.xml", PIT);
    let report = ingest(
        &[MutationInput::Pit(report_path)],
        &entries(),
        &analysis(),
        &mut budget(),
    )
    .unwrap();
    assert_eq!(report.summary.total, 2);
    assert_eq!(report.summary.killed, 1);
    assert_eq!(report.summary.survived, 1);
    let first = &report.mutants[0];
    assert_eq!(first.path.as_deref(), Some("src/a.ts"));
    assert_eq!(first.start_line, Some(15));
    assert_eq!(first.end_line, Some(16), "PIT lines are half-open");
    assert_eq!(first.function_id.as_deref(), Some("inner@10"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn traversal_paths_stay_unresolved_and_duplicate_reports_union_provenance() {
    let root = temporary_directory();
    let report_path = write(
        &root,
        "pit.xml",
        &PIT.replace(
            "<sourceFile>a.ts</sourceFile>",
            "<sourceFile>../../etc/a.ts</sourceFile>",
        ),
    );
    let report = ingest(
        &[MutationInput::Pit(report_path.clone())],
        &entries(),
        &analysis(),
        &mut budget(),
    )
    .unwrap();
    assert!(report.mutants.iter().all(|row| row.path.is_none()));

    let merged = ingest(
        &[
            MutationInput::Pit(report_path.clone()),
            MutationInput::Pit(report_path),
        ],
        &entries(),
        &analysis(),
        &mut budget(),
    )
    .unwrap();
    assert_eq!(merged.summary.total, 2, "identical reports collapse");
    assert!(merged.mutants.iter().all(|row| row.report_ids.len() == 1));
    std::fs::remove_dir_all(root).unwrap();
}
