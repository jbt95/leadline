use leadline::graph::{analyze_dependencies, analyze_dependencies_from_sources};
use leadline::source_snapshot::{SnapshotTarget, SourceEntry, SourceSnapshot, load};
use leadline::{analyze_path, analyze_sources};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory(label: &str) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "leadline-source-snapshot-{}-{id}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn git_init(root: &Path) {
    git(root, &["init", "-q", "."]);
    git(root, &["config", "user.email", "fixture@example.com"]);
    git(root, &["config", "user.name", "Fixture"]);
}

fn commit(root: &Path, message: &str) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", message, "--no-gpg-sign"]);
}

const BASE_CONFIG: &str =
    "[analysis]\nexclude = [\"src/excluded.ts\", \"src/tracked-excluded.ts\"]\n";
const BASE_MAILMAP: &str = "Fixture <fixture@example.com>\n";
const CURRENT_SOURCE: &str = "export const current = 1;\n";

struct Fixture {
    root: PathBuf,
    base: String,
}

/// Repository with tracked, tracked-ignored, untracked, ignored, excluded,
/// symlinked, and generated files across two commits.
fn snapshot_fixture(label: &str) -> Fixture {
    let root = temporary_directory(label);
    git_init(&root);
    // Track an ignored file before the ignore rule exists.
    write(
        &root,
        "src/tracked-ignored.ts",
        "export const trackedIgnored = 1;\n",
    );
    git(&root, &["add", "src/tracked-ignored.ts"]);
    write(
        &root,
        ".gitignore",
        "src/ignored.ts\nsrc/tracked-ignored.ts\n",
    );
    write(&root, "leadline.toml", BASE_CONFIG);
    write(&root, ".mailmap", BASE_MAILMAP);
    write(&root, "src/base-only.ts", "export const baseOnly = 1;\n");
    commit(&root, "base");
    let base = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();

    git(&root, &["rm", "-q", "src/base-only.ts"]);
    write(&root, "src/current.ts", CURRENT_SOURCE);
    write(
        &root,
        "src/tracked-excluded.ts",
        "export const trackedExcluded = 1;\n",
    );
    write(
        &root,
        "generated/tracked-gen.ts",
        "export const trackedGenerated = 1;\n",
    );
    commit(&root, "current");

    write(&root, "src/untracked.ts", "export const untracked = 1;\n");
    write(&root, "src/excluded.ts", "export const excluded = 1;\n");
    write(&root, "src/ignored.ts", "export const ignored = 1;\n");
    write(
        &root,
        "generated/untracked-gen.ts",
        "export const generated = 1;\n",
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink("current.ts", root.join("src/symlink.ts")).unwrap();
    Fixture { root, base }
}

fn graph_fixture(label: &str) -> Fixture {
    let root = temporary_directory(label);
    git_init(&root);
    write(&root, "src/a.ts", "import './b';\nexport const a = 1;\n");
    write(&root, "src/b.ts", "import './a';\nexport const b = 1;\n");
    write(&root, "src/standalone.ts", "export const standalone = 1;\n");
    commit(&root, "graph");
    let base = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    Fixture { root, base }
}

fn paths(snapshot: &SourceSnapshot) -> Vec<&str> {
    snapshot
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect()
}

fn bytes_of<'a>(snapshot: &'a SourceSnapshot, path: &str) -> &'a [u8] {
    &snapshot
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .unwrap_or_else(|| panic!("missing entry {path}"))
        .bytes
}

#[test]
fn targets_are_isolated_and_filtered_identically() {
    let fixture = snapshot_fixture("isolate");
    let (_, worktree) = load(&fixture.root, SnapshotTarget::Worktree).unwrap();
    let (_, index) = load(&fixture.root, SnapshotTarget::Index).unwrap();
    let (_, base) = load(
        &fixture.root,
        SnapshotTarget::Revision(fixture.base.clone()),
    )
    .unwrap();

    assert_eq!(worktree.target, SnapshotTarget::Worktree);
    assert_eq!(
        paths(&worktree),
        [
            "src/current.ts",
            "src/tracked-ignored.ts",
            "src/untracked.ts"
        ]
    );
    assert_eq!(paths(&index), ["src/current.ts", "src/tracked-ignored.ts"]);
    assert_eq!(paths(&base), ["src/base-only.ts", "src/tracked-ignored.ts"]);
    assert_ne!(worktree.commit, base.commit);
    assert_eq!(worktree.commit, index.commit);
    assert_eq!(worktree.commit_timestamp, index.commit_timestamp);
    assert_eq!(base.commit, Some(fixture.base.clone()));
}

