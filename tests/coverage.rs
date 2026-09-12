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
