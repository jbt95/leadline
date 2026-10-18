//! Test-file discovery by convention.
//!
//! Two predicates, and the split between them is deliberate:
//!
//! - [`is_test_file`] reads a path only and never touches the filesystem. It
//!   answers the per-language rows of the design's convention table: the
//!   file-name forms (`*_test.go`, `*.test.ts`, `*Tests.java`, `test_*.py`,
//!   ...) and the two directory layouts a runner actually compiles — Rust's
//!   `tests/` integration targets and the JavaScript ecosystem's `__tests__/`.
//! - [`declares_tests`] reads bytes, for the two rows a path cannot answer: a
//!   Rust file declaring `#[test]`, and a Zig file declaring `test "..."`.
//!   Selection applies it only to files the reverse closure of a changed file
//!   already reaches, so the reading is bounded by the blast radius of the diff
//!   rather than by the size of the repository.
//!
//! `config.test_globs` is **additive** to the built-in conventions, never a
//! replacement. A candidate must also be a file `parse::discovery` accepts, so
//! an unsupported extension or an ignored directory (`node_modules`, `vendor`,
//! `target`, ...) is never a test file, and `config.exclude` removes candidates
//! the same way it removes analysis sources.
//!
//! Directory layouts are language-specific, never a blanket "any `tests/`
//! component" rule. Rust compiles only a *top-level* file under `tests/`
//! (`tests/foo.rs`) or a directory target (`tests/<name>/main.rs`) as an
//! integration test; `tests/common/mod.rs` is a module the harness never runs
//! on its own. TypeScript, TSX, and JavaScript keep their `__tests__/`
//! directory, and Java keeps its `src/test/` build layout. Every other language
//! is named rather than placed. Matching a file that is not a test over-selects
//! and a missing one under-selects, so where a layout is genuine the generous
//! reading stays; the fix is to stop claiming layouts no runner compiles.
//!
//! Two shared exclusions sit above the per-language rules, split by how strong
//! the convention's signal is:
//!
//! - A **data** tree — `fixtures`, `testdata`, `snapshots`, `__snapshots__`,
//!   `__mocks__` — outranks every row, including a name that says "test".
//!   Nothing in `fixtures/` is executed by a runner, whatever it is called.
//! - A **helper** tree — `common`, `helpers`, `support` — outranks only the
//!   **directory-layout** rows, where the directory is the entire signal. A
//!   test's own name outranks where it sits: `src/common/app.test.ts` is a
//!   test, while an unnamed Java class in `src/test/java/.../helpers/` is not.
//!   Excluding the helper names from name-form matches as well made a whole
//!   source subtree invisible, and since the ladder then found no test it
//!   escalated that file's top-level directory — every test under `src/` for
//!   one changed file.
//!
//! `declares_tests` scans for the attribute rather than resolving it, so a
//! `#[test]` inside a comment or a string counts. That direction is the safe
//! one too, and the alternative — a second parse of every candidate — buys
//! nothing a test selection needs.

use analysis::duplication::ExcludeMatcher;
use config::TiaConfig;
use model::core::Language;
use parse::discovery::SourceFilter;
use parse::parser::detect_language;
use std::path::Path;

/// True when `path` is a test file by convention, discovery, and configuration.
///
/// Path-shaped rows only; see [`declares_tests`] for the rows that need the
/// file's bytes.
pub fn is_test_file(path: &str, config: &TiaConfig) -> bool {
    accepts_candidate(path, config)
        && (matches_builtin(path)
            || (!config.test_globs.is_empty()
                && ExcludeMatcher::new(&config.test_globs).matches(path)))
}

/// True when `path` is a file `parse::discovery` accepts and `config` does not
/// exclude: the gate every candidate, path-shaped or content-detected, passes
/// first.
pub(crate) fn accepts_candidate(path: &str, config: &TiaConfig) -> bool {
    let Ok(accepted) = SourceFilter::new(Path::new(""), &config.exclude) else {
        // `config` validates the patterns, so this is unreachable in practice;
        // rejecting every candidate is the safe reading of a broken matcher.
        return false;
    };
    accepted.accepts_file(Path::new(path))
}

/// True when `source` declares tests the path cannot name.
///
/// Rust files declaring `#[test]` and Zig files declaring `test "..."` are
/// test files wherever they sit — the design's `src/**` restriction is not
/// applied, because the question this answers is "does this file carry tests",
/// and the content check only ever runs on a file the closure already reached.
/// Every other grammar is answered by [`is_test_file`] alone, so any other
/// `language` yields `false`.
pub fn declares_tests(source: &[u8], language: Language) -> bool {
    match language {
        Language::Rust => contains(source, b"#[test]"),
        Language::Zig => contains(source, b"test \""),
        _ => false,
    }
}

fn contains(source: &[u8], needle: &[u8]) -> bool {
    source.windows(needle.len()).any(|window| window == needle)
}

