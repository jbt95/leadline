fn function_named<'a>(
    file: &'a leadline::core::FileAnalysis,
    name: &str,
) -> &'a leadline::core::FunctionAnalysis {
    file.functions
        .iter()
        .find(|function| function.name == name)
        .unwrap_or_else(|| panic!("missing function {name}"))
}

#[test]
fn mutual_recursion_is_not_counted() {
    let source = "class M {\n    void a() { b(); }\n    void b() { a(); }\n}\n";
    let file = leadline::analyze_source("M.java", source.as_bytes()).unwrap();
    for name in ["a", "b"] {
        let function = function_named(&file, name);
        assert_eq!(function.metrics.cognitive, 0, "{name}");
        assert!(function.contributions.iter().all(|c| c.rule != "recursion"));
    }
}

#[test]
fn super_calls_are_not_recursion() {
    let source = "class Sub extends Base {\n    void foo() {\n        super.foo();\n    }\n}\n";
    let file = leadline::analyze_source("Sub.java", source.as_bytes()).unwrap();
    let function = function_named(&file, "foo");
    assert_eq!(function.metrics.cognitive, 0);
}

#[test]
fn this_qualified_self_call_counts() {
    let source = "class C {\n    f() {\n        return this.f();\n    }\n}\n";
    let file = leadline::analyze_source("c.js", source.as_bytes()).unwrap();
    let function = function_named(&file, "f");
    assert_eq!(
        (function.metrics.cyclomatic, function.metrics.cognitive),
        (1, 1)
    );
}

#[test]
fn nested_function_call_is_not_attributed_to_outer() {
    let source = "function f(x) {\n  function g(y) {\n    return f(y);\n  }\n  return g(x);\n}\n";
    let file = leadline::analyze_source("nested-rec.js", source.as_bytes()).unwrap();
    for name in ["f", "g"] {
        let function = function_named(&file, name);
        assert_eq!(function.metrics.cognitive, 0, "{name}");
    }
}
