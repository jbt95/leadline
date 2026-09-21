//! Integration tests for `leadline unused`.
//!
//! Every fixture is a throwaway repository under the system temp dir, built
//! from the plan's fixture: an entry file, a reachable file with a used and a
//! dead export, an unreachable file, two files forming an unreachable cycle, a
//! test file, and one used plus one unused dependency.

use leadline::config::UnusedConfig;
use leadline::unused::{UNUSED_SCHEMA_VERSION, UnusedReport, analyze_unused};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-unused-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// Writes `source` to `root/relative`, creating parent directories.
fn write(root: &Path, relative: &str, source: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

/// The plan's fixture repository.
fn fixture() -> PathBuf {
    let root = temporary_directory();
    write(
        &root,
        "package.json",
        r#"{
  "name": "unused-fixture",
  "main": "./src/index.ts",
  "dependencies": {
    "left-pad": "^1.0.0",
    "unused-pkg": "^1.0.0"
  }
}
"#,
    );
    write(
        &root,
        "src/index.ts",
        "import { used } from \"./used\";\n\nexport function entry(): number {\n  return used();\n}\n",
    );
    write(
        &root,
        "src/used.ts",
        "import leftPad from \"left-pad\";\n\nexport function used(): string {\n  return leftPad(\"x\");\n}\n\nexport function orphan(): number {\n  return 1;\n}\n",
    );
    write(
        &root,
        "src/dead.ts",
        "export function dead(): number {\n  return 1;\n}\n",
    );
    write(
        &root,
        "src/cycle-a.ts",
        "import { b } from \"./cycle-b\";\n\nexport function a(): number {\n  return b();\n}\n",
    );
    write(
        &root,
        "src/cycle-b.ts",
        "import { a } from \"./cycle-a\";\n\nexport function b(): number {\n  return a();\n}\n",
    );
    write(
        &root,
        "src/used.test.ts",
        "export function testOnly(): number {\n  return 1;\n}\n",
    );
    root
}

fn unused_file_paths(report: &UnusedReport) -> Vec<&str> {
    report
        .unused_files
        .iter()
        .map(|file| file.path.as_str())
        .collect()
}

fn entry_rows(report: &UnusedReport) -> Vec<(&str, &str)> {
    report
        .entry_points
        .iter()
        .map(|entry| (entry.path.as_str(), entry.source))
        .collect()
}

fn export_rows(report: &UnusedReport) -> Vec<(&str, &str, u32, &str)> {
    report
        .unused_exports
        .iter()
        .map(|export| {
            (
                export.path.as_str(),
                export.name.as_str(),
                export.line,
                export.kind,
            )
        })
        .collect()
}

fn dependency_rows(report: &UnusedReport) -> Vec<(&str, &str, &str)> {
    report
        .unused_dependencies
        .iter()
        .map(|dependency| {
            (
                dependency.manifest.as_str(),
                dependency.package.as_str(),
                dependency.kind,
            )
        })
        .collect()
}

