//! Test-file discovery by convention.
//!
//! Two predicates, and the split between them is deliberate:
//!
//! - [`is_test_file`] reads a path only and never touches the filesystem. It
//!   answers every path-shaped row of the design's convention table: a
//!   directory component named `test`, `tests`, or `__tests__`, and the
//!   per-language file-name forms (`*_test.go`, `*.test.ts`, `*Tests.java`,
//!   `test_*.py`, ...).
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
//! The directory rows are read as "a path component named `tests`/`test`/
//! `__tests__`" at any depth rather than as a root-anchored directory: a Cargo
//! workspace keeps its integration tests in `<crate>/tests/`, and the
//! alternative misses every one of them. Matching a file that is not a test
//! file over-selects; missing one under-selects, so the generous reading is the
//! safe one.
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

/// The built-in conventions: the name form of the path's language, the Java
/// build-layout pair, plus a directory component any language's table spells
/// as `tests/**`, `test/**`, or `**/__tests__/**`.
fn matches_builtin(path: &str) -> bool {
    let Some(language) = detect_language(path) else {
        return false;
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    matches_builtin_name(name, language)
        || (language == Language::Java && in_source_test_tree(path))
        || path
            .split('/')
            .any(|component| matches!(component, "tests" | "test" | "__tests__"))
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
    fn a_test_directory_component_counts_for_every_language() {
        assert!(test_file("web/__tests__/render.ts"));
        assert!(test_file("app/test/helpers.py"));
        assert!(test_file("service/tests/handler.go"));
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