#[test]
fn dirty_worktree_state_does_not_change_index_or_revision_targets() {
    let fixture = snapshot_fixture("dirty");
    std::fs::write(
        fixture.root.join("src/current.ts"),
        "export const dirty = 1;\n",
    )
    .unwrap();
    std::fs::write(
        fixture.root.join("leadline.toml"),
        "[analysis]\nexclude = [\"src/tracked-ignored.ts\", \"src/tracked-excluded.ts\"]\n",
    )
    .unwrap();
    std::fs::write(fixture.root.join(".mailmap"), "Dirty <dirty@example.com>\n").unwrap();

    let (worktree_context, worktree) = load(&fixture.root, SnapshotTarget::Worktree).unwrap();
    let (index_context, index) = load(&fixture.root, SnapshotTarget::Index).unwrap();
    let (revision_context, revision) =
        load(&fixture.root, SnapshotTarget::Revision("HEAD".into())).unwrap();

    assert_eq!(
        bytes_of(&worktree, "src/current.ts"),
        b"export const dirty = 1;\n"
    );
    assert_eq!(
        bytes_of(&index, "src/current.ts"),
        CURRENT_SOURCE.as_bytes()
    );
    assert_eq!(
        bytes_of(&revision, "src/current.ts"),
        CURRENT_SOURCE.as_bytes()
    );

    assert_eq!(
        worktree_context.config.analysis_excludes,
        ["src/tracked-ignored.ts", "src/tracked-excluded.ts"]
    );
    assert_eq!(
        index_context.config.analysis_excludes,
        ["src/excluded.ts", "src/tracked-excluded.ts"]
    );
    assert_eq!(
        revision_context.config.analysis_excludes,
        ["src/excluded.ts", "src/tracked-excluded.ts"]
    );

    assert_eq!(
        paths(&worktree),
        ["src/current.ts", "src/excluded.ts", "src/untracked.ts"]
    );
    assert_eq!(paths(&index), ["src/current.ts", "src/tracked-ignored.ts"]);
    assert_eq!(
        paths(&revision),
        ["src/current.ts", "src/tracked-ignored.ts"]
    );

    assert_eq!(index.config_bytes.as_deref(), Some(BASE_CONFIG.as_bytes()));
    assert_eq!(
        revision.config_bytes.as_deref(),
        Some(BASE_CONFIG.as_bytes())
    );
    assert_eq!(
        index.mailmap_bytes.as_deref(),
        Some(BASE_MAILMAP.as_bytes())
    );
    assert_eq!(
        revision.mailmap_bytes.as_deref(),
        Some(BASE_MAILMAP.as_bytes())
    );
    assert_eq!(
        worktree.mailmap_bytes.as_deref(),
        Some(b"Dirty <dirty@example.com>\n".as_slice())
    );
}

#[test]
fn entries_are_sorted_and_deterministic() {
    let fixture = snapshot_fixture("deterministic");
    let (_, first) = load(&fixture.root, SnapshotTarget::Worktree).unwrap();
    let (_, second) = load(&fixture.root, SnapshotTarget::Worktree).unwrap();
    assert_eq!(first, second);
    assert!(
        first
            .entries
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    );

    let (_, base_first) = load(
        &fixture.root,
        SnapshotTarget::Revision(fixture.base.clone()),
    )
    .unwrap();
    let (_, base_second) = load(
        &fixture.root,
        SnapshotTarget::Revision(fixture.base.clone()),
    )
    .unwrap();
    assert_eq!(base_first, base_second);
    assert!(
        base_first
            .entries
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    );
}

