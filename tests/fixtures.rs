use leadline::core::{FileAnalysis, FunctionAnalysis, FunctionKind};
use std::path::Path;

fn fixture(relative: &str) -> FileAnalysis {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative);
    leadline::analyze_source(relative, &std::fs::read(path).unwrap()).unwrap()
}

fn function<'a>(file: &'a FileAnalysis, name: &str) -> &'a FunctionAnalysis {
    file.functions
        .iter()
        .find(|function| function.name == name)
        .unwrap_or_else(|| panic!("missing function {name} in {}", file.path))
}

fn assert_complexity(
    relative: &str,
    name: &str,
    cyclomatic: u32,
    cognitive: u32,
    max_nesting: u32,
    parameters: u32,
) {
    let file = fixture(relative);
    let function = function(&file, name);
    assert_eq!(
        (
            function.metrics.cyclomatic,
            function.metrics.cognitive,
            function.metrics.max_nesting,
            function.metrics.parameters,
        ),
        (cyclomatic, cognitive, max_nesting, parameters),
        "{relative}:{name}"
    );
}

#[test]
fn tiny_fixtures_define_default_v1() {
    assert_complexity("java/empty.java", "empty", 1, 0, 0, 0);
    assert_complexity("java/decisions.java", "choose", 4, 5, 2, 1);
    assert_complexity("java/equivalent.java", "positive", 2, 1, 1, 1);
    assert_complexity("java/switch_catch.java", "parse", 3, 2, 1, 1);
    assert_complexity("java/lambda.java", "wrap", 2, 0, 0, 1);
    assert_complexity("java/lambda.java", "twice", 2, 1, 1, 1);
    assert_complexity("java/record.java", "Range", 2, 1, 1, 2);
    assert_complexity("java/logic.java", "logic", 3, 2, 0, 3);

    assert_complexity("javascript/empty.js", "empty", 1, 0, 0, 0);
    assert_complexity("javascript/decisions.js", "choose", 4, 5, 2, 1);
    assert_complexity("javascript/loops.js", "loops", 3, 3, 2, 1);
    assert_complexity("javascript/logic.js", "logic", 3, 2, 0, 3);
    assert_complexity("javascript/nested.js", "outer", 2, 1, 1, 1);
    assert_complexity("javascript/nested.js", "inner", 2, 1, 1, 1);

    assert_complexity("typescript/equivalent.ts", "positive", 2, 1, 1, 1);
    assert_complexity("typescript/logic.ts", "logic", 3, 2, 0, 3);
    assert_complexity("typescript/method.ts", "update", 2, 1, 1, 2);
    assert_complexity("typescript/anonymous.ts", "<anonymous@1:24>", 1, 0, 0, 1);
    assert_complexity("tsx/component.tsx", "Result", 2, 1, 1, 1);
    assert_complexity("javascript/throw.js", "fail", 3, 1, 1, 1);
    assert_complexity("java/arrow.java", "twice", 2, 0, 0, 1);
    assert_complexity("java/arrow.java", "op", 1, 0, 0, 1);
    assert_complexity("javascript/recursion.js", "fact", 2, 2, 1, 1);
    assert_complexity("java/recursion.java", "fact", 2, 2, 1, 1);

    assert_complexity("go/empty.go", "empty", 1, 0, 0, 0);
    assert_complexity("go/decisions.go", "choose", 4, 5, 2, 1);
    assert_complexity("go/logic.go", "logic", 3, 2, 0, 3);
    assert_complexity("go/method.go", "add", 1, 0, 0, 1);
    assert_complexity("go/recursion.go", "fact", 2, 2, 1, 1);
    assert_complexity("go/loop.go", "sum", 2, 1, 1, 1);
    assert_complexity("go/closure.go", "outer", 1, 0, 0, 1);
    assert_complexity("go/closure.go", "double", 1, 0, 0, 1);

    assert_complexity("rust/empty.rs", "empty", 1, 0, 0, 0);
    assert_complexity("rust/decisions.rs", "choose", 4, 5, 2, 1);
    assert_complexity("rust/logic.rs", "logic", 3, 2, 0, 3);
    assert_complexity("rust/loop.rs", "sum", 2, 1, 1, 1);
    assert_complexity("rust/recursion.rs", "fact", 2, 2, 1, 1);
    assert_complexity("rust/recursion.rs", "down", 2, 2, 1, 1);
    assert_complexity("rust/macros.rs", "log", 1, 0, 0, 1);
    assert_complexity("rust/macros.rs", "pick_macro", 1, 0, 0, 1);
    assert_complexity("rust/macros.rs", "guarded", 2, 2, 1, 1);
    assert_complexity("rust/macros.rs", "checked", 1, 0, 0, 1);
    assert_complexity("rust/macros.rs", "allow", 2, 1, 0, 1);
    assert_complexity("rust/method.rs", "add", 1, 0, 0, 1);
    assert_complexity("rust/closure.rs", "outer", 1, 0, 0, 1);
    assert_complexity("rust/closure.rs", "double", 1, 0, 0, 1);
    assert_complexity("rust/match_arms.rs", "label", 4, 1, 1, 1);
    assert_complexity("rust/match_arms.rs", "label_last", 4, 1, 1, 1);
    assert_complexity("rust/labels.rs", "find", 3, 4, 2, 2);
    assert_complexity("rust/let_chain.rs", "both", 3, 2, 1, 2);
    assert_complexity("rust/try_operator.rs", "parse", 2, 0, 0, 1);

    assert_complexity("c/empty.c", "empty", 1, 0, 0, 0);
    assert_complexity("c/decisions.c", "choose", 4, 5, 2, 1);
    assert_complexity("c/logic.c", "logic", 3, 2, 0, 3);
    assert_complexity("c/loops.c", "loops", 4, 6, 3, 1);
    assert_complexity("c/recursion.c", "fact", 2, 2, 1, 1);
    assert_complexity("c/switch.c", "classify", 3, 1, 1, 1);
    assert_complexity("c/ternary.c", "absolute", 2, 1, 1, 1);
    assert_complexity("c/goto.c", "jumps", 1, 2, 0, 0);
    assert_complexity("c/preprocessor.c", "preprocessed", 1, 0, 0, 1);
    assert_complexity("c/parameters.c", "none", 1, 0, 0, 0);
    assert_complexity("c/parameters.c", "variadic", 1, 0, 0, 1);
    assert_complexity("c/macros.c", "macro_body", 1, 0, 0, 1);
    assert_complexity("cpp/empty.cpp", "empty", 1, 0, 0, 0);
    assert_complexity("cpp/decisions.cpp", "choose", 4, 5, 2, 1);
    assert_complexity("cpp/exceptions.cpp", "parse", 2, 2, 1, 1);
    assert_complexity("cpp/lambda.cpp", "positive", 2, 1, 1, 1);
    assert_complexity("cpp/method.cpp", "add", 2, 1, 1, 1);
    assert_complexity("cpp/range.cpp", "sum", 2, 1, 1, 1);
    assert_complexity("cpp/template.cpp", "identity", 1, 0, 0, 1);
    assert_complexity("cpp/ternary.cpp", "sign", 2, 1, 1, 1);
    assert_complexity("cpp/header.h", "header_value", 1, 0, 0, 1);
    assert_complexity("cpp/parameters.cpp", "no_args", 1, 0, 0, 0);
    assert_complexity("cpp/parameters.cpp", "optional", 1, 0, 0, 1);

    assert_complexity("py/empty.py", "empty", 1, 0, 0, 0);
    assert_complexity("py/decisions.py", "choose", 4, 5, 2, 1);
    assert_complexity("py/logic.py", "logic", 3, 2, 0, 3);
    assert_complexity("py/loops.py", "loops", 3, 3, 2, 1);
    assert_complexity("py/loops.py", "for_else", 3, 4, 2, 1);
    assert_complexity("py/recursion.py", "factorial", 2, 2, 1, 1);
    assert_complexity("py/ternary.py", "ternary", 2, 1, 1, 1);
    assert_complexity("py/try_except.py", "parse", 4, 3, 1, 1);
    assert_complexity("py/match_case.py", "route", 4, 1, 1, 1);
    assert_complexity("py/comprehension.py", "comprehension", 3, 2, 1, 1);
    assert_complexity("py/lambda.py", "double", 1, 0, 0, 1);
    assert_complexity("py/method.py", "__init__", 1, 0, 0, 2);
    assert_complexity("py/method.py", "bump", 2, 1, 1, 2);
    assert_complexity("py/method.py", "repeat", 2, 2, 1, 2);
    assert_complexity("py/parameters.py", "parameters", 1, 0, 0, 4);
    assert_complexity("py/parameters.py", "keyword_only", 1, 0, 0, 3);
    assert_complexity("py/parameters.py", "positional_only", 1, 0, 0, 2);

    assert_complexity("zig/empty.zig", "empty", 1, 0, 0, 0);
    assert_complexity("zig/decisions.zig", "choose", 3, 3, 1, 1);
    assert_complexity("zig/decisions.zig", "loops", 4, 4, 2, 1);
    assert_complexity("zig/decisions.zig", "classify", 4, 1, 1, 1);
    assert_complexity("zig/decisions.zig", "labeled", 3, 6, 2, 1);
    assert_complexity("zig/logic.zig", "logic", 5, 4, 1, 3);
    assert_complexity("zig/logic.zig", "tryValue", 2, 1, 1, 1);
    assert_complexity("zig/recursion.zig", "fact", 2, 2, 1, 1);
    assert_complexity("zig/recursion.zig", "two", 1, 0, 0, 2);
    assert_complexity("zig/parameters.zig", "parameters", 1, 0, 0, 3);
    assert_complexity("zig/test.zig", "named", 2, 1, 1, 0);
    assert_complexity("zig/test.zig", "<anonymous@7:1>", 2, 1, 1, 0);
}

