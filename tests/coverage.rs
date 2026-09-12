use leadline::coverage::CoverageMap;

#[test]
fn lcov_maps_known_lines_and_computes_crap() {
    let mut file = leadline::analyze_source(
        "src/payment.ts",
        b"function pay(ok: boolean) {\n  if (ok) {\n    return 1;\n  }\n  return 0;\n}\n",
    )
    .unwrap();
    let coverage = CoverageMap::from_lcov(
        "TN:\nSF:src/payment.ts\nDA:1,1\nDA:2,1\nDA:3,0\nDA:5,0\nend_of_record\n",
    )
    .unwrap();
    coverage.apply(&mut file);
    let metrics = &file.functions[0].metrics;
    assert_eq!(metrics.coverage, Some(0.5));
    assert_eq!(metrics.crap, Some(2.5));
}

#[test]
fn jacoco_uses_package_and_source_file_path() {
    let mut file = leadline::analyze_source(
        "sample/Value&More.java",
        b"class Value {\n  int value() {\n    return 1;\n  }\n}\n",
    )
    .unwrap();
    let coverage = CoverageMap::from_jacoco_xml(
        r#"<report name="test"><package name="sample"><sourcefile name="Value&amp;More.java"><line nr="2" mi="0" ci="1"/><line nr="3" mi="1" ci="0"/></sourcefile></package></report>"#,
    )
    .unwrap();
    coverage.apply(&mut file);
    assert_eq!(file.functions[0].metrics.coverage, Some(0.5));
}

#[test]
fn missing_coverage_stays_unavailable() {
    let mut file = leadline::analyze_source("other.ts", b"function empty() {}\n").unwrap();
    let coverage = CoverageMap::from_lcov("SF:covered.ts\nDA:1,1\nend_of_record\n").unwrap();
    coverage.apply(&mut file);
    assert_eq!(file.functions[0].metrics.coverage, None);
    assert_eq!(file.functions[0].metrics.crap, None);
}

#[test]
fn crap_formula_handles_coverage_bounds() {
    let source = b"function branch(x) { if (x) return 1; return 0; }\n";
    for (count, expected) in [(0, 6.0), (1, 2.0)] {
        let mut file = leadline::analyze_source("branch.js", source).unwrap();
        let coverage =
            CoverageMap::from_lcov(&format!("SF:branch.js\nDA:1,{count}\nend_of_record\n"))
                .unwrap();
        coverage.apply(&mut file);
        assert_eq!(file.functions[0].metrics.crap, Some(expected));
    }
}

#[test]
fn windows_coverage_paths_match_repository_paths() {
    let mut file =
        leadline::analyze_source("src/branch.ts", b"function branch() { return 1; }\n").unwrap();
    let coverage =
        CoverageMap::from_lcov("SF:C:\\repo\\src\\branch.ts\nDA:1,1\nend_of_record\n").unwrap();
    coverage.apply(&mut file);
    assert_eq!(file.functions[0].metrics.coverage, Some(1.0));
}

#[test]
fn ambiguous_suffixes_do_not_attach_wrong_coverage() {
    let mut file =
        leadline::analyze_source("src/branch.ts", b"function branch() { return 1; }\n").unwrap();
    let coverage = CoverageMap::from_lcov(
        "SF:/first/src/branch.ts\nDA:1,1\nend_of_record\nSF:/second/src/branch.ts\nDA:1,0\nend_of_record\n",
    )
    .unwrap();
    coverage.apply(&mut file);
    assert_eq!(file.functions[0].metrics.coverage, None);
}

#[test]
fn repeated_lcov_lines_accumulate_execution_counts() {
    let mut file =
        leadline::analyze_source("branch.ts", b"function branch() { return 1; }\n").unwrap();
    let coverage = CoverageMap::from_lcov("SF:branch.ts\nDA:1,1\nDA:1,0\nend_of_record\n").unwrap();
    coverage.apply(&mut file);
    assert_eq!(file.functions[0].metrics.coverage, Some(1.0));
}
use leadline::core::{AnalysisReport, METRIC_PROFILE, MetricSpecs, OUTPUT_SCHEMA_VERSION};

fn test_report(files: Vec<leadline::core::FileAnalysis>) -> AnalysisReport {
    AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: "test",
        metric_specs: MetricSpecs::default(),
        files,
    }
}

fn lcov_lines(path: &str, hits: &[(u32, u64)]) -> CoverageMap {
    let mut text = format!("TN:\nSF:{path}\n");
    for (line, count) in hits {
        text.push_str(&format!("DA:{line},{count}\n"));
    }
    text.push_str("end_of_record\n");
    CoverageMap::from_lcov(&text).unwrap()
}

fn analyzed(path: &str, source: &[u8], coverage: &CoverageMap) -> leadline::core::FileAnalysis {
    let mut file = leadline::analyze_source(path, source).unwrap();
    coverage.apply(&mut file);
    file
}

