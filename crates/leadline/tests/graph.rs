mod common;

use leadline::graph::{DEPENDENCY_SCHEMA_VERSION, DependencyReport, analyze_dependencies};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-graph-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, source).unwrap();
}

fn edge_pairs(report: &DependencyReport) -> Vec<(&str, &str)> {
    report
        .edges
        .iter()
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect()
}

#[test]
fn python_relative_imports_resolve_to_files_packages_and_ancestors() {
    let root = temporary_directory();
    write(
        &root,
        "app/svc/api.py",
        "from . import models\nfrom .helpers import run\nfrom .. import shared\nfrom ..util.text import clean\nimport os.path\nfrom os import path\n",
    );
    write(&root, "app/svc/models.py", "VALUE = 1\n");
    write(
        &root,
        "app/svc/helpers/__init__.py",
        "def run():\n    return 1\n",
    );
    write(&root, "app/__init__.py", "");
    write(&root, "app/shared.py", "SHARED = 1\n");
    write(&root, "app/util/__init__.py", "");
    write(
        &root,
        "app/util/text.py",
        "def clean(value):\n    return value\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();

    // Level 1 names a sibling, level 2 climbs to the parent package, and a
    // dotted module path resolves through the directory tree.
    assert_eq!(
        edge_pairs(&report),
        [
            ("app/svc/api.py", "app/shared.py"),
            ("app/svc/api.py", "app/svc/helpers/__init__.py"),
            ("app/svc/api.py", "app/svc/models.py"),
            ("app/svc/api.py", "app/util/text.py"),
        ]
    );
    // Absolute imports are package-qualified, so no parser can resolve them;
    // they are reported as unsupported instead of disappearing.
    let unresolved: Vec<(&str, &str)> = report
        .unresolved
        .iter()
        .map(|entry| (entry.specifier.as_str(), entry.reason))
        .collect();
    assert_eq!(
        unresolved,
        [("os", "unsupported"), ("os.path", "unsupported")]
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn python_relative_imports_report_missing_ambiguous_and_out_of_scope() {
    let root = temporary_directory();
    write(
        &root,
        "pkg/mod.py",
        "from .missing import a\nfrom .either import value\nfrom . import present\n",
    );
    write(&root, "pkg/either.py", "either = 1\n");
    write(&root, "pkg/either/__init__.py", "package = 1\n");
    write(&root, "pkg/present.py", "present = 1\n");
    write(&root, "top.py", "from ..above import b\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(edge_pairs(&report), [("pkg/mod.py", "pkg/present.py")]);
    let unresolved: Vec<(&str, &str, u32, &str)> = report
        .unresolved
        .iter()
        .map(|entry| {
            (
                entry.source.as_str(),
                entry.specifier.as_str(),
                entry.line,
                entry.reason,
            )
        })
        .collect();
    assert_eq!(
        unresolved,
        [
            ("pkg/mod.py", "either", 2, "ambiguous"),
            ("pkg/mod.py", "missing", 1, "not_found"),
            ("top.py", "../above", 1, "outside_scope"),
        ]
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolves_javascript_and_typescript_references_and_reports_cycles() {
    let root = temporary_directory();
    write(
        &root,
        "src/a.ts",
        r#"
            import "./b";
            import { b } from "./b.js";
            export { value } from "./folder";
            export * from "./exported";
            const c = require("./c");
            const d = import("./d");
            import thing from "third-party";
        "#,
    );
    write(&root, "src/b.ts", "import './a'; export const b = 1;\n");
    write(&root, "src/folder/index.tsx", "export const value = 1;\n");
    write(&root, "src/exported.mts", "export const value = 1;\n");
    write(&root, "src/c.cts", "module.exports = 1;\n");
    write(&root, "src/d.js", "export const d = 1;\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(report.schema_version, DEPENDENCY_SCHEMA_VERSION);
    assert_eq!(
        edge_pairs(&report),
        [
            ("src/a.ts", "src/b.ts"),
            ("src/a.ts", "src/c.cts"),
            ("src/a.ts", "src/d.js"),
            ("src/a.ts", "src/exported.mts"),
            ("src/a.ts", "src/folder/index.tsx"),
            ("src/b.ts", "src/a.ts"),
        ]
    );
    let a = report
        .files
        .iter()
        .find(|file| file.path == "src/a.ts")
        .unwrap();
    let b = report
        .files
        .iter()
        .find(|file| file.path == "src/b.ts")
        .unwrap();
    assert_eq!((a.fan_in, a.fan_out), (1, 5));
    assert_eq!((b.fan_in, b.fan_out), (1, 1));
    let kinds: Vec<(&str, &str)> = report
        .edges
        .iter()
        .filter(|edge| edge.source == "src/a.ts")
        .map(|edge| (edge.target.as_str(), edge.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("src/b.ts", "import"),
            ("src/c.cts", "call"),
            ("src/d.js", "call"),
            ("src/exported.mts", "import"),
            ("src/folder/index.tsx", "import"),
        ]
    );
    assert_eq!(report.cycles[0].files, ["src/a.ts", "src/b.ts"]);
    assert!(report.unresolved.is_empty(), "package imports are ignored");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn zig_imports_resolve_local_files() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.zig",
        include_str!("fixtures/zig/imports.zig"),
    );
    write(&root, "src/helper.zig", "pub fn run() void {}\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(edge_pairs(&report), [("src/main.zig", "src/helper.zig")]);
    let unresolved: Vec<(&str, &str, u32, &str)> = report
        .unresolved
        .iter()
        .map(|entry| {
            (
                entry.source.as_str(),
                entry.specifier.as_str(),
                entry.line,
                entry.reason,
            )
        })
        .collect();
    assert_eq!(
        unresolved,
        [
            ("src/main.zig", "../../outside.zig", 4, "outside_scope"),
            ("src/main.zig", "./styles.css", 5, "unsupported"),
            ("src/main.zig", "missing.zig", 3, "not_found"),
        ]
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn zig_import_skips_comments_before_argument() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.zig",
        "const helper = @import(// path comment\n\"helper.zig\");\n",
    );
    write(&root, "src/helper.zig", "pub fn run() void {}\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(edge_pairs(&report), [("src/main.zig", "src/helper.zig")]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn zig_imports_reject_absolute_paths() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.zig",
        r#"const root_absolute = @import("/helper.zig");
const drive_absolute = @import("C:/helper.zig");
const unc = @import("\\server\\share\\helper.zig");
"#,
    );
    write(&root, "src/helper.zig", "pub fn run() void {}\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert!(edge_pairs(&report).is_empty());
    let unresolved: Vec<(&str, &str)> = report
        .unresolved
        .iter()
        .map(|entry| (entry.specifier.as_str(), entry.reason))
        .collect();
    assert_eq!(
        unresolved,
        [
            ("/helper.zig", "outside_scope"),
            ("C:/helper.zig", "outside_scope"),
            (r"\\server\\share\\helper.zig", "outside_scope")
        ]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn c_local_includes_resolve_relative_paths_and_report_only_missing_quotes() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.c",
        "#include \"detail/value.c\"\n#include \"missing.c\"\n#include <stdio.h>\n#include HEADER\nint main(void) { return value(); }\n",
    );
    write(
        &root,
        "src/detail/value.c",
        "int value(void) { return 1; }\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(edge_pairs(&report), [("src/main.c", "src/detail/value.c")]);
    assert_eq!(report.unresolved.len(), 1);
    assert_eq!(report.unresolved[0].source, "src/main.c");
    assert_eq!(report.unresolved[0].specifier, "missing.c");
    assert_eq!(report.unresolved[0].line, 2);
    assert_eq!(report.unresolved[0].reason, "not_found");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cpp_local_includes_resolve_and_report_missing_files() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.cpp",
        "#include \"../include/api.hpp\"\n#include \"missing.hpp\"\n#include <vector>\n#include HEADER_FILE\nint main() { return api(); }\n",
    );
    write(&root, "include/api.hpp", "int api();\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(edge_pairs(&report), [("src/main.cpp", "include/api.hpp")]);
    let unresolved: Vec<(&str, &str)> = report
        .unresolved
        .iter()
        .map(|entry| (entry.specifier.as_str(), entry.reason))
        .collect();
    assert_eq!(unresolved, [("missing.hpp", "not_found")]);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rust_modules_resolve_to_files() {
    let root = temporary_directory();
    write(
        &root,
        "src/lib.rs",
        "mod parser;\nmod util;\npub use parser::run;\n",
    );
    write(&root, "src/parser.rs", "pub fn run() {}\n");
    write(&root, "src/util/mod.rs", "pub fn helper() {}\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(
        edge_pairs(&report),
        [
            ("src/lib.rs", "src/parser.rs"),
            ("src/lib.rs", "src/util/mod.rs")
        ]
    );
    assert!(
        report.unresolved.is_empty(),
        "use paths are module-qualified and ignored"
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rust_module_declarations_resolve_from_the_module_directory() {
    let root = temporary_directory();
    write(&root, "src/lib.rs", "mod telemetry;\nmod parser;\n");
    write(&root, "src/telemetry.rs", "mod counters;\nmod sampler;\n");
    write(&root, "src/telemetry/counters.rs", "pub fn count() {}\n");
    write(&root, "src/telemetry/sampler.rs", "pub fn sample() {}\n");
    write(&root, "src/parser/mod.rs", "mod rust;\n");
    write(&root, "src/parser/rust.rs", "pub fn parse() {}\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(
        edge_pairs(&report),
        [
            ("src/lib.rs", "src/parser/mod.rs"),
            ("src/lib.rs", "src/telemetry.rs"),
            ("src/parser/mod.rs", "src/parser/rust.rs"),
            ("src/telemetry.rs", "src/telemetry/counters.rs"),
            ("src/telemetry.rs", "src/telemetry/sampler.rs"),
        ]
    );
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rust_modules_handle_inline_bodies_ambiguity_and_misses() {
    let root = temporary_directory();
    write(
        &root,
        "src/lib.rs",
        "mod inline {\n    mod nested;\n}\nmod ambiguous;\nmod missing;\n",
    );
    write(&root, "src/inline/nested.rs", "pub fn nested() {}\n");
    write(&root, "src/ambiguous.rs", "pub fn a() {}\n");
    write(&root, "src/ambiguous/mod.rs", "pub fn b() {}\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(
        edge_pairs(&report),
        [("src/lib.rs", "src/inline/nested.rs")]
    );
    let unresolved: Vec<(&str, &str)> = report
        .unresolved
        .iter()
        .map(|entry| (entry.specifier.as_str(), entry.reason))
        .collect();
    assert_eq!(
        unresolved,
        [("ambiguous", "ambiguous"), ("missing", "not_found")]
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn supports_js_tsx_reexports_and_unique_emitted_javascript_resolution() {
    let root = temporary_directory();
    write(
        &root,
        "app/main.js",
        r#"
            export { Widget } from "./widget.js";
            import("./lazy.mjs");
        "#,
    );
    write(
        &root,
        "app/view.tsx",
        "import Component from './component.jsx';\nexport default Component;\n",
    );
    write(&root, "app/widget.ts", "export const Widget = 1;\n");
    write(&root, "app/lazy.mts", "export const lazy = 1;\n");
    write(
        &root,
        "app/component.jsx",
        "export default function Component() {}\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(
        edge_pairs(&report),
        [
            ("app/main.js", "app/lazy.mts"),
            ("app/main.js", "app/widget.ts"),
            ("app/view.tsx", "app/component.jsx"),
        ]
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn decodes_javascript_utf16_surrogate_pair_specifiers() {
    let root = temporary_directory();
    write(&root, "src/main.ts", r#"import "./\uD83D\uDE00";"#);
    write(&root, "src/😀.ts", "export const value = 1;\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(edge_pairs(&report), [("src/main.ts", "src/😀.ts")]);
    assert!(report.unresolved.is_empty());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ignores_dot_prefixed_bare_javascript_specifiers() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.ts",
        "import '.package';\nconst value = require('..package');\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert!(report.edges.is_empty());
    assert!(report.unresolved.is_empty());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unresolved_relative_references_are_conservative_and_explainable() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.ts",
        r#"
            import "./missing";
            import "./ambiguous";
            import "../../../outside";
            import "./styles.css";
            import "./emitted.js";
            import value from "package-name";
            import(variable);
        "#,
    );
    write(&root, "src/ambiguous.ts", "export const one = 1;\n");
    write(&root, "src/ambiguous.tsx", "export const two = 2;\n");
    write(&root, "src/emitted.ts", "export const one = 1;\n");
    write(&root, "src/emitted.tsx", "export const two = 2;\n");
    write(&root, "outside.ts", "export const outside = 1;\n");

    let report = analyze_dependencies(&root, &[]).unwrap();
    let reasons: Vec<(&str, &str)> = report
        .unresolved
        .iter()
        .map(|item| (item.specifier.as_str(), item.reason))
        .collect();

    assert_eq!(
        reasons,
        [
            ("../../../outside", "outside_scope"),
            ("./ambiguous", "ambiguous"),
            ("./emitted.js", "ambiguous"),
            ("./missing", "not_found"),
            ("./styles.css", "unsupported"),
        ]
    );
    assert!(report.edges.is_empty());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolves_java_exact_and_static_imports_but_not_wildcards_or_ambiguous_types() {
    let root = temporary_directory();
    write(
        &root,
        "java/app/Main.java",
        r#"
            package app;
            import model.Service;
            import static util.Tools.run;
            import static util.Outer.Inner.member;
            import wildcard.*;
            import duplicate.Value;
            class Main {}
        "#,
    );
    write(
        &root,
        "java/model/Service.java",
        "package model; public class Service {}\n",
    );
    write(
        &root,
        "java/util/Tools.java",
        "package util; public class Tools {}\n",
    );
    write(
        &root,
        "java/util/Outer.java",
        "package util; public class Outer { static class Inner {} }\n",
    );
    write(
        &root,
        "java/one/Value.java",
        "package duplicate; public class Value {}\n",
    );
    write(
        &root,
        "java/two/Other.java",
        "package duplicate; class Value {}\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(
        edge_pairs(&report),
        [
            ("java/app/Main.java", "java/model/Service.java"),
            ("java/app/Main.java", "java/util/Outer.java"),
            ("java/app/Main.java", "java/util/Tools.java"),
        ]
    );
    let reasons: Vec<(&str, &str)> = report
        .unresolved
        .iter()
        .map(|item| (item.specifier.as_str(), item.reason))
        .collect();
    assert_eq!(
        reasons,
        [
            ("duplicate.Value", "ambiguous"),
            ("wildcard.*", "unsupported")
        ]
    );

    std::fs::remove_dir_all(root).unwrap();
}

/// Go import paths are module-qualified, so no edge can be resolved. They must
/// still be reported as unsupported rather than dropped, or a Go repository
/// looks like it has no dependencies at all and `unused` reports a complete
/// verdict off an empty graph.
#[test]
fn go_imports_are_reported_unsupported_instead_of_dropped() {
    let root = temporary_directory();
    write(
        &root,
        "go.mod",
        "module github.com/example/svc\n\ngo 1.24\n",
    );
    write(
        &root,
        "cmd/server/main.go",
        "package main\n\nimport (\n\t\"fmt\"\n\n\t\"github.com/example/svc/internal/domain\"\n)\n\nfunc main() { fmt.Println(domain.Name) }\n",
    );
    write(
        &root,
        "internal/domain/skill.go",
        "package domain\n\nvar Name = \"x\"\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();
    assert!(report.edges.is_empty(), "no Go edge is resolvable");
    let reasons: Vec<(&str, &str)> = report
        .unresolved
        .iter()
        .map(|item| (item.specifier.as_str(), item.reason))
        .collect();
    assert_eq!(
        reasons,
        [
            ("fmt", "unsupported"),
            ("github.com/example/svc/internal/domain", "unsupported")
        ]
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn does_not_treat_java_package_prefix_as_static_keyword() {
    let root = temporary_directory();
    write(
        &root,
        "java/app/Main.java",
        "package app; import staticpkg.Outer.Missing; class Main {}\n",
    );
    write(
        &root,
        "java/staticpkg/Outer.java",
        "package staticpkg; public class Outer {}\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert!(report.edges.is_empty());
    assert_eq!(report.unresolved.len(), 1);
    assert_eq!(report.unresolved[0].specifier, "staticpkg.Outer.Missing");
    assert_eq!(report.unresolved[0].reason, "not_found");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn excludes_are_out_of_scope_and_reports_are_deterministic() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.ts",
        "import './included';\nimport './ignored';\nimport './included';\n",
    );
    write(&root, "src/included.ts", "export const value = 1;\n");
    write(&root, "src/ignored.ts", "export const ignored = 1;\n");
    let excludes = vec!["src/ignored.ts".to_owned()];

    let first = analyze_dependencies(&root, &excludes).unwrap();
    let second = analyze_dependencies(&root, &excludes).unwrap();

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(edge_pairs(&first), [("src/main.ts", "src/included.ts")]);
    assert_eq!(
        first
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["src/included.ts", "src/main.ts"]
    );
    assert_eq!(first.unresolved[0].specifier, "./ignored");
    assert_eq!(first.unresolved[0].reason, "not_found");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn undecodable_specifiers_are_unsupported_not_silent() {
    let root = temporary_directory();
    write(&root, "src/main.ts", "import './\\uZZZZ';\n");

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert!(report.edges.is_empty());
    assert_eq!(report.unresolved.len(), 1);
    assert_eq!(report.unresolved[0].specifier, "./\\uZZZZ");
    assert_eq!(report.unresolved[0].reason, "unsupported");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn self_import_counts_but_forms_no_cycle() {
    let root = temporary_directory();
    write(
        &root,
        "src/self.ts",
        "import './self';\nexport const value = 1;\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(edge_pairs(&report), [("src/self.ts", "src/self.ts")]);
    let file = &report.files[0];
    assert_eq!((file.fan_in, file.fan_out), (1, 1));
    assert!(report.cycles.is_empty());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn three_file_cycle_is_sorted_and_import_wins_over_call() {
    let root = temporary_directory();
    write(&root, "src/c.ts", "import './a';\nexport const c = 1;\n");
    write(&root, "src/a.ts", "import './b';\nexport const a = 1;\n");
    write(
        &root,
        "src/b.ts",
        "import './c';\nconst c = require('./c');\nexport const b = 1;\n",
    );

    let report = analyze_dependencies(&root, &[]).unwrap();

    assert_eq!(
        edge_pairs(&report),
        [
            ("src/a.ts", "src/b.ts"),
            ("src/b.ts", "src/c.ts"),
            ("src/c.ts", "src/a.ts"),
        ]
    );
    let edge = report
        .edges
        .iter()
        .find(|edge| edge.source == "src/b.ts")
        .unwrap();
    assert_eq!(edge.kind, "import");
    assert_eq!(
        report.cycles.len(),
        1,
        "one strongly connected component, not three pairs"
    );
    assert_eq!(report.cycles[0].files, ["src/a.ts", "src/b.ts", "src/c.ts"]);

    std::fs::remove_dir_all(root).unwrap();
}

fn cli_fixture() -> PathBuf {
    let root = temporary_directory();
    write(&root, "src/a.ts", "import './b';\nexport const a = 1;\n");
    write(&root, "src/b.ts", "import './a';\nexport const b = 1;\n");
    root
}

#[test]
fn cli_dependencies_terminal_reports_counts_and_cycles() {
    let root = cli_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["dependencies"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Dependencies"),
        "missing header in:\n{stdout}"
    );
    assert!(
        stdout.contains("2 files"),
        "missing file count in:\n{stdout}"
    );
    assert!(
        stdout.contains("2 edges"),
        "missing edge count in:\n{stdout}"
    );
    assert!(
        stdout.to_lowercase().contains("cycle"),
        "missing cycle in:\n{stdout}"
    );
    assert!(
        stdout.contains("src/a.ts"),
        "missing fan-in/out row in:\n{stdout}"
    );
    assert!(
        stdout.contains("src/b.ts"),
        "missing fan-in/out row in:\n{stdout}"
    );
    assert!(
        stdout.contains("Top fan-in"),
        "missing top fan-in in:\n{stdout}"
    );
    assert!(
        stdout.contains("Top fan-out"),
        "missing top fan-out in:\n{stdout}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dependencies_json_reports_canonical_counts() {
    let root = cli_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["dependencies", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], DEPENDENCY_SCHEMA_VERSION);
    assert_eq!(value["files"].as_array().unwrap().len(), 2);
    assert_eq!(value["edges"].as_array().unwrap().len(), 2);
    assert_eq!(value["cycles"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["cycles"][0]["files"],
        serde_json::json!(["src/a.ts", "src/b.ts"])
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dependencies_agent_json_is_compact() {
    let root = cli_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["dependencies", "--format", "agent-json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["files"], 2);
    assert_eq!(value["summary"]["edges"], 2);
    assert_eq!(value["summary"]["cycles"], 1);
    assert_eq!(value["truncated"], false);
    assert_eq!(value["files"].as_array().unwrap().len(), 2);
    assert_eq!(value["edges"].as_array().unwrap().len(), 2);
    assert!(
        value["edges"][0].get("confidence").is_none(),
        "agent edges must drop confidence"
    );
    assert!(
        value.get("analyzer_version").is_none(),
        "agent view must stay compact"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dependencies_json_is_deterministic() {
    let root = cli_fixture();
    let args = ["dependencies", "--json"];
    let first = common::leadline()
        .current_dir(&root)
        .args(args)
        .output()
        .unwrap();
    let second = common::leadline()
        .current_dir(&root)
        .args(args)
        .output()
        .unwrap();
    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dependencies_rejects_unknown_format() {
    let root = cli_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["dependencies", "--format", "xml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown --format"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dependencies_honors_config_excludes() {
    let root = temporary_directory();
    write(&root, "src/main.ts", "import './keep';\nimport './skip';\n");
    write(&root, "src/keep.ts", "export const keep = 1;\n");
    write(&root, "src/skip.ts", "export const skip = 1;\n");
    std::fs::write(
        root.join("leadline.toml"),
        "[analysis]\nexclude = [\"src/skip.ts\"]\n",
    )
    .unwrap();
    let output = common::leadline()
        .current_dir(&root)
        .args(["dependencies", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let paths: Vec<&str> = value["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"src/main.ts"));
    assert!(paths.contains(&"src/keep.ts"));
    assert!(
        !paths.contains(&"src/skip.ts"),
        "excluded file leaked: {paths:?}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dependencies_help_lists_the_command() {
    let output = common::leadline()
        .args(["dependencies", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("leadline dependencies"));
}

#[test]
fn cli_dependencies_empty_scope_is_incomplete() {
    let root = temporary_directory();
    let output = common::leadline()
        .current_dir(&root)
        .args(["dependencies", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no supported files found"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dependencies_agent_json_reports_unresolved() {
    let root = temporary_directory();
    write(
        &root,
        "src/main.ts",
        "import './present';\nimport './missing';\n",
    );
    write(&root, "src/present.ts", "export const present = 1;\n");
    let output = common::leadline()
        .current_dir(&root)
        .args(["dependencies", "--format", "agent-json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["unresolved"], 1);
    assert_eq!(value["unresolved"].as_array().unwrap().len(), 1);
    assert_eq!(value["unresolved"][0]["source"], "src/main.ts");
    assert_eq!(value["unresolved"][0]["specifier"], "./missing");
    assert_eq!(value["unresolved"][0]["reason"], "not_found");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_packages_from_changed_sources() {
    use leadline::graph::external_packages_from_sources;
    use leadline::source_snapshot::SourceEntry;
    let entries = vec![
        SourceEntry {
            path: "src/new.ts".to_owned(),
            bytes: b"import _ from 'lodash';
const m = require('minimist');
const c = await import('@scope/client/sub');
import './local';
import fs from 'node:fs';
const fsp = require('node:fs/promises');
"
            .to_vec(),
        },
        SourceEntry {
            path: "src/again.ts".to_owned(),
            bytes: b"import _ from 'lodash/sub';
"
            .to_vec(),
        },
    ];
    let imports = external_packages_from_sources(&entries).unwrap();
    let packages: Vec<&str> = imports.iter().map(|item| item.package.as_str()).collect();
    assert_eq!(
        packages,
        vec!["@scope/client", "lodash", "lodash", "minimist"]
    );
    assert_eq!(imports[0].path, "src/new.ts");
    assert!(imports.iter().all(|item| item.line >= 1));
    // Relative imports and Node builtins never surface as external packages.
    assert!(imports.iter().all(|item| !item.package.starts_with('.')));
    assert!(
        imports
            .iter()
            .all(|item| !item.package.starts_with("node:"))
    );
}