#[test]
fn fixture_reports_exact_lists_and_identical_bytes_across_runs() {
    let root = fixture();
    let report = analyze_unused(&root, &[], &[], &UnusedConfig::default()).unwrap();

    assert_eq!(report.schema_version, UNUSED_SCHEMA_VERSION);
    assert_eq!(report.metric_profile, leadline::core::METRIC_PROFILE);
    assert_eq!(report.analyzer_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(report.files_analyzed, 6);
    assert_eq!(report.unresolved, 0);
    assert!(report.complete, "{:?}", report.reason);
    assert_eq!(report.reason, None);
    assert_eq!(report.export_star_files, Vec::<String>::new());

    // `src/index.ts` is the manifest's `main` and the `src/index.*`
    // convention, so it is an entry point twice over.
    assert_eq!(
        entry_rows(&report),
        [
            ("src/index.ts", "convention"),
            ("src/index.ts", "package.json")
        ]
    );
    assert_eq!(
        unused_file_paths(&report),
        ["src/cycle-a.ts", "src/cycle-b.ts", "src/dead.ts"]
    );
    assert_eq!(report.excluded_test_files, 1);
    // `orphan` is not imported from `src/used.ts`; the cycle's exports import
    // each other, and the entry file's exports are its public surface.
    // `src/dead.ts` carries its own unused-file row, so its export is not
    // repeated beside it.
    assert_eq!(
        export_rows(&report),
        [("src/used.ts", "orphan", 7, "named")]
    );
    assert_eq!(
        dependency_rows(&report),
        [("package.json", "unused-pkg", "dependencies")]
    );

    let again = analyze_unused(&root, &[], &[], &UnusedConfig::default()).unwrap();
    let first = serde_json::to_string(&report).unwrap();
    let second = serde_json::to_string(&again).unwrap();
    assert_eq!(first, second, "two runs must produce identical bytes");
}

#[test]
fn include_tests_reports_test_files_as_ordinary_candidates() {
    let root = fixture();
    let config = UnusedConfig {
        entries: Vec::new(),
        include_tests: true,
    };
    let report = analyze_unused(&root, &[], &[], &config).unwrap();

    assert_eq!(report.excluded_test_files, 0);
    assert_eq!(
        unused_file_paths(&report),
        [
            "src/cycle-a.ts",
            "src/cycle-b.ts",
            "src/dead.ts",
            "src/used.test.ts"
        ]
    );
    // The file itself is the finding; its exports are not repeated beside it.
    assert!(
        report
            .unused_exports
            .iter()
            .all(|export| export.path != "src/used.test.ts"),
        "{:?}",
        report.unused_exports
    );
}

#[test]
fn entry_patterns_and_config_entries_join_the_entry_set() {
    let root = fixture();
    let config = UnusedConfig {
        entries: vec!["src/cycle-a.ts".to_owned()],
        include_tests: false,
    };
    let report = analyze_unused(&root, &[], &["src/dead.ts".to_owned()], &config).unwrap();

    assert_eq!(
        entry_rows(&report),
        [
            ("src/cycle-a.ts", "config"),
            ("src/dead.ts", "entry"),
            ("src/index.ts", "convention"),
            ("src/index.ts", "package.json"),
        ]
    );
    // `src/cycle-b.ts` is reachable from the cycle's entry point, and the
    // test file stays out of the candidate set.
    assert_eq!(unused_file_paths(&report), Vec::<&str>::new());
    assert_eq!(report.excluded_test_files, 1);
    // Entry points are exempt, so only the reachable file's dead export is
    // left.
    assert_eq!(
        export_rows(&report),
        [("src/used.ts", "orphan", 7, "named")]
    );
}

#[test]
fn unresolved_references_make_the_report_incomplete() {
    let root = temporary_directory();
    write(
        &root,
        "src/index.ts",
        "import { missing } from \"./missing\";\n\nexport function entry(): number {\n  return missing();\n}\n",
    );
    write(
        &root,
        "src/spare.ts",
        "export function spare(): number {\n  return 1;\n}\n",
    );
    let report = analyze_unused(&root, &[], &[], &UnusedConfig::default()).unwrap();

    assert_eq!(report.unresolved, 1);
    assert!(!report.complete);
    assert_eq!(report.reason, Some("unresolved_references"));
    // The unused lists are still printed, but they must not be trusted.
    assert_eq!(unused_file_paths(&report), ["src/spare.ts"]);
}

#[test]
fn empty_entry_set_makes_the_report_incomplete() {
    let root = temporary_directory();
    write(
        &root,
        "src/spare.ts",
        "export function spare(): number {\n  return 1;\n}\n",
    );
    let report = analyze_unused(&root, &[], &[], &UnusedConfig::default()).unwrap();

    assert_eq!(report.files_analyzed, 1);
    assert!(report.entry_points.is_empty());
    assert!(!report.complete);
    assert_eq!(report.reason, Some("no_entry_points"));
    // Reachability from nothing means nothing, so no file is reported unused.
    assert_eq!(unused_file_paths(&report), Vec::<&str>::new());
}

#[test]
fn star_reexport_exempts_the_reexported_file_only() {
    let root = temporary_directory();
    write(&root, "src/index.ts", "export * from \"./lib\";\n");
    write(
        &root,
        "src/lib.ts",
        "export function helper(): number {\n  return 1;\n}\n",
    );
    write(
        &root,
        "src/other.ts",
        "export function lonely(): number {\n  return 1;\n}\n",
    );
    let report = analyze_unused(&root, &[], &[], &UnusedConfig::default()).unwrap();

    assert_eq!(report.export_star_files, ["src/index.ts"]);
    // `src/lib.ts` is re-exported wholesale and reachable through the star;
    // `src/other.ts` is neither.
    assert_eq!(unused_file_paths(&report), ["src/other.ts"]);
    // `src/other.ts` is unreachable, so its export is covered by that row.
    assert!(
        report.unused_exports.is_empty(),
        "{:?}",
        report.unused_exports
    );
}

#[test]
fn nested_manifest_declares_its_own_entry_points_and_dependencies() {
    let root = temporary_directory();
    write(
        &root,
        "package.json",
        r#"{
  "main": "./src/index.ts",
  "dependencies": {
    "root-unused": "^1.0.0",
    "web-used": "^1.0.0"
  }
}
"#,
    );
    write(
        &root,
        "src/index.ts",
        "export function entry(): number {\n  return 1;\n}\n",
    );
    write(
        &root,
        "packages/web/package.json",
        r#"{
  "main": "./src/page.ts",
  "dependencies": {
    "web-used": "^1.0.0",
    "web-unused": "^1.0.0"
  }
}
"#,
    );
    write(
        &root,
        "packages/web/src/page.ts",
        "import \"web-used\";\n\nexport function page(): number {\n  return 1;\n}\n",
    );
    write(
        &root,
        "packages/web/src/helper.ts",
        "export function helper(): number {\n  return 1;\n}\n",
    );
    let report = analyze_unused(&root, &[], &[], &UnusedConfig::default()).unwrap();

    // The nested manifest's `main` is an entry point, so its own subtree is
    // reachable: only the file nothing imports is reported, and the entry's
    // exports stay exempt.
    assert_eq!(
        entry_rows(&report),
        [
            ("packages/web/src/page.ts", "package.json"),
            ("src/index.ts", "convention"),
            ("src/index.ts", "package.json"),
        ]
    );
    assert_eq!(unused_file_paths(&report), ["packages/web/src/helper.ts"]);
    // The unreachable file's own row covers its export.
    assert!(
        report.unused_exports.is_empty(),
        "{:?}",
        report.unused_exports
    );
    // `web-used` is imported under `packages/web` only, so the root manifest
    // does not see that import: attribution follows the nearest manifest.
    assert_eq!(
        dependency_rows(&report),
        [
            ("package.json", "root-unused", "dependencies"),
            ("package.json", "web-used", "dependencies"),
            ("packages/web/package.json", "web-unused", "dependencies"),
        ]
    );
}

