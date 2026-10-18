use leadline::analytics::{DebtRequest, ProjectRequest, analyze_debt, build, capture_trend};
use leadline::history::HistoryWindow;
use leadline::ownership::OwnershipMode;
use leadline::snapshots::SnapshotOutcome;
use leadline::source_snapshot::SnapshotTarget;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-analytics-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn commit(root: &Path, message: &str) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", message, "--no-gpg-sign"]);
}

fn request(path: &Path, target: SnapshotTarget) -> ProjectRequest {
    ProjectRequest {
        path: path.to_path_buf(),
        target,
        window: HistoryWindow::Days90,
        mutation_inputs: Vec::new(),
        test_maps: Vec::new(),
        ownership_mode: OwnershipMode::AggregateOnly,
        snapshots_path: None,
        coverage: None,
    }
}

#[test]
fn project_builds_from_worktree_and_revision_consistently() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "a@example.invalid"]);
    git(&root, &["config", "user.name", "Author A"]);
    write(
        &root,
        "src/a.ts",
        "export function risky(x: number): number {\n  if (x > 1) { return 1; }\n  return 0;\n}\n",
    );
    commit(&root, "one");
    write(
        &root,
        "src/a.ts",
        "export function risky(x: number): number {\n  if (x > 1) { return 1; }\n  if (x > 2) { return 2; }\n  return 0;\n}\n",
    );
    commit(&root, "two");

    let worktree = build(&request(&root, SnapshotTarget::Worktree)).unwrap();
    assert!(worktree.meta.git_available);
    assert_eq!(worktree.files.len(), 1);
    assert_eq!(worktree.ownership.as_ref().unwrap().files.len(), 1);
    assert!(!worktree.risk.rows.is_empty());
    assert_eq!(worktree.risk.rows[0].path, "src/a.ts");
    assert!(worktree.git_activity.is_some());
    assert!(worktree.temporal_coupling.is_some());

    let revision = build(&request(&root, SnapshotTarget::Revision("HEAD".to_owned()))).unwrap();
    assert_eq!(revision.meta.head_commit, worktree.meta.head_commit);
    assert_eq!(revision.files, worktree.files);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn debt_classifies_new_debt_between_complete_states() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "a@example.invalid"]);
    git(&root, &["config", "user.name", "Author A"]);
    write(
        &root,
        "leadline.toml",
        "[thresholds.function]\ncognitive = 2\n",
    );
    write(
        &root,
        "src/a.ts",
        "export function calm(x: number): number {\n  return x;\n}\n",
    );
    commit(&root, "base");
    write(
        &root,
        "src/a.ts",
        "export function calm(x: number): number {\n  if (x > 1) { return 1; }\n  if (x > 2) { return 2; }\n  if (x > 3) { return 3; }\n  return x;\n}\n",
    );
    commit(&root, "new debt");

    let report = analyze_debt(&DebtRequest {
        path: root.clone(),
        base: "HEAD~1".to_owned(),
        target: SnapshotTarget::Worktree,
        detect_renames: false,
        window: HistoryWindow::Days90,
        fail_on_regression: false,
    })
    .unwrap();
    assert!(report.summary.new >= 1, "{:?}", report.summary);
    assert_eq!(report.findings[0].dimension, "cognitive");
    assert_eq!(report.findings[0].status, leadline::debt::DebtStatus::New);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capture_trend_is_idempotent_for_one_head() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "a@example.invalid"]);
    git(&root, &["config", "user.name", "Author A"]);
    write(&root, "src/a.ts", "export const a = 1;\n");
    commit(&root, "one");

    let store = root.join("trends.json");
    assert_eq!(
        capture_trend(&root, &store, HistoryWindow::Days90, &[], false).unwrap(),
        SnapshotOutcome::Added
    );
    let first = std::fs::read_to_string(&store).unwrap();
    assert_eq!(
        capture_trend(&root, &store, HistoryWindow::Days90, &[], false).unwrap(),
        SnapshotOutcome::Unchanged
    );
    assert_eq!(
        std::fs::read_to_string(&store).unwrap(),
        first,
        "idempotent capture rewrites nothing"
    );

    // Dirty worktree state never enters a HEAD capture.
    write(&root, "src/a.ts", "export const a = 999;\n");
    assert_eq!(
        capture_trend(&root, &store, HistoryWindow::Days90, &[], false).unwrap(),
        SnapshotOutcome::Unchanged
    );

    std::fs::remove_dir_all(root).unwrap();
}