#[test]
fn unmerged_index_entries_are_incomplete() {
    let root = temporary_directory("unmerged");
    git_init(&root);
    write(&root, "src/conflict.ts", "export const value = 'base';\n");
    commit(&root, "base");
    git(&root, &["checkout", "-q", "-b", "side"]);
    write(&root, "src/conflict.ts", "export const value = 'side';\n");
    commit(&root, "side");
    let side = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    git(&root, &["checkout", "-q", "-"]);
    write(&root, "src/conflict.ts", "export const value = 'main';\n");
    commit(&root, "main");

    let merge = Command::new("git")
        .current_dir(&root)
        .args(["merge", "--no-edit", &side])
        .output()
        .unwrap();
    assert!(
        !merge.status.success(),
        "expected a merge conflict, got: {}",
        String::from_utf8_lossy(&merge.stdout)
    );

    let error = load(&root, SnapshotTarget::Index).unwrap_err().to_string();
    assert!(error.contains("unmerged"), "unexpected error: {error}");
    assert!(load(&root, SnapshotTarget::Revision("HEAD".into())).is_ok());
}

#[test]
fn intent_to_add_index_entries_are_omitted() {
    let fixture = snapshot_fixture("intent");
    write(&fixture.root, "src/intent.ts", "export const intent = 1;\n");
    git(&fixture.root, &["add", "-N", "src/intent.ts"]);

    let (_, index) = load(&fixture.root, SnapshotTarget::Index).unwrap();
    assert_eq!(paths(&index), ["src/current.ts", "src/tracked-ignored.ts"]);
    let (_, worktree) = load(&fixture.root, SnapshotTarget::Worktree).unwrap();
    assert_eq!(
        paths(&worktree),
        [
            "src/current.ts",
            "src/intent.ts",
            "src/tracked-ignored.ts",
            "src/untracked.ts"
        ]
    );
}

#[test]
fn missing_index_objects_are_incomplete() {
    let root = temporary_directory("missing-object");
    git_init(&root);
    write(&root, "src/a.ts", "export const a = 1;\n");
    commit(&root, "base");
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "100644,1111111111111111111111111111111111111111,src/missing.ts",
        ],
    );

    let error = load(&root, SnapshotTarget::Index).unwrap_err().to_string();
    assert!(
        error.contains("missing") || error.contains("promisor"),
        "unexpected error: {error}"
    );
}

#[test]
fn scope_prefix_maps_repository_paths_into_the_analysis_root() {
    let fixture = snapshot_fixture("scope");
    let nested = fixture.root.join("src");
    let (context, worktree) = load(&nested, SnapshotTarget::Worktree).unwrap();
    assert_eq!(context.scope_prefix, "src/");
    assert_eq!(
        context.repo_root.as_deref(),
        Some(std::fs::canonicalize(&fixture.root).unwrap().as_path())
    );
    assert_eq!(
        context.analysis_root,
        std::fs::canonicalize(&nested).unwrap()
    );
    assert_eq!(
        paths(&worktree),
        [
            "current.ts",
            "excluded.ts",
            "tracked-excluded.ts",
            "tracked-ignored.ts",
            "untracked.ts"
        ]
    );

    let (_, index) = load(&nested, SnapshotTarget::Index).unwrap();
    assert_eq!(
        paths(&index),
        ["current.ts", "tracked-excluded.ts", "tracked-ignored.ts"]
    );
    let (_, base) = load(&nested, SnapshotTarget::Revision(fixture.base.clone())).unwrap();
    assert_eq!(paths(&base), ["base-only.ts", "tracked-ignored.ts"]);
}

#[test]
fn explicit_files_bypass_directory_filters() {
    let fixture = snapshot_fixture("explicit");
    let ignored = fixture.root.join("src/ignored.ts");
    let (context, snapshot) = load(&ignored, SnapshotTarget::Worktree).unwrap();
    assert_eq!(
        context.analysis_root,
        std::fs::canonicalize(fixture.root.join("src")).unwrap()
    );
    assert_eq!(context.scope_prefix, "src/");
    assert_eq!(paths(&snapshot), ["ignored.ts"]);
    assert_eq!(snapshot.entries[0].bytes, b"export const ignored = 1;\n");

    let current = fixture.root.join("src/current.ts");
    let (_, index) = load(&current, SnapshotTarget::Index).unwrap();
    assert_eq!(paths(&index), ["current.ts"]);
    assert_eq!(index.entries[0].bytes, CURRENT_SOURCE.as_bytes());
    let (_, revision) = load(&current, SnapshotTarget::Revision("HEAD".into())).unwrap();
    assert_eq!(paths(&revision), ["current.ts"]);
    assert_eq!(revision.entries[0].bytes, CURRENT_SOURCE.as_bytes());

    write(&fixture.root, "README.md", "not source\n");
    assert!(load(&fixture.root.join("README.md"), SnapshotTarget::Worktree).is_err());
}

