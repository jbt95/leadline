//! Git history integration tests.
//!
//! Every test builds a synthetic repository with deterministic commit dates so
//! that the recency windows are stable. Windows are relative to the HEAD
//! commit time, never to the wall clock.

use leadline::history::analyze_history;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn git(root: &Path, args: &[&str]) {
    git_dated(root, args, None);
}

/// Runs git with fixed author and committer dates when `date` is provided.
fn git_dated(root: &Path, args: &[&str], date: Option<&str>) {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    if let Some(date) = date {
        command
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit(root: &Path, message: &str, date: &str) {
    git(root, &["add", "-A"]);
    git_dated(root, &["commit", "-q", "-m", message], Some(date));
}

fn init(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "a@example.invalid"]);
    git(root, &["config", "user.name", "Author A"]);
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-history-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn head_timestamp(root: &Path) -> i64 {
    let output = Command::new("git")
        .current_dir(root)
        .args(["log", "-1", "--format=%ct"])
        .output()
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn churn_windows_contributors_and_age_are_normalized() {
    let root = temporary_directory();
    init(&root);
    write(&root, "src/a.ts", "export const a = 1;\n");
    write(&root, "src/b.ts", "export const b = 1;\n");
    commit(&root, "one", "2025-01-02T12:00:00Z");

    git(&root, &["config", "user.name", "Author B"]);
    git(&root, &["config", "user.email", "b@example.invalid"]);
    write(
        &root,
        "src/a.ts",
        "export const a = 2;\nexport const a2 = 2;\n",
    );
    commit(&root, "two", "2026-01-10T12:00:00Z");

    git(&root, &["config", "user.name", "Author A"]);
    git(&root, &["config", "user.email", "a@example.invalid"]);
    write(
        &root,
        "src/a.ts",
        "export const a = 2;\nexport const a2 = 2;\nexport const a3 = 3;\n",
    );
    commit(&root, "three", "2026-03-20T12:00:00Z");

    write(
        &root,
        "src/b.ts",
        "export const b = 1;\nexport const b2 = 2;\n",
    );
    commit(&root, "four", "2026-03-31T12:00:00Z");

    let report = analyze_history(&root).unwrap();
    assert!(report.available);
    assert_eq!(report.head_timestamp, Some(head_timestamp(&root)));
    assert!(report.head_commit.is_some());
    assert_eq!(report.reference, "head-commit-time");

    let a = report
        .files
        .iter()
        .find(|file| file.path == "src/a.ts")
        .unwrap();
    assert_eq!(a.commits, 3);
    assert_eq!(a.changes_30d, 1);
    assert_eq!(a.changes_90d, 2);
    assert_eq!(a.changes_365d, 2);
    assert_eq!(a.days_since_last_change, 11);
    assert_eq!(a.age_days, 453);
    assert_eq!(a.contributors, 2);
    assert_eq!(a.recent_contributors, 2);
    assert_eq!(a.lines_added, 4);
    assert_eq!(a.lines_deleted, 1);

    let b = report
        .files
        .iter()
        .find(|file| file.path == "src/b.ts")
        .unwrap();
    assert_eq!(b.commits, 2);
    assert_eq!(b.changes_30d, 1);
    assert_eq!(b.changes_365d, 1);
    assert_eq!(b.days_since_last_change, 0);
    assert_eq!(b.contributors, 1);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rename_carries_history_to_the_new_path() {
    let root = temporary_directory();
    init(&root);
    write(&root, "src/old.ts", "export const a = 1;\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");

    git(&root, &["mv", "src/old.ts", "src/new.ts"]);
    write(
        &root,
        "src/new.ts",
        "export const a = 1;\nexport const a2 = 2;\n",
    );
    commit(&root, "two", "2026-03-02T12:00:00Z");

    write(
        &root,
        "src/new.ts",
        "export const a = 1;\nexport const a2 = 2;\nexport const a3 = 3;\n",
    );
    commit(&root, "three", "2026-03-03T12:00:00Z");

    let report = analyze_history(&root).unwrap();
    let paths: Vec<&str> = report.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["src/new.ts"]);
    let new = &report.files[0];
    assert_eq!(new.commits, 3);
    assert_eq!(new.changes_30d, 3);
    assert_eq!(new.lines_added, 3);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn merge_commits_are_not_counted_twice() {
    let root = temporary_directory();
    init(&root);
    write(&root, "a.ts", "export const a = 1;\n");
    write(&root, "b.ts", "export const b = 1;\n");
    commit(&root, "base", "2026-03-01T12:00:00Z");

    git(&root, &["checkout", "-q", "-b", "side"]);
    write(&root, "b.ts", "export const b = 1;\nexport const b2 = 2;\n");
    commit(&root, "side", "2026-03-02T12:00:00Z");

    git(&root, &["checkout", "-q", "-"]);
    write(&root, "a.ts", "export const a = 1;\nexport const a2 = 2;\n");
    commit(&root, "main", "2026-03-03T12:00:00Z");

    git_dated(
        &root,
        &["merge", "-q", "--no-edit", "--no-ff", "side"],
        Some("2026-03-04T12:00:00Z"),
    );

    let report = analyze_history(&root).unwrap();
    assert_eq!(
        report.head_timestamp,
        Some(head_timestamp(&root)),
        "HEAD is the merge commit; its timestamp is the reference"
    );
    let a = report
        .files
        .iter()
        .find(|file| file.path == "a.ts")
        .unwrap();
    let b = report
        .files
        .iter()
        .find(|file| file.path == "b.ts")
        .unwrap();
    assert_eq!(a.commits, 2);
    assert_eq!(b.commits, 2);
    assert_eq!(a.changes_30d, 2);
    assert_eq!(b.changes_30d, 2);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn history_is_scoped_to_the_requested_directory() {
    let root = temporary_directory();
    init(&root);
    write(&root, "top.ts", "export const top = 1;\n");
    write(&root, "sub/a.ts", "export const a = 1;\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");

    let report = analyze_history(&root.join("sub")).unwrap();
    assert!(report.available);
    let paths: Vec<&str> = report.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["a.ts"]);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshots_without_git_degrade_gracefully() {
    let root = temporary_directory();
    write(&root, "src/a.ts", "export const a = 1;\n");

    let report = analyze_history(&root).unwrap();
    assert!(!report.available);
    assert!(report.files.is_empty());
    assert_eq!(report.head_commit, None);
    assert_eq!(report.head_timestamp, None);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cross_scope_rename_is_reported_as_an_add() {
    let root = temporary_directory();
    init(&root);
    write(&root, "top.ts", "export const top = 1;\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");

    std::fs::create_dir_all(root.join("sub")).unwrap();
    git(&root, &["mv", "top.ts", "sub/moved.ts"]);
    write(
        &root,
        "sub/moved.ts",
        "export const top = 1;\nexport const moved = 2;\n",
    );
    commit(&root, "two", "2026-03-02T12:00:00Z");

    write(
        &root,
        "sub/moved.ts",
        "export const top = 1;\nexport const moved = 2;\nexport const more = 3;\n",
    );
    commit(&root, "three", "2026-03-03T12:00:00Z");

    let report = analyze_history(&root.join("sub")).unwrap();
    let paths: Vec<&str> = report.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["moved.ts"]);
    // The file's life before it entered the scope stays out of scope.
    assert_eq!(report.files[0].commits, 2);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rename_survives_skewed_commit_dates() {
    let root = temporary_directory();
    init(&root);
    write(&root, "src/old.ts", "export const a = 1;\n");
    // The parent commit carries the later date; the rename child is older.
    // Reverse-chronological order alone would emit the parent first and lose
    // the rename, so `--date-order` is what keeps this test green.
    commit(&root, "one", "2026-03-10T12:00:00Z");

    git(&root, &["mv", "src/old.ts", "src/new.ts"]);
    write(
        &root,
        "src/new.ts",
        "export const a = 1;\nexport const a2 = 2;\n",
    );
    commit(&root, "two", "2026-03-01T12:00:00Z");

    let report = analyze_history(&root).unwrap();
    let paths: Vec<&str> = report.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["src/new.ts"]);
    assert_eq!(report.files[0].commits, 2);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn repository_without_commits_is_unavailable() {
    let root = temporary_directory();
    init(&root);
    write(&root, "src/a.ts", "export const a = 1;\n");

    let report = analyze_history(&root).unwrap();
    assert!(!report.available);
    assert!(report.files.is_empty());
    assert_eq!(report.head_timestamp, None);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn repeated_analysis_is_deterministic() {
    let root = temporary_directory();
    init(&root);
    write(&root, "src/a.ts", "export const a = 1;\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");
    write(
        &root,
        "src/a.ts",
        "export const a = 1;\nexport const a2 = 2;\n",
    );
    commit(&root, "two", "2026-03-02T12:00:00Z");

    let first = analyze_history(&root).unwrap();
    let second = analyze_history(&root).unwrap();
    assert_eq!(first, second);

    std::fs::remove_dir_all(root).unwrap();
}