/// The built-in conventions: the name form of the path's language, plus the
/// directory layout that language's runner compiles.
///
/// The two are excluded differently on purpose. A name form carries its own
/// evidence, so only a data tree suppresses it; a layout is the entire signal,
/// so a helper tree suppresses it too.
fn matches_builtin(path: &str) -> bool {
    let Some(language) = detect_language(path) else {
        return false;
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    if matches_builtin_name(name, language) {
        return !in_data_tree(path);
    }
    !in_helper_or_data_tree(path) && matches_builtin_layout(path, language)
}

/// The directory layout a runner compiles, by language. A language absent here
/// has a name form only.
fn matches_builtin_layout(path: &str, language: Language) -> bool {
    match language {
        Language::Rust => is_rust_integration_test(path),
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            path.split('/').any(|component| component == "__tests__")
        }
        Language::Java => in_source_test_tree(path),
        _ => false,
    }
}

/// Rust compiles a *top-level* file under `tests/` (`tests/foo.rs`) or a
/// directory target (`tests/<name>/main.rs`) as an integration test. A deeper
/// module such as `tests/common/mod.rs` is a helper, not a target.
fn is_rust_integration_test(path: &str) -> bool {
    let components: Vec<&str> = path.split('/').collect();
    let Some(index) = components
        .iter()
        .rposition(|component| *component == "tests")
    else {
        return false;
    };
    match &components[index + 1..] {
        [file] => file.ends_with(".rs"),
        [_, "main.rs"] => true,
        _ => false,
    }
}

/// True when a component of `path` names a data directory: golden files,
/// snapshots, and mocks a test reads rather than anything a runner executes.
/// Strong enough to outrank a test-shaped file name.
fn in_data_tree(path: &str) -> bool {
    path.split('/').any(|component| {
        matches!(
            component,
            "fixtures" | "testdata" | "snapshots" | "__snapshots__" | "__mocks__"
        )
    })
}

/// True when a component of `path` names a helper directory: the near-universal
/// conventions for code a test calls rather than a test. Weak enough that it
/// only suppresses a directory-layout match; see [`matches_builtin`].
fn in_helper_or_data_tree(path: &str) -> bool {
    in_data_tree(path)
        || path
            .split('/')
            .any(|component| matches!(component, "common" | "helpers" | "support"))
}

fn matches_builtin_name(name: &str, language: Language) -> bool {
    match language {
        Language::Rust => name.ends_with("_test.rs"),
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            name.contains(".test.") || name.contains(".spec.")
        }
        Language::Go => name.ends_with("_test.go"),
        Language::Python => {
            name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
        }
        Language::Java => {
            name.ends_with("Test.java") || name.ends_with("Tests.java") || name.ends_with("IT.java")
        }
        Language::C | Language::Cpp => {
            is_source_of_c_family(name)
                && (name.starts_with("test_") || has_stem_suffix(name, "_test"))
        }
        Language::Zig => name.starts_with("test_") && name.ends_with(".zig"),
    }
}

/// Java's `src/test/**` build layout, at any depth: Maven puts it at the root
/// and Gradle inside each module.
fn in_source_test_tree(path: &str) -> bool {
    let components: Vec<&str> = path.split('/').collect();
    components.windows(2).any(|pair| pair == ["src", "test"])
}

/// True for an extension the C-family row spells `c*`: `c`, `cc`, `cpp`,
/// `cxx`, and the rest, but not a header.
fn is_source_of_c_family(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.starts_with('c'))
}

