use leadline::core::{AnalysisReport, FunctionKind, Language, MetricSpecs};
use std::path::Path;

fn function_metrics(path: &str, source: &str) -> (u32, u32, u32) {
    let file = leadline::analyze_source(path, source.as_bytes()).unwrap();
    assert!(file.parse_errors.is_empty(), "parse errors in {path}");
    let metrics = &file.functions.first().expect("missing function").metrics;
    (metrics.cyclomatic, metrics.cognitive, metrics.max_nesting)
}

#[test]
fn public_language_detection_contract_covers_every_extension() {
    use leadline::parser::detect_language;

    for (path, expected) in [
        ("Example.JAVA", Some(Language::Java)),
        ("example.js", Some(Language::JavaScript)),
        ("example.jsx", Some(Language::JavaScript)),
        ("example.mjs", Some(Language::JavaScript)),
        ("example.cjs", Some(Language::JavaScript)),
        ("example.h", Some(Language::Cpp)),
        ("example.cpp", Some(Language::Cpp)),
        ("example.hpp", Some(Language::Cpp)),
        ("example.ts", Some(Language::TypeScript)),
        ("example.mts", Some(Language::TypeScript)),
        ("example.cts", Some(Language::TypeScript)),
        ("example.test.tsx", Some(Language::Tsx)),
        ("example.c", Some(Language::C)),
        ("example.rs", Some(Language::Rust)),
        ("example.zig", Some(Language::Zig)),
        ("Example.ZIG", Some(Language::Zig)),
        ("example.py", Some(Language::Python)),
        ("example.PY", Some(Language::Python)),
        // Type stubs declare signatures with no bodies, so they are not
        // analyzed and not discovered.
        ("example.pyi", None),
        ("example.json", None),
        ("Makefile", None),
    ] {
        assert_eq!(detect_language(path), expected, "{path}");
    }
}

#[test]
fn path_normalization_is_lexical_and_stable() {
    assert_eq!(
        leadline::normalize_path(Path::new("./src/generated/../main.ts")),
        "src/main.ts"
    );
    assert_eq!(
        leadline::normalized_relative_path(
            Path::new("workspace/src/main.ts"),
            Path::new("workspace")
        ),
        "src/main.ts"
    );
}

#[test]
fn documented_metric_edges_are_contracts() {
    for (name, source, expected) in [
        (
            "repeated logical operators",
            "function f(a, b, c) { return (a && b) && c; }",
            (3, 1, 0),
        ),
        (
            "mixed logical operators",
            "function f(a, b, c, d) { return a && b || c && d; }",
            (4, 3, 0),
        ),
        (
            "labeled continue",
            "function f(a) { outer: while (a) { continue outer; } }",
            (2, 2, 1),
        ),
        (
            "default and finally",
            "function f(value) { try { switch (value) { case 1: return 1; default: return 0; } } finally { value = 0; } }",
            (2, 1, 1),
        ),
    ] {
        assert_eq!(function_metrics("fixture.js", source), expected, "{name}");
    }
}

#[test]
fn python_metric_edges_are_contracts() {
    for (name, source, expected) in [
        // `raise` follows JavaScript's `throw`: one cyclomatic point, no
        // cognitive point.
        (
            "raise in a guard",
            "def f(value):\n    if value < 0:\n        raise ValueError(value)\n    return value\n",
            (3, 1, 1),
        ),
        // `assert` is a debug-time guard the optimizer strips, so it scores
        // nothing, matching the Rust `panic!` row.
        (
            "assert is free",
            "def f(value):\n    assert value\n    return value\n",
            (1, 0, 0),
        ),
        // The `else` on `for` is another path a reader must follow.
        (
            "for else adds a path",
            "def f(items):\n    for item in items:\n        return item\n    else:\n        return None\n",
            (2, 2, 1),
        ),
        (
            "while else adds a path",
            "def f(items):\n    while items:\n        return items\n    else:\n        return None\n",
            (2, 2, 1),
        ),
        // `except` is a catch and `finally` is not a decision; the `else` on
        // `try` adds one cognitive point and no cyclomatic point.
        (
            "except is a catch",
            "def f():\n    try:\n        return 1\n    except ValueError:\n        return 2\n",
            (2, 1, 1),
        ),
        (
            "finally is not a decision",
            "def f():\n    try:\n        return 1\n    finally:\n        return 2\n",
            (1, 0, 0),
        ),
        (
            "try else adds a path",
            "def f(value):\n    try:\n        return int(value)\n    except ValueError:\n        return 0\n    else:\n        return 1\n",
            (2, 2, 1),
        ),
        // `with` and `not` are not decisions.
        (
            "with is not a decision",
            "def f(path):\n    with open(path):\n        return 1\n",
            (1, 0, 0),
        ),
        (
            "not is not a decision",
            "def f(value):\n    return not value\n",
            (1, 0, 0),
        ),
        // A comprehension scores like the explicit loop it replaces.
        (
            "comprehension clauses",
            "def f(items):\n    return [x for x in items if x]\n",
            (3, 2, 1),
        ),
    ] {
        assert_eq!(function_metrics("fixture.py", source), expected, "{name}");
    }
}

#[test]
fn generators_and_java_constructors_are_discovered() {
    let generator = leadline::analyze_source(
        "generator.js",
        b"function* values(limit) { for (let i = 0; i < limit; i++) { yield i; } }",
    )
    .unwrap();
    assert_eq!(generator.functions.len(), 1);
    assert_eq!(generator.functions[0].name, "values");
    assert_eq!(generator.functions[0].metrics.parameters, 1);

    let constructor = leadline::analyze_source(
        "Point.java",
        b"class Point { Point(int x, int y) { if (x < y) { this.x = x; } } int x; }",
    )
    .unwrap();
    assert_eq!(constructor.functions.len(), 1);
    assert_eq!(constructor.functions[0].name, "Point");
    assert_eq!(constructor.functions[0].kind, FunctionKind::Constructor);
    assert_eq!(constructor.functions[0].metrics.parameters, 2);
}

#[test]
fn terminal_report_keeps_human_readable_metric_labels() {
    let file =
        leadline::analyze_source("fixture.ts", b"function score(x) { if (x) return 1; }").unwrap();
    let report = AnalysisReport {
        schema_version: leadline::core::OUTPUT_SCHEMA_VERSION,
        metric_profile: leadline::core::METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        metric_specs: MetricSpecs::default(),
        files: vec![file],
    };

    let output = leadline::report::terminal(&report);
    assert!(output.starts_with("fixture.ts\n\nscore()\n"));
    for label in [
        "Cognitive",
        "Cyclomatic",
        "Halstead V",
        "LOC",
        "Logical LOC",
        "Parameters",
        "Max nesting",
        "Coverage",
        "CRAP",
    ] {
        assert!(output.contains(&format!("  {label}:")), "missing {label}");
    }
    assert!(output.contains("Coverage:      unavailable\n"));
    assert!(output.contains("CRAP:          unavailable\n"));
}