#[test]
fn package_directory_conventions_cover_index_and_config_files() {
    let root = temporary_directory();
    write(
        &root,
        "package.json",
        "{\n  \"name\": \"conventions\",\n  \"main\": \"./src/index.ts\"\n}\n",
    );
    write(
        &root,
        "src/index.ts",
        "export function entry(): number {\n  return 1;\n}\n",
    );
    // A nested package whose manifest names no source field: its index and its
    // tooling config are still loaded by name, not by an import edge.
    write(
        &root,
        "packages/host/package.json",
        "{ \"name\": \"host\" }\n",
    );
    write(
        &root,
        "packages/host/index.ts",
        "export function host(): number {\n  return 1;\n}\n",
    );
    write(
        &root,
        "packages/host/vite.config.ts",
        "export default { plugins: [] };\n",
    );
    write(
        &root,
        "src/dead.ts",
        "export function dead(): number {\n  return 1;\n}\n",
    );

    let report = analyze_unused(&root, &[], &[], &UnusedConfig::default()).unwrap();
    let entries: Vec<&str> = report
        .entry_points
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert!(entries.contains(&"packages/host/index.ts"), "{entries:?}");
    assert!(
        entries.contains(&"packages/host/vite.config.ts"),
        "{entries:?}"
    );
    let unused: Vec<&str> = report
        .unused_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(unused, ["src/dead.ts"], "{unused:?}");
    assert!(report.complete, "{report:?}");
    std::fs::remove_dir_all(&root).unwrap();
}