#[test]
fn zig_expression_else_try_literals_and_test_names_are_scored() {
    let source = br#"pub fn expression(a: bool, b: bool, c: bool) bool {
    return if (a) true else if (b) false else if (c) true else false;
}
pub fn literals() void {
    const text = "hello";
    const ch = 'x';
    const missing = undefined;
    const dead = unreachable;
}
pub fn attempt() !void {
    try consume();
}
pub extern fn prototype(value: i32) i32;
test namedIdentifier {
    return;
}
test {
    const text = "not a name";
}
"#;
    let file = leadline::analyze_source("custom.zig", source).unwrap();
    assert!(file.parse_errors.is_empty(), "{:?}", file.parse_errors);

    let names = file
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    assert!(!names.contains(&"prototype"), "{names:?}");
    assert!(names.contains(&"namedIdentifier"), "{names:?}");
    assert!(names.iter().any(|name| name.starts_with("<anonymous@")));
    assert!(
        !names.iter().any(|name| name.contains("not a name")),
        "{names:?}"
    );

    let expression = function(&file, "expression");
    assert_eq!(
        (
            expression.metrics.cyclomatic,
            expression.metrics.cognitive,
            expression.metrics.max_nesting,
            expression.metrics.parameters,
        ),
        (4, 4, 1, 3)
    );
    let rules = |analysis: &FileAnalysis, name: &str| {
        analysis
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .contributions
            .iter()
            .map(|contribution| contribution.rule.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rules(&file, "expression")
            .into_iter()
            .filter(|rule| matches!(rule.as_str(), "if" | "else-if" | "else"))
            .collect::<Vec<_>>(),
        ["if", "else-if", "else-if", "else"]
    );

    let attempt = function(&file, "attempt");
    assert_eq!(
        (
            attempt.metrics.cyclomatic,
            attempt.metrics.cognitive,
            attempt.metrics.max_nesting,
        ),
        (2, 0, 0)
    );
    assert!(rules(&file, "attempt").iter().any(|rule| rule == "try"));

    let literals = function(&file, "literals");
    assert_eq!(
        (
            literals.metrics.halstead_n2,
            literals.metrics.halstead_total_operands,
        ),
        (10, 10)
    );

    let tests = fixture("zig/test.zig");
    assert_eq!(function(&tests, "named").kind, FunctionKind::Function);
    assert_eq!(
        function(&tests, "<anonymous@7:1>").kind,
        FunctionKind::Function
    );
    assert_eq!(
        function(&file, "namedIdentifier").kind,
        FunctionKind::Function
    );
}

