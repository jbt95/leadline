//! Revision-bounded Git analytics integration tests.
//!
//! Every test builds a synthetic repository with deterministic commit dates so
//! recency windows are stable. One revision-bounded walk must produce history,
//! per-file identity touches, and whole-project coupling from the same commit
//! stream.

use leadline::coupling::{CouplingEdge, CouplingOptions, MAX_COMMIT_FILES, analyze_coupling};
use leadline::history::{
    FileTouches, GitAnalyticsSnapshot, analyze_git_at, analyze_git_at_with_mailmap,
};
use leadline::source_snapshot::{SnapshotContext, SnapshotTarget, load};
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
    identify(root, "Author A", "a@example.invalid");
}

fn identify(root: &Path, name: &str, email: &str) {
    git(root, &["config", "user.name", name]);
    git(root, &["config", "user.email", email]);
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn append(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    existing.push_str(contents);
    std::fs::write(path, existing).unwrap();
}

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "leadline-git-analytics-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn rev_parse(root: &Path, revision: &str) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", revision])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn worktree_context(path: &Path) -> SnapshotContext {
    load(path, SnapshotTarget::Worktree).unwrap().0
}

fn options(min_co_changes: u64, limit: usize) -> CouplingOptions {
    CouplingOptions {
        min_co_changes,
        limit,
    }
}

