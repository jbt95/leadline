// tests/sonar_probe.rs — SCRATCH, deleted before merge.
#[test]
fn probe_sonar_shapes() {
    let java_arrow_switch = r#"class A {
    int f(int x) {
        return switch (x) {
            case 1 -> 10;
            default -> 0;
        };
    }
}
"#;
    let java_throw = "class B {\n    void g(boolean b) {\n        if (b) {\n            throw new RuntimeException();\n        }\n    }\n}\n";
    let ts_throw = "function g(b: boolean) {\n  if (b) {\n    throw new Error();\n  }\n}\n";
    for (path, src) in [
        ("Probe.java", java_arrow_switch),
        ("Throw.java", java_throw),
        ("throw.ts", ts_throw),
    ] {
        let file = leadline::analyze_source(path, src.as_bytes()).unwrap();
        for function in &file.functions {
            println!(
                "{}:{} cyclomatic={} cognitive={} contributions={:?} parse_errors={:?}",
                path,
                function.name,
                function.metrics.cyclomatic,
                function.metrics.cognitive,
                function.contributions,
                file.parse_errors,
            );
        }
    }
    panic!("probe output only");
}