/// True when `name`'s stem before the first dot ends with `suffix`.
fn has_stem_suffix(name: &str, suffix: &str) -> bool {
    name.split_once('.')
        .is_some_and(|(stem, _)| stem.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TiaConfig {
        TiaConfig::default()
    }

    fn test_file(path: &str) -> bool {
        is_test_file(path, &config())
    }

    #[test]
    fn rust_conventions_match_tests_directories_and_test_suffixed_files() {
        assert!(test_file("tests/symbol_key.rs"));
        assert!(test_file("crates/model/tests/index.rs"));
        assert!(test_file("src/billing_test.rs"));
        assert!(!test_file("src/billing.rs"));
    }

    #[test]
    fn java_conventions_match_the_build_layout_and_the_name_forms() {
        assert!(test_file("src/test/java/com/example/DaoTest.java"));
        assert!(test_file("module/src/test/java/Dao.java"));
        assert!(test_file("src/main/java/com/example/DaoTests.java"));
        assert!(test_file("src/main/java/com/example/DaoIT.java"));
        assert!(!test_file("src/main/java/com/example/Dao.java"));
    }

    #[test]
    fn every_other_language_row_matches_its_own_name_form() {
        assert!(test_file("web/app.test.ts"));
        assert!(test_file("web/app.spec.tsx"));
        assert!(test_file("store/save_test.go"));
        assert!(test_file("pkg/test_store.py"));
        assert!(test_file("pkg/store_test.py"));
        assert!(test_file("native/test_store.c"));
        assert!(test_file("native/store_test.cpp"));
        assert!(test_file("src/test_store.zig"));
        assert!(!test_file("web/app.ts"));
        assert!(!test_file("store/save.go"));
        assert!(!test_file("pkg/store.py"));
        assert!(!test_file("native/store.c"));
        assert!(!test_file("src/store.zig"));
    }

    #[test]
    fn a_directory_layout_follows_the_language_that_compiles_it() {
        // The JavaScript ecosystem's `__tests__/` directory.
        assert!(test_file("web/__tests__/render.ts"));
        // Rust's integration targets: a top-level file and a directory main.
        assert!(test_file("tests/symbol_key.rs"));
        assert!(test_file("tests/backtest/main.rs"));
        // No other language is selected by directory placement alone: a Go
        // file and a Python helper that are not named as tests are not tests.
        assert!(!test_file("service/tests/handler.go"));
        assert!(!test_file("app/test/helpers.py"));
    }

    #[test]
    fn a_data_tree_outranks_even_a_test_shaped_file_name() {
        // Fixtures are data a test reads, not a target a runner executes; this
        // is the path that forced the tightening.
        assert!(!test_file("crates/leadline/tests/fixtures/c/store_test.c"));
        assert!(!test_file("testdata/golden_test.go"));
        assert!(!test_file("web/__snapshots__/app.test.tsx"));
        assert!(!test_file("src/__mocks__/client.test.ts"));
        // A real test beside them still counts.
        assert!(test_file("crates/leadline/tests/runner.rs"));
    }

    #[test]
    fn a_helper_tree_outranks_a_directory_layout_only() {
        // `tests/common/mod.rs` is not a cargo target, so the Rust layout rule
        // already refuses it; the helper exclusion earns its keep on
        // `tests/<name>/main.rs`, which *is* one.
        assert!(!test_file("crates/leadline/tests/common/mod.rs"));
        assert!(!test_file("crates/leadline/tests/support/main.rs"));
        assert!(test_file("crates/leadline/tests/cases/main.rs"));
        // Java's `src/test/**` layout claims every class in the test source
        // tree, so a helper package there needs the exclusion.
        assert!(!test_file("src/test/java/com/example/helpers/Fixture.java"));
        assert!(test_file("src/test/java/com/example/Dao.java"));
    }

    #[test]
    fn a_test_name_is_a_test_wherever_a_helper_directory_sits() {
        // The name declares the file a test unambiguously, so the helper
        // exclusion must not suppress it: rejecting these escalated a change
        // under `src/common/` or `src/support/` to every test under `src/`.
        assert!(test_file("src/common/app.test.ts"));
        assert!(test_file("src/support/db.test.ts"));
        assert!(test_file("service/common/db_test.go"));
        assert!(test_file("pkg/support/test_store.py"));
    }

    #[test]
    fn declared_tests_are_recognized_only_where_the_path_cannot_say() {
        assert!(declares_tests(
            b"#[cfg(test)]\nmod tests {\n    #[test]\n    fn it_works() {}\n}\n",
            Language::Rust
        ));
        assert!(!declares_tests(b"fn charge() {}\n", Language::Rust));
        assert!(declares_tests(
            b"test \"adds\" {\n    try std.testing.expect(true);\n}\n",
            Language::Zig
        ));
        assert!(!declares_tests(
            b"pub fn add(a: u32) u32 { return a; }\n",
            Language::Zig
        ));
        // No other grammar has a content row.
        assert!(!declares_tests(
            b"test('it works', () => {});",
            Language::TypeScript
        ));
    }

    #[test]
    fn discovery_rejects_unsupported_extensions_and_ignored_directories() {
        assert!(!test_file("tests/notes.txt"));
        assert!(!test_file("tests/data.json"));
        assert!(!test_file("node_modules/app.test.ts"));
        assert!(!test_file("vendor/app.test.ts"));
        assert!(!test_file(""));
    }

    #[test]
    fn configured_test_globs_are_additive() {
        let configured = TiaConfig {
            test_globs: vec!["**/*.check.ts".to_owned()],
            ..config()
        };
        assert!(is_test_file("web/flow.check.ts", &configured));
        // Additive, never a replacement.
        assert!(is_test_file("web/flow.test.ts", &configured));
        assert!(!is_test_file("web/flow.ts", &configured));
        assert!(!is_test_file("web/flow.check.ts", &config()));
    }

    #[test]
    fn configured_excludes_remove_candidates() {
        let configured = TiaConfig {
            exclude: vec!["vendor/**".to_owned()],
            ..config()
        };
        assert!(!is_test_file("vendor/dao_test.go", &configured));
        assert!(is_test_file("service/dao_test.go", &configured));
    }
}