#[test]
fn go_method_kind_and_parity() {
    let file = fixture("go/method.go");
    assert_eq!(function(&file, "add").kind, FunctionKind::Method);
    // tree-sitter-java also emits `method_declaration`; only Go maps it to
    // Method, so Java methods keep their pre-Go Function classification.
    let java_file = fixture("java/decisions.java");
    assert_eq!(function(&java_file, "choose").kind, FunctionKind::Function);
    let go_file = fixture("go/decisions.go");
    let js_file = fixture("javascript/decisions.js");
    let go_choose = &function(&go_file, "choose").metrics;
    let js_choose = &function(&js_file, "choose").metrics;
    assert_eq!(go_choose.cyclomatic, js_choose.cyclomatic);
    assert_eq!(go_choose.cognitive, js_choose.cognitive);
    assert_eq!(go_choose.max_nesting, js_choose.max_nesting);
}

#[test]
fn rust_kinds_and_parity() {
    let method = fixture("rust/method.rs");
    assert_eq!(function(&method, "add").kind, FunctionKind::Method);
    let closure = fixture("rust/closure.rs");
    assert_eq!(function(&closure, "double").kind, FunctionKind::Lambda);

    let rust_file = fixture("rust/decisions.rs");
    let go_file = fixture("go/decisions.go");
    let rust_choose = &function(&rust_file, "choose").metrics;
    let go_choose = &function(&go_file, "choose").metrics;
    assert_eq!(rust_choose.cyclomatic, go_choose.cyclomatic);
    assert_eq!(rust_choose.cognitive, go_choose.cognitive);
    assert_eq!(rust_choose.max_nesting, go_choose.max_nesting);
}

