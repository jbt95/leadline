use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn changed_analysis_compares_function_versions() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("app.ts"),
        "function choose(x: number) {\n  if (x > 0) return 1;\n  return 0;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "app.ts"]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("app.ts"),
        "function choose(x: number) {\n  if (x > 0) {\n    if (x > 1) return 2;\n    return 1;\n  }\n  return 0;\n}\n",
    )
    .unwrap();

    let report = leadline::diff::analyze_changed(&root, "HEAD").unwrap();
    assert_eq!(report.functions.len(), 1);
    let change = &report.functions[0];
    assert_eq!(change.name, "choose");
    assert_eq!(change.before.as_ref().unwrap().metrics.cyclomatic, 2);
    assert_eq!(change.after.as_ref().unwrap().metrics.cyclomatic, 3);
    assert_eq!(
        leadline::diff::changed_regressions(
            &report,
            &leadline::config::RegressionLimits::default()
        )
        .len(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_analysis_honors_scope_and_reports_parse_errors() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::create_dir_all(root.join("first")).unwrap();
    std::fs::create_dir_all(root.join("second")).unwrap();
    std::fs::write(root.join("first/a.ts"), "function a() { return 1; }\n").unwrap();
    std::fs::write(root.join("second/b.ts"), "function b() { return 1; }\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("first/a.ts"),
        "function a(x: boolean) { if (x) return 1; return 0; }\n",
    )
    .unwrap();
    std::fs::write(root.join("second/b.ts"), "function b( { return 1; }\n").unwrap();

    let scoped = leadline::diff::analyze_changed(&root.join("first"), "HEAD").unwrap();
    assert_eq!(scoped.functions.len(), 1);
    assert_eq!(scoped.functions[0].path, "first/a.ts");
    assert!(scoped.parse_errors.is_empty());

    let complete = leadline::diff::analyze_changed(&root, "HEAD").unwrap();
    assert_eq!(complete.parse_errors.len(), 1);
    assert_eq!(complete.parse_errors[0].path, "second/b.ts");
    assert!(!complete.parse_errors[0].after.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_analysis_rejects_option_like_revisions() {
    let error = leadline::diff::analyze_changed(Path::new("missing"), "--help").unwrap_err();
    assert!(error.to_string().contains("unsupported characters"));
}

#[test]
fn analyze_changed_wraps_worktree_analysis() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(root.join("app.ts"), "function a() { return 1; }\n").unwrap();
    git(&root, &["add", "app.ts"]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("app.ts"),
        "function a() { if (true) return 1; return 0; }\n",
    )
    .unwrap();

    let wrapped = leadline::diff::analyze_changed(&root, "HEAD").unwrap();
    let direct = leadline::diff::analyze_changes(
        &root,
        &leadline::diff::ChangeOptions {
            base: "HEAD".to_owned(),
            target: leadline::diff::ComparisonTarget::Worktree,
            detect_renames: false,
        },
    )
    .unwrap();
    assert_eq!(wrapped, direct);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn index_target_excludes_unstaged_edits() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("app.ts"),
        "function calc(x: number) {\n  return x;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "app.ts"]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("app.ts"),
        "function calc(x: number) {\n  if (x > 0) return x;\n  return 0;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "app.ts"]);
    std::fs::write(
        root.join("app.ts"),
        "function calc(x: number) {\n  if (x > 0) { if (x > 1) return 2; return x; }\n  return 0;\n}\n",
    )
    .unwrap();

    let report = leadline::diff::analyze_changes(
        &root,
        &leadline::diff::ChangeOptions {
            base: "HEAD".to_owned(),
            target: leadline::diff::ComparisonTarget::Index,
            detect_renames: false,
        },
    )
    .unwrap();
    assert_eq!(report.functions.len(), 1);
    assert_eq!(
        report.functions[0]
            .after
            .as_ref()
            .unwrap()
            .metrics
            .cyclomatic,
        2
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn revision_target_ignores_worktree() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("app.ts"),
        "function calc(x: number) {\n  return x;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "app.ts"]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("app.ts"),
        "function calc(x: number) {\n  if (x > 0) return x;\n  return 0;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "app.ts"]);
    git(&root, &["commit", "-qm", "target"]);
    std::fs::write(
        root.join("app.ts"),
        "function calc(x: number) {\n  if (x > 0) { if (x > 1) return 2; return x; }\n  return 0;\n}\n",
    )
    .unwrap();

    let report = leadline::diff::analyze_changes(
        &root,
        &leadline::diff::ChangeOptions {
            base: "HEAD~1".to_owned(),
            target: leadline::diff::ComparisonTarget::Revision("HEAD".to_owned()),
            detect_renames: false,
        },
    )
    .unwrap();
    assert_eq!(report.functions.len(), 1);
    assert_eq!(
        report.functions[0]
            .after
            .as_ref()
            .unwrap()
            .metrics
            .cyclomatic,
        2
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn revision_target_supports_scope_absent_from_worktree() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::create_dir(root.join("archived")).unwrap();
    std::fs::write(
        root.join("archived/app.ts"),
        "function calc(x: number) {\n  return x;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "archived/app.ts"]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("archived/app.ts"),
        "function calc(x: number) {\n  if (x > 0) return x;\n  return 0;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "archived/app.ts"]);
    git(&root, &["commit", "-qm", "target"]);
    std::fs::remove_dir_all(root.join("archived")).unwrap();

    let report = leadline::diff::analyze_changes(
        &root.join("archived"),
        &leadline::diff::ChangeOptions {
            base: "HEAD~1".to_owned(),
            target: leadline::diff::ComparisonTarget::Revision("HEAD".to_owned()),
            detect_renames: false,
        },
    )
    .unwrap();
    assert_eq!(report.functions.len(), 1);
    assert_eq!(report.functions[0].path, "archived/app.ts");
    assert_eq!(
        report.functions[0]
            .after
            .as_ref()
            .unwrap()
            .metrics
            .cyclomatic,
        2
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn option_like_target_revision_is_rejected() {
    let error = leadline::diff::analyze_changes(
        Path::new("missing"),
        &leadline::diff::ChangeOptions {
            base: "HEAD".to_owned(),
            target: leadline::diff::ComparisonTarget::Revision("--evil".to_owned()),
            detect_renames: false,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("unsupported characters"));
}

#[test]
fn renames_pairs_git_detected_file_rename() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("app.ts"),
        "function keep(): number {\n  return 1;\n}\n\nfunction foo(x: number) {\n  return x + 1;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "app.ts"]);
    git(&root, &["commit", "-qm", "base"]);
    git(&root, &["mv", "app.ts", "app2.ts"]);
    std::fs::write(
        root.join("app2.ts"),
        "function keep(): number {\n  return 1;\n}\n\nfunction foo(x: number) {\n  if (x > 0) return x + 1;\n  return x;\n}\n",
    )
    .unwrap();

    let without_renames = leadline::diff::analyze_changes(
        &root,
        &leadline::diff::ChangeOptions {
            base: "HEAD".to_owned(),
            target: leadline::diff::ComparisonTarget::Worktree,
            detect_renames: false,
        },
    )
    .unwrap();
    assert_eq!(without_renames.functions.len(), 4);

    let with_renames = leadline::diff::analyze_changes(
        &root,
        &leadline::diff::ChangeOptions {
            base: "HEAD".to_owned(),
            target: leadline::diff::ComparisonTarget::Worktree,
            detect_renames: true,
        },
    )
    .unwrap();
    assert_eq!(with_renames.functions.len(), 1);
    let change = &with_renames.functions[0];
    assert_eq!(change.name, "foo");
    assert_eq!(change.path, "app2.ts");
    assert!(change.before.is_some());
    assert!(change.after.is_some());
    std::fs::remove_dir_all(root).unwrap();
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-git-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn regression_analysis(name: &str, cognitive: u32) -> leadline::core::FunctionAnalysis {
    let source = format!("function {name}() {{ return 1; }}\n");
    let mut function = leadline::analyze_source("src/a.ts", source.as_bytes())
        .unwrap()
        .functions
        .into_iter()
        .next()
        .unwrap();
    function.metrics.cognitive = cognitive;
    function
}

#[test]
fn regression_predicate_allows_unchanged_and_improved_functions() {
    let limits = leadline::config::RegressionLimits::default();
    let before = regression_analysis("same", 2);
    let unchanged = regression_analysis("same", 2);
    assert!(!leadline::diff::regression_violates(
        &before, &unchanged, &limits
    ));
    let improved = regression_analysis("same", 1);
    assert!(!leadline::diff::regression_violates(
        &before, &improved, &limits
    ));
}

#[test]
fn regression_predicate_rejects_positive_delta_over_limit() {
    let limits = leadline::config::RegressionLimits {
        cognitive: 1,
        ..Default::default()
    };
    let before = regression_analysis("changed", 2);
    let after = regression_analysis("changed", 4);
    assert!(leadline::diff::regression_violates(
        &before, &after, &limits
    ));
    let allowed = regression_analysis("changed", 3);
    assert!(!leadline::diff::regression_violates(
        &before, &allowed, &limits
    ));
}

#[test]
fn added_and_removed_functions_are_not_delta_regressions() {
    let limits = leadline::config::RegressionLimits::default();
    let added = regression_analysis("added", 99);
    let removed = regression_analysis("removed", 99);
    let added_change = leadline::diff::FunctionChange {
        path: "src/a.ts".to_owned(),
        name: "added".to_owned(),
        before: None,
        after: Some(added),
    };
    let removed_change = leadline::diff::FunctionChange {
        path: "src/a.ts".to_owned(),
        name: "removed".to_owned(),
        before: Some(removed),
        after: None,
    };
    let report = leadline::diff::ChangedReport {
        schema_version: 1,
        metric_profile: leadline::core::METRIC_PROFILE,
        analyzer_version: "test",
        metric_specs: leadline::core::MetricSpecs::default(),
        base: "base".to_owned(),
        functions: vec![added_change, removed_change],
        parse_errors: Vec::new(),
    };
    assert!(leadline::diff::changed_regressions(&report, &limits).is_empty());
}

fn init_change_repo() -> PathBuf {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.ts"), "function a() { return 1; }\n").unwrap();
    std::fs::write(root.join("src/empty.ts"), "export const ready = true;\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    root
}

fn worktree_options() -> leadline::diff::ChangeOptions {
    leadline::diff::ChangeOptions {
        base: "HEAD".to_owned(),
        target: leadline::diff::ComparisonTarget::Worktree,
        detect_renames: false,
    }
}

#[test]
fn changed_source_entries_and_paths_include_functionless_files() {
    let root = init_change_repo();
    std::fs::write(root.join("src/a.ts"), "function a() { return 2; }\n").unwrap();
    std::fs::remove_file(root.join("src/empty.ts")).unwrap();
    std::fs::write(root.join("src/c.ts"), "export const fresh = 1;\n").unwrap();
    let options = worktree_options();
    let entries = leadline::diff::changed_source_entries(&root, &options).unwrap();
    let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
    // Modified and untracked files land as after-side entries, even the
    // functionless one; the deleted file has no after-side bytes.
    assert_eq!(paths, vec!["src/a.ts", "src/c.ts"]);
    assert!(entries.iter().all(|entry| !entry.bytes.is_empty()));
    let paths = leadline::diff::changed_paths(&root, &options).unwrap();
    // Deleted paths stay in the set, and non-source files are included:
    // changed attribution is Git state, not analyzable content.
    assert!(paths.contains("src/a.ts"));
    assert!(paths.contains("src/c.ts"));
    assert!(paths.contains("src/empty.ts"));
    std::fs::write(root.join(".env"), "TOKEN=abc\n").unwrap();
    let paths = leadline::diff::changed_paths(&root, &options).unwrap();
    assert!(paths.contains(".env"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_paths_are_relative_to_the_analysis_root() {
    let root = init_change_repo();
    std::fs::write(root.join("src/a.ts"), "function a() { return 2; }\n").unwrap();
    let options = worktree_options();
    // A directory scope strips its own prefix, matching the paths a scanner
    // rooted at that directory reports.
    let paths = leadline::diff::changed_paths(&root.join("src"), &options).unwrap();
    assert!(paths.contains("a.ts"), "{paths:?}");
    assert!(!paths.contains("src/a.ts"), "{paths:?}");
    // A single-file scope strips the file's parent.
    let paths = leadline::diff::changed_paths(&root.join("src/a.ts"), &options).unwrap();
    assert!(paths.contains("a.ts"), "{paths:?}");
    assert!(!paths.contains("src/a.ts"), "{paths:?}");
    // Repo-root scopes keep Git-relative paths unchanged.
    let paths = leadline::diff::changed_paths(&root, &options).unwrap();
    assert!(paths.contains("src/a.ts"), "{paths:?}");
    assert!(!paths.contains("a.ts"), "{paths:?}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_source_entries_follow_renames_to_new_paths() {
    let root = init_change_repo();
    git(&root, &["mv", "src/a.ts", "src/renamed.ts"]);
    let options = leadline::diff::ChangeOptions {
        base: "HEAD".to_owned(),
        target: leadline::diff::ComparisonTarget::Index,
        detect_renames: true,
    };
    let entries = leadline::diff::changed_source_entries(&root, &options).unwrap();
    let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(paths, vec!["src/renamed.ts"]);
    let paths = leadline::diff::changed_paths(&root, &options).unwrap();
    assert!(paths.contains("src/renamed.ts"));
    assert!(!paths.contains("src/a.ts"));
    std::fs::remove_dir_all(root).unwrap();
}