#[test]
fn non_git_worktrees_fall_back_to_filesystem_discovery() {
    let root = temporary_directory("non-git");
    write(
        &root,
        "leadline.toml",
        "[analysis]\nexclude = [\"src/excluded.ts\"]\n",
    );
    write(&root, "src/a.ts", "export const a = 1;\n");
    write(&root, "src/b.ts", "export const b = 1;\n");
    write(&root, "src/excluded.ts", "export const excluded = 1;\n");
    write(&root, "src/ignored.ts", "export const ignored = 1;\n");
    write(&root, ".gitignore", "src/ignored.ts\n");
    write(&root, "generated/gen.ts", "export const generated = 1;\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.ts", root.join("src/symlink.ts")).unwrap();

    let (context, snapshot) = load(&root, SnapshotTarget::Worktree).unwrap();
    assert!(context.repo_root.is_none());
    assert!(snapshot.commit.is_none());
    assert!(snapshot.commit_timestamp.is_none());
    assert!(snapshot.mailmap_bytes.is_none());
    assert_eq!(
        snapshot.config_bytes.as_deref(),
        Some(b"[analysis]\nexclude = [\"src/excluded.ts\"]\n".as_slice())
    );
    assert_eq!(paths(&snapshot), ["src/a.ts", "src/b.ts"]);

    assert!(load(&root, SnapshotTarget::Index).is_err());
    assert!(load(&root, SnapshotTarget::Revision("HEAD".into())).is_err());
}

#[test]
fn analyze_sources_sorts_parallel_results_and_matches_filesystem_analysis() {
    let root = temporary_directory("sources");
    let mut expected = Vec::new();
    for index in 0..48 {
        let relative = format!("src/module-{index:02}.ts");
        let source = format!(
            "export function f{index}(x: number) {{ return x > {index} ? x : {index}; }}\n"
        );
        write(&root, &relative, &source);
        expected.push((relative, source));
    }
    let entries: Vec<SourceEntry> = expected
        .iter()
        .rev()
        .map(|(path, source)| SourceEntry {
            path: path.clone(),
            bytes: source.as_bytes().to_vec(),
        })
        .collect();

    let report = analyze_sources(&entries, None).unwrap();
    let report_paths: Vec<&str> = report.files.iter().map(|file| file.path.as_str()).collect();
    let expected_paths: Vec<&str> = expected.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(report_paths, expected_paths);
    assert!(report.files.iter().all(|file| file.functions.len() == 1));

    let filesystem = analyze_path(&root, None).unwrap();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(&filesystem).unwrap()
    );
}

#[test]
fn in_memory_graph_matches_filesystem_graph() {
    let fixture = graph_fixture("graph");
    let (context, snapshot) = load(&fixture.root, SnapshotTarget::Worktree).unwrap();
    assert_eq!(
        analyze_dependencies_from_sources(&snapshot.entries).unwrap(),
        analyze_dependencies(&context.analysis_root, &context.config.analysis_excludes).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_git_paths_are_input_errors() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let root = temporary_directory("non-utf8");
    git_init(&root);
    write(&root, "src/ok.ts", "export const ok = 1;\n");
    commit(&root, "base");

    // APFS rejects non-UTF-8 filenames, so record the path in the index
    // directly; the file never needs to exist on disk.
    let oid = git(&root, &["rev-parse", "HEAD:src/ok.ts"])
        .trim()
        .to_owned();
    let mut cacheinfo = OsString::from("100644,");
    cacheinfo.push(&oid);
    cacheinfo.push(",src/bad-");
    cacheinfo.push(OsString::from_vec(vec![0xff]));
    cacheinfo.push(".ts");
    let output = Command::new("git")
        .current_dir(&root)
        .args(["update-index", "--add", "--cacheinfo"])
        .arg(&cacheinfo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "update-index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(load(&root, SnapshotTarget::Index).is_err());
}

#[test]
fn control_character_paths_are_input_errors() {
    let root = temporary_directory("control-path");
    std::fs::write(root.join("bad-\u{1}.ts"), "export const bad = 1;\n").unwrap();
    assert!(load(&root, SnapshotTarget::Worktree).is_err());
}