#[test]
fn c_function_kind_and_javascript_parity() {
    let c_file = fixture("c/decisions.c");
    assert_eq!(function(&c_file, "choose").kind, FunctionKind::Function);
    let javascript_file = fixture("javascript/decisions.js");
    let c_choose = &function(&c_file, "choose").metrics;
    let javascript_choose = &function(&javascript_file, "choose").metrics;
    assert_eq!(c_choose.cyclomatic, javascript_choose.cyclomatic);
    assert_eq!(c_choose.cognitive, javascript_choose.cognitive);
    assert_eq!(c_choose.max_nesting, javascript_choose.max_nesting);
}

#[test]
fn go_grouped_parameters_survive_c_support() {
    let go = leadline::analyze_source(
        "grouped.go",
        b"package fixtures\nfunc grouped(a, b, c bool) {}\n",
    )
    .unwrap();
    assert_eq!(function(&go, "grouped").metrics.parameters, 3);
}

#[test]
fn c_only_discovers_function_definitions() {
    let file = leadline::analyze_source(
        "definitions.c",
        b"int forward(int value);\nint defined(int value) { return value; }\n",
    )
    .unwrap();
    assert_eq!(
        file.functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        ["defined"]
    );
}

#[test]
fn c_preprocessor_conditionals_only_add_logical_loc() {
    let file = fixture("c/preprocessor.c");
    let metrics = &function(&file, "preprocessed").metrics;
    assert_eq!(metrics.logical_loc, 4);
    assert_eq!((metrics.cyclomatic, metrics.cognitive), (1, 0));
}

#[test]
fn c_number_and_string_literals_are_halstead_operands() {
    let number =
        leadline::analyze_source("number.c", b"int probe(int value) { return value + 42; }\n")
            .unwrap();
    assert_eq!(function(&number, "probe").metrics.halstead_n2, 3);

    let string =
        leadline::analyze_source("string.c", b"const char *text(void) { return \"x\"; }\n")
            .unwrap();
    assert_eq!(function(&string, "text").metrics.halstead_n2, 2);
}

#[test]
fn cpp_kinds_and_parity() {
    let method = fixture("cpp/method.cpp");
    assert_eq!(function(&method, "add").kind, FunctionKind::Method);
    let lambda = fixture("cpp/lambda.cpp");
    assert_eq!(function(&lambda, "positive").kind, FunctionKind::Lambda);

    let cpp_file = fixture("cpp/decisions.cpp");
    let js_file = fixture("javascript/decisions.js");
    let cpp_choose = &function(&cpp_file, "choose").metrics;
    let js_choose = &function(&js_file, "choose").metrics;
    assert_eq!(cpp_choose.cyclomatic, js_choose.cyclomatic);
    assert_eq!(cpp_choose.cognitive, js_choose.cognitive);
    assert_eq!(cpp_choose.max_nesting, js_choose.max_nesting);
}