#[test]
fn line_hits_distinguishes_covered_uncovered_and_unknown() {
    let coverage = lcov_lines("a.ts", &[(2, 0), (3, 5)]);
    assert_eq!(coverage.hits("a.ts", 2), Some(0));
    assert_eq!(coverage.hits("a.ts", 3), Some(5));
    assert_eq!(coverage.hits("a.ts", 9), None);
    assert_eq!(coverage.hits("missing.ts", 2), None);
}

#[test]
fn zero_hit_contribution_lines_become_test_targets() {
    let source = b"function pay(ok: boolean) {\n  if (ok) {\n    return 1;\n  }\n  return 0;\n}\n";
    let mut probe = leadline::analyze_source("src/payment.ts", source).unwrap();
    let lines: Vec<(u32, u64)> = probe.functions[0]
        .contributions
        .iter()
        .map(|contribution| (contribution.line, 0))
        .collect();
    assert!(!lines.is_empty());
    let coverage = lcov_lines("src/payment.ts", &lines);
    probe = analyzed("src/payment.ts", source, &coverage);
    let report = test_report(vec![probe]);
    let targets = leadline::test_targets::test_targets(&report, &coverage);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].path, "src/payment.ts");
    assert_eq!(targets[0].function, "pay");
    assert_eq!(targets[0].uncovered.len(), lines.len());
    assert!(targets[0].unknown.is_empty());
}

#[test]
fn covered_contribution_lines_produce_no_targets() {
    let source = b"function pay(ok: boolean) {\n  if (ok) {\n    return 1;\n  }\n  return 0;\n}\n";
    let probe = leadline::analyze_source("src/payment.ts", source).unwrap();
    let lines: Vec<(u32, u64)> = probe.functions[0]
        .contributions
        .iter()
        .map(|contribution| (contribution.line, 1))
        .collect();
    let coverage = lcov_lines("src/payment.ts", &lines);
    let file = analyzed("src/payment.ts", source, &coverage);
    let report = test_report(vec![file]);
    assert!(leadline::test_targets::test_targets(&report, &coverage).is_empty());
}

#[test]
fn unknown_lines_are_separate_from_uncovered() {
    let source = b"function pay(ok: boolean) {\n  if (ok) {\n    return 1;\n  }\n  return 0;\n}\n";
    let probe = leadline::analyze_source("src/payment.ts", source).unwrap();
    let contributions = &probe.functions[0].contributions;
    assert!(!contributions.is_empty());
    let coverage = lcov_lines("src/payment.ts", &[(contributions[0].line, 0)]);
    let file = analyzed("src/payment.ts", source, &coverage);
    let report = test_report(vec![file]);
    let targets = leadline::test_targets::test_targets(&report, &coverage);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].uncovered.len(), 1);
    assert_eq!(
        targets[0].unknown.len(),
        contributions.len().saturating_sub(1)
    );
    for known in &targets[0].uncovered {
        for unknown in &targets[0].unknown {
            assert_ne!(known.line, unknown.line);
        }
    }
}

#[test]
fn functions_without_any_known_coverage_produce_no_targets() {
    let source = b"function pay(ok: boolean) {\n  if (ok) {\n    return 1;\n  }\n  return 0;\n}\n";
    let coverage = lcov_lines("other.ts", &[(2, 0)]);
    let file = analyzed("src/payment.ts", source, &coverage);
    let report = test_report(vec![file]);
    assert!(leadline::test_targets::test_targets(&report, &coverage).is_empty());
}

#[test]
fn targets_sort_by_crap_then_path_function_line() {
    let complex =
        b"function complex(x: boolean) {\n  if (x) {\n    return 1;\n  }\n  return 0;\n}\n";
    let probe = leadline::analyze_source("b.ts", complex).unwrap();
    let complex_lines: Vec<(u32, u64)> = probe.functions[0]
        .contributions
        .iter()
        .map(|contribution| (contribution.line, 0))
        .collect();
    let mut text = String::from("TN:\n");
    text.push_str("SF:b.ts\n");
    for (line, count) in &complex_lines {
        text.push_str(&format!("DA:{line},{count}\n"));
    }
    text.push_str(
        "end_of_record\nSF:a.ts\nDA:1,0\nDA:2,0\nDA:3,0\nDA:4,0\nDA:5,0\nDA:6,0\nend_of_record\n",
    );
    let coverage = CoverageMap::from_lcov(&text).unwrap();
    let simple = b"function simple(x: boolean) {\n  if (x) {\n    return 1;\n  }\n  return 0;\n}\n";
    let file_b = analyzed("b.ts", complex, &coverage);
    let file_a = analyzed("a.ts", simple, &coverage);
    let report = test_report(vec![file_b, file_a]);
    let targets = leadline::test_targets::test_targets(&report, &coverage);
    assert_eq!(targets.len(), 2);
    let crap: Vec<Option<f64>> = targets.iter().map(|target| target.crap).collect();
    assert!(crap[0] >= crap[1]);
    if crap[0] == crap[1] {
        assert!(targets[0].path <= targets[1].path);
    }
}
