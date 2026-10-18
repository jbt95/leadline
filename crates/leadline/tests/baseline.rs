use leadline::config::RegressionLimits;
use leadline::core::{AnalysisReport, METRIC_PROFILE, MetricSpecs, OUTPUT_SCHEMA_VERSION};

const SIMPLE: &[u8] = b"function calc(x: boolean) { return 1; }\n";
const COMPLEX: &[u8] =
    b"function calc(x: boolean) { if (x) { if (!x) { return 2; } return 1; } return 0; }\n";
const EXTRA: &[u8] = b"function calc(x: boolean) { return 1; }\nfunction fresh() { return 2; }\n";

fn report(path: &str, source: &[u8]) -> AnalysisReport {
    let file = leadline::analyze_source(path, source).unwrap();
    AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: "test",
        metric_specs: MetricSpecs::default(),
        files: vec![file],
    }
}

fn zero_limits() -> RegressionLimits {
    RegressionLimits::default()
}

#[test]
fn round_trip_is_deterministic() {
    let baseline = leadline::baseline::Baseline::from_report(&report("calc.ts", COMPLEX));
    let dir = std::env::temp_dir().join(format!("leadline-baseline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("first.json");
    let second = dir.join("second.json");
    baseline.write(&first).unwrap();
    baseline.write(&second).unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
    let reloaded = leadline::baseline::Baseline::read(&first).unwrap();
    assert_eq!(baseline, reloaded);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn complexity_increase_matches_baseline_entry_as_regression() {
    let baseline = leadline::baseline::Baseline::from_report(&report("calc.ts", SIMPLE));
    let after = report("calc.ts", COMPLEX);
    let regressions = baseline.compare(&after, &zero_limits());
    assert_eq!(regressions.len(), 1);
    assert_eq!(regressions[0].path, "calc.ts");
    assert_eq!(regressions[0].name, "calc");
    assert!(regressions[0].after.metrics.cognitive > regressions[0].before.cognitive);
}

#[test]
fn allowed_deltas_and_improvements_are_not_regressions() {
    let baseline = leadline::baseline::Baseline::from_report(&report("calc.ts", SIMPLE));
    let after = report("calc.ts", COMPLEX);
    let generous = RegressionLimits {
        cognitive: 100,
        cyclomatic: 100,
        crap: 100.0,
        max_nesting: 100,
    };
    assert!(baseline.compare(&after, &generous).is_empty());
    let improved = baseline.compare(&report("calc.ts", SIMPLE), &zero_limits());
    assert!(improved.is_empty());
}

#[test]
fn new_functions_are_not_delta_regressions_and_deleted_are_ignored() {
    let baseline = leadline::baseline::Baseline::from_report(&report("calc.ts", SIMPLE));
    let after = report("calc.ts", EXTRA);
    assert!(baseline.compare(&after, &zero_limits()).is_empty());

    let full = leadline::baseline::Baseline::from_report(&report("calc.ts", EXTRA));
    let pruned = report("calc.ts", SIMPLE);
    assert!(full.compare(&pruned, &zero_limits()).is_empty());
}

#[test]
fn unknown_schema_is_rejected() {
    let dir = std::env::temp_dir().join(format!("leadline-bad-schema-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("baseline.json");
    std::fs::write(
        &path,
        r#"{"schema_version":99,"metric_profile":"default","functions":[]}"#,
    )
    .unwrap();
    let error = leadline::baseline::Baseline::read(&path).unwrap_err();
    assert!(error.to_string().contains("schema_version"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unknown_profile_is_rejected() {
    let dir = std::env::temp_dir().join(format!("leadline-bad-profile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("baseline.json");
    std::fs::write(
        &path,
        r#"{"schema_version":1,"metric_profile":"other-v9","functions":[]}"#,
    )
    .unwrap();
    let error = leadline::baseline::Baseline::read(&path).unwrap_err();
    assert!(error.to_string().contains("metric_profile"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn duplicate_identities_are_rejected() {
    let mut baseline = leadline::baseline::Baseline::from_report(&report("calc.ts", SIMPLE));
    assert!(!baseline.functions.is_empty());
    baseline.functions.push(baseline.functions[0].clone());
    let dir = std::env::temp_dir().join(format!("leadline-dupe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("baseline.json");
    std::fs::write(&path, serde_json::to_string(&baseline).unwrap()).unwrap();
    let error = leadline::baseline::Baseline::read(&path).unwrap_err();
    assert!(error.to_string().contains("duplicate"));
    std::fs::remove_dir_all(&dir).unwrap();
}