#[test]
fn python_kinds_and_javascript_parity() {
    let method = fixture("py/method.py");
    assert_eq!(function(&method, "bump").kind, FunctionKind::Method);
    let lambda = fixture("py/lambda.py");
    assert_eq!(function(&lambda, "double").kind, FunctionKind::Lambda);

    // `if/elif/else` is the same shape as JavaScript's `if/else if/else`, so
    // the parity test asserts cyclomatic, cognitive, and nesting together.
    let python_file = fixture("py/decisions.py");
    let javascript_file = fixture("javascript/decisions.js");
    let python_choose = &function(&python_file, "choose").metrics;
    let javascript_choose = &function(&javascript_file, "choose").metrics;
    assert_eq!(python_choose.cyclomatic, javascript_choose.cyclomatic);
    assert_eq!(python_choose.cognitive, javascript_choose.cognitive);
    assert_eq!(python_choose.max_nesting, javascript_choose.max_nesting);

    // `a and b or c` is the same logical sequence as `a && b || c`.
    let python_logic_file = fixture("py/logic.py");
    let javascript_logic_file = fixture("javascript/logic.js");
    let python_logic = &function(&python_logic_file, "logic").metrics;
    let javascript_logic = &function(&javascript_logic_file, "logic").metrics;
    assert_eq!(python_logic.cyclomatic, javascript_logic.cyclomatic);
    assert_eq!(python_logic.cognitive, javascript_logic.cognitive);
}

#[test]
fn python_elif_chain_is_a_chain_not_an_else() {
    let file = leadline::analyze_source(
        "chain.py",
        b"def chain(x):\n    if x == 1:\n        return 1\n    elif x == 2:\n        return 2\n    elif x == 3:\n        return 3\n    else:\n        return 0\n",
    )
    .unwrap();
    let metrics = &function(&file, "chain").metrics;
    // Three conditionals and one else, each worth one point, and no branch
    // raises the enclosing depth: the chain is flat.
    assert_eq!(
        (metrics.cyclomatic, metrics.cognitive, metrics.max_nesting),
        (4, 4, 1)
    );
}

#[test]
fn python_literals_and_keywords_are_halstead_leaves() {
    let number = leadline::analyze_source("probe.py", b"def probe():\n    return 42\n").unwrap();
    assert_eq!(function(&number, "probe").metrics.halstead_n2, 2);

    let text = leadline::analyze_source("probe.py", b"def probe():\n    return \"x\"\n").unwrap();
    assert_eq!(function(&text, "probe").metrics.halstead_n2, 2);

    let nothing = leadline::analyze_source("probe.py", b"def probe():\n    return None\n").unwrap();
    assert_eq!(function(&nothing, "probe").metrics.halstead_n2, 2);

    // `def`, `global`, `del`, and `lambda` are operator keywords the shared
    // punctuation table cannot name. The enclosing function sees `def`,
    // `global`, `del` and its own `:`, `=`; the bound lambda sees its own `:`
    // and `lambda`.
    let keywords = leadline::analyze_source(
        "probe.py",
        b"def probe():\n    global g\n    f = lambda v: v\n    del f\n",
    )
    .unwrap();
    assert_eq!(function(&keywords, "probe").metrics.halstead_n1, 5);
    assert_eq!(function(&keywords, "f").metrics.halstead_n1, 2);
}

#[test]
fn rust_macro_bodies_are_unscored_but_their_leaves_count() {
    let file = fixture("rust/macros.rs");
    let log = &function(&file, "log").metrics;
    assert_eq!(log.max_nesting, 0);
    assert!(
        log.halstead_total_operands > 0,
        "macro token leaves count as operands"
    );
    // `vec![if .. { } else { }]` is a token tree, so the branch never scores.
    assert_complexity("rust/macros.rs", "pick_macro", 1, 0, 0, 1);
    // `flag || true` holds three distinct operands across four occurrences:
    // the function name, the parameter, the body use, and the literal, which
    // the grammar reports as an anonymous token but still counts.
    let allow = &function(&file, "allow").metrics;
    assert_eq!(allow.halstead_n2, 3);
    assert_eq!(allow.halstead_total_operands, 4);
}