fn touch_identities<'a>(touches: &'a [FileTouches], path: &str) -> Vec<&'a str> {
    touches
        .iter()
        .find(|file| file.path == path)
        .map(|file| {
            file.identities
                .iter()
                .map(|(identity, _)| identity.as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn edge<'a>(snapshot: &'a GitAnalyticsSnapshot, source: &str, target: &str) -> &'a CouplingEdge {
    snapshot
        .coupling
        .edges
        .iter()
        .find(|edge| edge.source == source && edge.target == target)
        .unwrap()
}

struct AnalyticsRepo {
    path: PathBuf,
    second: String,
    third: String,
}

/// Three commits over two files; the third commit adds a second author.
fn analytics_fixture_three_commits() -> AnalyticsRepo {
    let path = temporary_directory();
    init(&path);
    write(&path, "src/a.ts", "export const a = 1;\n");
    write(&path, "src/b.ts", "export const b = 1;\n");
    commit(&path, "one", "2026-03-01T12:00:00Z");

    write(
        &path,
        "src/a.ts",
        "export const a = 1;\nexport const a2 = 2;\n",
    );
    commit(&path, "two", "2026-03-02T12:00:00Z");
    let second = rev_parse(&path, "HEAD");

    identify(&path, "Author B", "b@example.invalid");
    write(
        &path,
        "src/a.ts",
        "export const a = 1;\nexport const a2 = 2;\nexport const a3 = 3;\n",
    );
    write(
        &path,
        "src/b.ts",
        "export const b = 1;\nexport const b2 = 2;\n",
    );
    commit(&path, "three", "2026-03-03T12:00:00Z");
    let third = rev_parse(&path, "HEAD");

    AnalyticsRepo {
        path,
        second,
        third,
    }
}

struct MailmapRepo {
    path: PathBuf,
    head: String,
    context: SnapshotContext,
    mailmap: Vec<u8>,
}

/// One commit by `raw@example.test`, with a committed `.mailmap` that maps it
/// to `canonical@example.test`.
fn mailmap_fixture() -> MailmapRepo {
    let path = temporary_directory();
    init(&path);
    identify(&path, "Raw Author", "raw@example.test");
    write(
        &path,
        ".mailmap",
        "Canonical Name <canonical@example.test> <raw@example.test>\n",
    );
    write(&path, "src/a.ts", "export const a = 1;\n");
    commit(&path, "one", "2026-03-01T12:00:00Z");
    write(
        &path,
        "src/a.ts",
        "export const a = 1;\nexport const a2 = 2;\n",
    );
    commit(&path, "two", "2026-03-02T12:00:00Z");

    let head = rev_parse(&path, "HEAD");
    let (context, snapshot) = load(&path, SnapshotTarget::Revision("HEAD".to_owned())).unwrap();
    let mailmap = snapshot
        .mailmap_bytes
        .expect(".mailmap is committed at HEAD");
    MailmapRepo {
        path,
        head,
        context,
        mailmap,
    }
}

#[test]
fn revision_snapshot_stops_history_at_requested_commit() {
    let repo = analytics_fixture_three_commits();
    let context = worktree_context(&repo.path);

    let at_two = analyze_git_at(&context, &repo.second).unwrap();
    assert_eq!(
        at_two.history.head_commit.as_deref(),
        Some(repo.second.as_str())
    );
    assert_eq!(at_two.history.file("src/a.ts").unwrap().commits, 2);
    assert_eq!(at_two.history.file("src/b.ts").unwrap().commits, 1);
    assert_eq!(
        at_two.touches,
        vec![
            FileTouches {
                path: "src/a.ts".to_owned(),
                identities: vec![("a@example.invalid".to_owned(), 2)],
            },
            FileTouches {
                path: "src/b.ts".to_owned(),
                identities: vec![("a@example.invalid".to_owned(), 1)],
            },
        ],
        "touches stop at the requested commit too"
    );
    assert_eq!(at_two.coupling.edges.len(), 1);
    assert_eq!(at_two.coupling.edges[0].co_changes, 1);

    let at_three = analyze_git_at(&context, &repo.third).unwrap();
    assert_eq!(
        at_three.history.head_commit.as_deref(),
        Some(repo.third.as_str())
    );
    assert_eq!(at_three.history.file("src/a.ts").unwrap().commits, 3);
    assert_eq!(at_three.history.file("src/b.ts").unwrap().commits, 2);
    assert_eq!(at_three.coupling.edges[0].co_changes, 2);
    assert_ne!(at_two, at_three);

    std::fs::remove_dir_all(repo.path).unwrap();
}

#[test]
fn mailmap_is_target_blob_not_dirty_worktree_state() {
    let repo = mailmap_fixture();
    let before =
        analyze_git_at_with_mailmap(&repo.context, &repo.head, Some(&repo.mailmap)).unwrap();
    assert_eq!(
        touch_identities(&before.touches, "src/a.ts"),
        ["canonical@example.test"]
    );

    std::fs::write(
        repo.path.join(".mailmap"),
        "Different <different@example.test>\n",
    )
    .unwrap();
    let after =
        analyze_git_at_with_mailmap(&repo.context, &repo.head, Some(&repo.mailmap)).unwrap();
    assert_eq!(before.touches, after.touches);
    assert_eq!(before.history, after.history);

    // The parameterless entry point never consults the worktree mailmap.
    let dirty_context = worktree_context(&repo.path);
    let bare = analyze_git_at(&dirty_context, &repo.head).unwrap();
    assert_eq!(
        touch_identities(&bare.touches, "src/a.ts"),
        ["raw@example.test"]
    );

    std::fs::remove_dir_all(repo.path).unwrap();
}

#[test]
fn raw_author_fields_are_used_and_ambient_mailmap_config_is_ignored() {
    let repo = mailmap_fixture();
    let ambient = repo.path.join("ambient.mailmap");
    std::fs::write(
        &ambient,
        "Ambient <ambient@example.test> <raw@example.test>\n",
    )
    .unwrap();
    git(
        &repo.path,
        &["config", "mailmap.file", ambient.to_str().unwrap()],
    );

    let raw = analyze_git_at(&repo.context, &repo.head).unwrap();
    assert_eq!(
        touch_identities(&raw.touches, "src/a.ts"),
        ["raw@example.test"],
        "the walk requests raw %an/%ae, not mailmap-expanded fields"
    );
    assert_eq!(raw.history.file("src/a.ts").unwrap().contributors, 1);

    let mapped =
        analyze_git_at_with_mailmap(&repo.context, &repo.head, Some(&repo.mailmap)).unwrap();
    assert_eq!(
        touch_identities(&mapped.touches, "src/a.ts"),
        ["canonical@example.test"]
    );

    std::fs::remove_dir_all(repo.path).unwrap();
}

#[test]
fn one_touch_per_file_and_commit_by_identity() {
    let root = temporary_directory();
    init(&root);
    write(&root, "src/a.ts", "export const a = 1;\n");
    write(&root, "src/b.ts", "export const b = 1;\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");

    write(
        &root,
        "src/a.ts",
        "export const a = 1;\nexport const a2 = 2;\n",
    );
    commit(&root, "two", "2026-03-02T12:00:00Z");

    identify(&root, "Author B", "b@example.invalid");
    write(
        &root,
        "src/a.ts",
        "export const a = 1;\nexport const a2 = 2;\nexport const a3 = 3;\n",
    );
    write(
        &root,
        "src/b.ts",
        "export const b = 1;\nexport const b2 = 2;\n",
    );
    commit(&root, "three", "2026-03-03T12:00:00Z");

    let context = worktree_context(&root);
    let snapshot = analyze_git_at(&context, "HEAD").unwrap();
    assert_eq!(
        snapshot.touches,
        vec![
            FileTouches {
                path: "src/a.ts".to_owned(),
                identities: vec![
                    ("a@example.invalid".to_owned(), 2),
                    ("b@example.invalid".to_owned(), 1),
                ],
            },
            FileTouches {
                path: "src/b.ts".to_owned(),
                identities: vec![
                    ("a@example.invalid".to_owned(), 1),
                    ("b@example.invalid".to_owned(), 1),
                ],
            },
        ],
        "one commit touching two files adds one touch to each, per identity"
    );
    let a = snapshot.history.file("src/a.ts").unwrap();
    assert_eq!(a.commits, 3);
    assert_eq!(a.contributors, 2);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn coupling_edges_match_target_coupling_formulas() {
    let root = temporary_directory();
    init(&root);
    write(&root, "src/a.ts", "a0\n");
    write(&root, "src/b.ts", "b0\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");

    append(&root, "src/a.ts", "a1\n");
    append(&root, "src/b.ts", "b1\n");
    commit(&root, "two", "2026-03-02T12:00:00Z");

    append(&root, "src/a.ts", "a2\n");
    append(&root, "src/b.ts", "b2\n");
    write(&root, "src/c.ts", "c0\n");
    commit(&root, "three", "2026-03-03T12:00:00Z");

    append(&root, "src/b.ts", "b3\n");
    append(&root, "src/c.ts", "c1\n");
    commit(&root, "four", "2026-03-04T12:00:00Z");

    let context = worktree_context(&root);
    let snapshot = analyze_git_at(&context, "HEAD").unwrap();
    let pairs: Vec<(&str, &str)> = snapshot
        .coupling
        .edges
        .iter()
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("src/a.ts", "src/b.ts"),
            ("src/a.ts", "src/c.ts"),
            ("src/b.ts", "src/c.ts"),
        ],
        "undirected pairs are sorted by (source, target)"
    );

    let ab = edge(&snapshot, "src/a.ts", "src/b.ts");
    assert_eq!(ab.co_changes, 3);
    assert!((ab.directional - 1.0).abs() < 1e-9, "a: 3 of 3 commits");
    assert!((ab.reverse_directional - 0.75).abs() < 1e-9, "b: 3 of 4");
    assert!((ab.jaccard - 0.75).abs() < 1e-9);

    for target in ["src/a.ts", "src/b.ts", "src/c.ts"] {
        let report = analyze_coupling(&root, target, &options(1, 10)).unwrap();
        assert!(report.git_available);
        for related in &report.related {
            let (source, pair_target) = if target < related.path.as_str() {
                (target, related.path.as_str())
            } else {
                (related.path.as_str(), target)
            };
            let edge = edge(&snapshot, source, pair_target);
            let (directional, reverse) = if edge.source == target {
                (edge.directional, edge.reverse_directional)
            } else {
                (edge.reverse_directional, edge.directional)
            };
            assert!(
                (related.directional - directional).abs() < 1e-9,
                "{target} -> {}",
                related.path
            );
            assert!(
                (related.reverse_directional - reverse).abs() < 1e-9,
                "{} -> {target}",
                related.path
            );
            assert!((related.jaccard - edge.jaccard).abs() < 1e-9);
        }
    }

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn wide_commits_count_but_do_not_pair() {
    let root = temporary_directory();
    init(&root);
    let wide: Vec<String> = (0..MAX_COMMIT_FILES)
        .map(|index| format!("src/wide{index:03}.ts"))
        .collect();
    let mut files: Vec<&str> = vec!["src/a.ts"];
    files.extend(wide.iter().map(String::as_str));
    assert_eq!(files.len(), MAX_COMMIT_FILES + 1);
    for file in &files {
        write(&root, file, "export const wide = 1;\n");
    }
    commit(&root, "wide", "2026-03-01T12:00:00Z");
    append(&root, "src/a.ts", "export const a1 = 1;\n");
    write(&root, "src/b.ts", "export const b = 1;\n");
    commit(&root, "pair", "2026-03-02T12:00:00Z");

    let context = worktree_context(&root);
    let snapshot = analyze_git_at(&context, "HEAD").unwrap();
    assert_eq!(snapshot.coupling.edges.len(), 1);
    let edge = &snapshot.coupling.edges[0];
    assert_eq!(
        (edge.source.as_str(), edge.target.as_str()),
        ("src/a.ts", "src/b.ts")
    );
    assert_eq!(edge.co_changes, 1);
    assert!((edge.directional - 0.5).abs() < 1e-9, "a: 1 of 2 commits");
    assert!((edge.reverse_directional - 1.0).abs() < 1e-9);

    let a = snapshot
        .touches
        .iter()
        .find(|touches| touches.path == "src/a.ts")
        .unwrap();
    assert_eq!(a.identities, [("a@example.invalid".to_owned(), 2)]);

    let report = analyze_coupling(&root, "src/a.ts", &options(1, 10)).unwrap();
    assert_eq!(report.target_commits, 2, "the wide commit still counts");
    assert_eq!(report.pair_commits, 1, "only the small commit pairs");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analytics_are_scoped_to_the_analysis_root() {
    let root = temporary_directory();
    init(&root);
    write(&root, "top.ts", "export const top = 1;\n");
    write(&root, "src/a.ts", "export const a = 1;\n");
    write(&root, "src/b.ts", "export const b = 1;\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");

    let context = worktree_context(&root.join("src"));
    let snapshot = analyze_git_at(&context, "HEAD").unwrap();
    let paths: Vec<&str> = snapshot
        .history
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(paths, ["a.ts", "b.ts"]);
    let touch_paths: Vec<&str> = snapshot
        .touches
        .iter()
        .map(|touches| touches.path.as_str())
        .collect();
    assert_eq!(touch_paths, ["a.ts", "b.ts"]);
    assert_eq!(snapshot.coupling.edges.len(), 1);
    assert_eq!(snapshot.coupling.edges[0].source, "a.ts");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn configured_excludes_are_removed_from_analytics() {
    let root = temporary_directory();
    init(&root);
    write(
        &root,
        "leadline.toml",
        "[analysis]\nexclude = [\"vendor/**\"]\n",
    );
    write(&root, "src/a.ts", "export const a = 1;\n");
    write(&root, "vendor/c.ts", "export const c = 1;\n");
    write(&root, "notes.txt", "not a supported source file\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");

    let context = worktree_context(&root);
    let snapshot = analyze_git_at(&context, "HEAD").unwrap();
    let paths: Vec<&str> = snapshot
        .history
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(paths, ["src/a.ts"]);
    let touch_paths: Vec<&str> = snapshot
        .touches
        .iter()
        .map(|touches| touches.path.as_str())
        .collect();
    assert_eq!(touch_paths, ["src/a.ts"]);
    assert!(snapshot.coupling.available);
    assert!(snapshot.coupling.edges.is_empty());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rename_retention_moves_touches_and_edges_to_the_new_path() {
    let root = temporary_directory();
    init(&root);
    write(
        &root,
        "src/old.ts",
        "export const a = 1;\nexport const b = 2;\nexport const c = 3;\n",
    );
    write(&root, "src/companion.ts", "export const companion = 1;\n");
    commit(&root, "one", "2026-03-01T12:00:00Z");

    git(&root, &["mv", "src/old.ts", "src/new.ts"]);
    append(&root, "src/new.ts", "export const d = 4;\n");
    append(&root, "src/companion.ts", "line two\n");
    commit(&root, "two", "2026-03-02T12:00:00Z");

    let context = worktree_context(&root);
    let snapshot = analyze_git_at(&context, "HEAD").unwrap();
    let paths: Vec<&str> = snapshot
        .history
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(paths, ["src/companion.ts", "src/new.ts"]);
    assert_eq!(snapshot.history.file("src/new.ts").unwrap().commits, 2);
    assert_eq!(
        touch_identities(&snapshot.touches, "src/new.ts"),
        ["a@example.invalid"]
    );
    assert_eq!(snapshot.coupling.edges.len(), 1);
    let edge = &snapshot.coupling.edges[0];
    assert_eq!(
        (edge.source.as_str(), edge.target.as_str()),
        ("src/companion.ts", "src/new.ts")
    );
    assert_eq!(edge.co_changes, 2);
    assert!((edge.directional - 1.0).abs() < 1e-9);
    assert!((edge.jaccard - 1.0).abs() < 1e-9);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ordering_and_public_serialization_are_deterministic() {
    let repo = analytics_fixture_three_commits();
    let context = worktree_context(&repo.path);
    let first = analyze_git_at(&context, &repo.third).unwrap();
    let second = analyze_git_at(&context, &repo.third).unwrap();
    assert_eq!(first, second);

    let touch_paths: Vec<&str> = first
        .touches
        .iter()
        .map(|touches| touches.path.as_str())
        .collect();
    let mut sorted_paths = touch_paths.clone();
    sorted_paths.sort_unstable();
    assert_eq!(touch_paths, sorted_paths);
    for touches in &first.touches {
        let identities: Vec<&str> = touches
            .identities
            .iter()
            .map(|(identity, _)| identity.as_str())
            .collect();
        let mut sorted = identities.clone();
        sorted.sort_unstable();
        assert_eq!(identities, sorted);
    }
    assert_eq!(
        serde_json::to_string(&first.history).unwrap(),
        serde_json::to_string(&second.history).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&first.coupling).unwrap(),
        serde_json::to_string(&second.coupling).unwrap()
    );
    let projected = serde_json::to_string(&first).unwrap();
    assert!(
        !projected.contains("identities") && !projected.contains("touches"),
        "identity rows are not serialized by default: {projected}"
    );
    assert!(projected.contains("\"history\""));
    assert!(projected.contains("\"coupling\""));

    std::fs::remove_dir_all(repo.path).unwrap();
}

#[test]
fn missing_git_reports_unavailable_analytics() {
    let root = temporary_directory();
    write(&root, "src/a.ts", "export const a = 1;\n");

    let context = worktree_context(&root);
    let snapshot = analyze_git_at(&context, "HEAD").unwrap();
    assert!(!snapshot.history.available);
    assert!(snapshot.history.files.is_empty());
    assert_eq!(snapshot.history.head_commit, None);
    assert!(snapshot.touches.is_empty());
    assert!(!snapshot.coupling.available);
    assert_eq!(snapshot.coupling.reason.as_deref(), Some("git_unavailable"));
    assert!(snapshot.coupling.edges.is_empty());

    std::fs::remove_dir_all(root).unwrap();
}