#[test]
fn equivalent_language_rules_match() {
    let java = fixture("java/decisions.java");
    let javascript = fixture("javascript/decisions.js");
    let java = &function(&java, "choose").metrics;
    let javascript = &function(&javascript, "choose").metrics;
    assert_eq!(java.cyclomatic, javascript.cyclomatic);
    assert_eq!(java.cognitive, javascript.cognitive);
    assert_eq!(java.max_nesting, javascript.max_nesting);

    let java = fixture("java/equivalent.java");
    let typescript = fixture("typescript/equivalent.ts");
    let java = &function(&java, "positive").metrics;
    let typescript = &function(&typescript, "positive").metrics;
    assert_eq!(java.cyclomatic, typescript.cyclomatic);
    assert_eq!(java.cognitive, typescript.cognitive);
    assert_eq!(java.max_nesting, typescript.max_nesting);

    let java = fixture("java/logic.java");
    let typescript = fixture("typescript/logic.ts");
    let java = &function(&java, "logic").metrics;
    let typescript = &function(&typescript, "logic").metrics;
    assert_eq!(java.cyclomatic, typescript.cyclomatic);
    assert_eq!(java.cognitive, typescript.cognitive);
}

#[test]
fn nested_functions_are_independent() {
    let file = fixture("javascript/nested.js");
    let outer = function(&file, "outer");
    let inner = function(&file, "inner");
    assert_eq!(outer.metrics.cyclomatic, 2);
    assert_eq!(inner.metrics.cyclomatic, 2);
    assert!(outer.start_line < inner.start_line);
}

#[test]
fn size_and_halstead_fixtures_have_explicit_results() {
    let decisions = fixture("javascript/decisions.js");
    let decisions = &function(&decisions, "choose").metrics;
    assert_eq!(
        (
            decisions.loc,
            decisions.logical_loc,
            decisions.function_length,
            decisions.halstead_n1,
            decisions.halstead_n2,
            decisions.halstead_total_operators,
            decisions.halstead_total_operands,
        ),
        (12, 7, 12, 6, 6, 13, 12)
    );

    let logic = fixture("javascript/logic.js");
    let logic = &function(&logic, "logic").metrics;
    assert!((logic.halstead_volume - 28.073_549_220_576_04).abs() < f64::EPSILON);
    assert!((logic.halstead_difficulty - 2.625).abs() < f64::EPSILON);
    assert!((logic.halstead_effort - 73.693_066_704_012_11).abs() < f64::EPSILON);
    assert!((logic.maintainability_index - 79.047_588_443_260_75).abs() < f64::EPSILON);
}

#[test]
fn parse_diagnostics_are_explicit() {
    assert!(fixture("typescript/method.ts").parse_errors.is_empty());
    let malformed =
        leadline::analyze_source("malformed.ts", b"function broken( { return 1; }\n").unwrap();
    assert!(!malformed.parse_errors.is_empty());
    assert_eq!(malformed.parse_errors[0].start_line, 1);
}

#[test]
fn output_is_deterministic() {
    let first = serde_json::to_string(&fixture("typescript/method.ts")).unwrap();
    let second = serde_json::to_string(&fixture("typescript/method.ts")).unwrap();
    assert_eq!(first, second);
}

#[test]
fn functions_carry_stable_ids_and_byte_spans() {
    for relative in [
        "java/decisions.java",
        "java/lambda.java",
        "javascript/decisions.js",
        "javascript/nested.js",
        "typescript/method.ts",
        "typescript/anonymous.ts",
        "tsx/component.tsx",
        "rust/decisions.rs",
        "rust/closure.rs",
        "cpp/decisions.cpp",
        "cpp/lambda.cpp",
    ] {
        let file = fixture(relative);
        assert!(!file.functions.is_empty(), "no functions in {relative}");
        for function in &file.functions {
            assert!(!function.id.is_empty(), "empty id in {relative}");
            assert!(
                function.id.starts_with(file.path.as_str()),
                "id {} does not start with {}",
                function.id,
                file.path
            );
            assert!(
                function.start_byte <= function.end_byte,
                "inverted byte span in {}",
                function.id
            );
        }
    }
}

#[test]
fn reports_carry_analyzer_version_and_metric_specs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let report = leadline::analyze_path(&root, None).unwrap();
    assert_eq!(report.analyzer_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(report.metric_specs.cyclomatic, "default");
    assert_eq!(report.metric_specs.cognitive, "default");
    assert_eq!(report.metric_specs.halstead, "default");
    assert_eq!(report.metric_specs.maintainability, "default");
    assert_eq!(report.metric_specs.crap, "default");
}
