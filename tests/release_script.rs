//! Release script: the bump it infers from conventional commits, the guards
//! that refuse an unsafe release, and the full path over throwaway
//! repositories.
//!
//! The script is bash and releases from Unix checkouts only, so these tests
//! compile to nothing on Windows.
#![cfg(unix)]

mod common;

use common::temporary_directory;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CURRENT: &str = "0.1.0";

/// The ten versioned files, stubbed with the same shapes as the real ones —
/// including where the trailing commas sit.
const VERSIONED_STUBS: [(&str, &str); 10] = [
    (
        "Cargo.toml",
        "[package]\nname = \"leadline\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    ),
    (
        "Cargo.lock",
        "[[package]]\nname = \"leadline\"\nversion = \"0.1.0\"\n",
    ),
    (
        "package.json",
        "{\n  \"name\": \"leadline\",\n  \"version\": \"0.1.0\",\n  \"private\": true\n}\n",
    ),
    (
        "integrations/agent-adapter-ts/package.json",
        "{\n  \"name\": \"leadline-agent-adapter\",\n  \"version\": \"0.1.0\",\n  \"private\": true\n}\n",
    ),
    (
        "integrations/agent-adapter-ts/package-lock.json",
        "{\n  \"name\": \"leadline-agent-adapter\",\n  \"version\": \"0.1.0\",\n  \"packages\": {\n    \"\": {\n      \"name\": \"leadline-agent-adapter\",\n      \"version\": \"0.1.0\",\n      \"devDependencies\": {}\n    }\n  }\n}\n",
    ),
    (
        "integrations/typesafe-triage/package.json",
        "{\n  \"name\": \"@leadline/typesafe-triage\",\n  \"version\": \"0.1.0\",\n  \"private\": true\n}\n",
    ),
    (
        "integrations/typesafe-triage/package-lock.json",
        "{\n  \"name\": \"@leadline/typesafe-triage\",\n  \"version\": \"0.1.0\",\n  \"packages\": {\n    \"\": {\n      \"name\": \"@leadline/typesafe-triage\",\n      \"version\": \"0.1.0\",\n      \"bin\": {}\n    }\n  }\n}\n",
    ),
    (
        "integrations/claude-code/.claude-plugin/plugin.json",
        "{\"name\":\"leadline\",\"version\":\"0.1.0\"}\n",
    ),
    (
        "integrations/pi/package.json",
        "{\n  \"name\": \"leadline-pi\",\n  \"version\": \"0.1.0\",\n  \"private\": true\n}\n",
    ),
    (
        "integrations/omp/package.json",
        "{\n  \"name\": \"leadline-omp\",\n  \"version\": \"0.1.0\",\n  \"private\": true\n}\n",
    ),
];

const CHANGELOG_STUB: &str = "# Changelog\n\n## Unreleased\n\n### Added\n\n- A feature.\n\n## 0.1.0 - 2026-01-01\n\n- The first one.\n";

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/release.sh")
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

fn git_output(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8(output.stdout).unwrap()
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn commit(root: &Path, message: &str) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "--no-gpg-sign", "-m", message]);
}

/// A clean repository on `main` with one commit tagged `v0.1.0`.
fn fixture() -> PathBuf {
    let root = temporary_directory();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "release@example.invalid"]);
    git(&root, &["config", "user.name", "Release Test"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["config", "tag.gpgsign", "false"]);
    write(&root, "README.md", "seed\n");
    commit(&root, "chore: seed");
    git(&root, &["tag", "v0.1.0"]);
    root
}

/// A repository with the ten versioned files and one `feat` commit on top of
/// `v0.1.0`, so a 0.2.0 minor release is ready to cut.
fn release_fixture() -> PathBuf {
    let root = fixture();
    for (path, contents) in VERSIONED_STUBS {
        write(&root, path, contents);
    }
    write(&root, "CHANGELOG.md", CHANGELOG_STUB);
    commit(&root, "feat: a feature");
    root
}

/// Give `root` a bare `origin` carrying its `main` and its `v0.1.0` tag.
fn bare_remote(root: &Path) -> PathBuf {
    let remote = temporary_directory().join("origin.git");
    git(root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(root, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(root, &["push", "-q", "-u", "origin", "main"]);
    git(root, &["push", "-q", "origin", "v0.1.0"]);
    remote
}

fn ls_remote(remote: &Path, pattern: &str) -> String {
    let output = Command::new("git")
        .args(["ls-remote", remote.to_str().unwrap(), pattern])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git ls-remote: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn release(root: &Path, args: &[&str]) -> Output {
    Command::new("bash")
        .arg(script())
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}

/// The `release: current=...` decision line, asserting the run succeeded.
fn decision(root: &Path, args: &[&str]) -> String {
    let output = release(root, args);
    assert!(
        output.status.success(),
        "release.sh {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .find(|line| line.starts_with("release: current="))
        .expect("the script prints a decision line")
        .to_owned()
}

fn decision_after(message: &str) -> String {
    let root = fixture();
    write(&root, "src/change.rs", "// change\n");
    commit(&root, message);
    decision(&root, &["--dry-run"])
}

#[test]
fn feat_is_a_minor_bump() {
    assert_eq!(
        decision_after("feat(parser): parse grouped imports"),
        "release: current=0.1.0 level=minor next=0.2.0"
    );
}

#[test]
fn fix_perf_and_revert_are_patch_bumps() {
    for message in [
        "fix: repair a thing",
        "perf: speed up a thing",
        "revert: undo a thing",
    ] {
        assert_eq!(
            decision_after(message),
            "release: current=0.1.0 level=patch next=0.1.1",
            "{message}"
        );
    }
}

#[test]
fn bang_and_breaking_change_are_major_bumps() {
    assert_eq!(
        decision_after("feat(api)!: drop the old envelope"),
        "release: current=0.1.0 level=major next=1.0.0"
    );
    assert_eq!(
        decision_after("chore: tidy\n\nBREAKING CHANGE: the store moves"),
        "release: current=0.1.0 level=major next=1.0.0"
    );
}

#[test]
fn other_types_release_nothing() {
    for message in [
        "docs: write words",
        "chore: tidy",
        "ci: reorder jobs",
        "test: add a case",
        "refactor: reshape a helper",
        "Merge branch 'topic' into main",
    ] {
        assert_eq!(
            decision_after(message),
            "release: current=0.1.0 level=none next=none",
            "{message}"
        );
    }
}

#[test]
fn a_tag_at_head_releases_nothing() {
    let root = fixture();
    assert_eq!(
        decision(&root, &["--dry-run"]),
        "release: current=0.1.0 level=none next=none"
    );
}

#[test]
fn explicit_version_overrides_inference() {
    let root = fixture();
    write(&root, "src/change.rs", "// change\n");
    commit(&root, "docs: irrelevant");
    assert_eq!(
        decision(&root, &["--dry-run", "2.0.0"]),
        "release: current=0.1.0 level=explicit next=2.0.0"
    );
}

#[test]
fn an_older_explicit_version_stops_the_release() {
    let root = fixture();
    let output = release(&root, &["--dry-run", "0.0.9"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not greater"), "{stderr}");
}

#[test]
fn a_dirty_tree_stops_the_release() {
    let root = release_fixture();
    write(&root, "README.md", "dirty\n");
    let output = release(&root, &["--no-verify", "--no-push"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unstaged changes"), "{stderr}");
}

#[test]
fn an_existing_tag_stops_the_release() {
    let root = release_fixture();
    // A tag on an unmerged (dangling) commit is invisible to the "highest
    // reachable tag" selection, so it collides with the inferred 0.2.0.
    let tree = git_output(&root, &["rev-parse", "HEAD^{tree}"])
        .trim()
        .to_owned();
    let dangling = git_output(&root, &["commit-tree", &tree, "-m", "orphan"])
        .trim()
        .to_owned();
    git(&root, &["tag", "v0.2.0", &dangling]);

    let output = release(&root, &["--no-verify", "--no-push"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"), "{stderr}");
}

#[test]
fn a_missing_unreleased_heading_stops_the_release() {
    let root = release_fixture();
    write(
        &root,
        "CHANGELOG.md",
        "# Changelog\n\n## 0.1.0 - 2026-01-01\n",
    );
    commit(&root, "docs: drop the heading");

    let output = release(&root, &["--no-verify", "--no-push"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no '## Unreleased' heading"), "{stderr}");
    assert!(stderr.contains("rolled back"), "{stderr}");
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo.contains(CURRENT), "{cargo}");
}

#[test]
fn releases_every_versioned_file_and_closes_the_changelog() {
    let root = release_fixture();
    write(&root, "untracked.txt", "not part of the release\n");

    let output = release(&root, &["--no-verify", "--no-push"]);
    assert!(
        output.status.success(),
        "release.sh: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("release: released v0.2.0 (minor)"),
        "{stdout}"
    );

    for (path, _) in VERSIONED_STUBS {
        let contents = fs::read_to_string(root.join(path)).unwrap();
        assert!(
            contents.contains("0.2.0"),
            "{path} keeps {CURRENT}: {contents}"
        );
        assert!(
            !contents.contains(CURRENT),
            "{path} still mentions {CURRENT}: {contents}"
        );
    }
    let changelog = fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
    assert!(
        changelog.contains("## Unreleased\n\n## 0.2.0 - "),
        "{changelog}"
    );
    assert!(changelog.contains("- A feature."), "{changelog}");
    assert!(changelog.ends_with("- The first one.\n"), "{changelog}");

    assert_eq!(
        git_output(&root, &["log", "-1", "--format=%s"]).trim(),
        "chore(release): bump to 0.2.0"
    );
    assert_eq!(
        git_output(&root, &["tag", "--points-at", "HEAD"]).trim(),
        "v0.2.0"
    );
    assert!(
        !git_output(&root, &["show", "--name-only", "--format=", "HEAD"]).contains("untracked.txt"),
        "untracked files stay out of the release commit"
    );
    assert!(root.join("untracked.txt").exists());
}

#[test]
fn push_publishes_the_commit_and_tag_together() {
    let root = release_fixture();
    let remote = bare_remote(&root);

    let output = release(&root, &["--no-verify"]);
    assert!(
        output.status.success(),
        "release.sh: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let head = git_output(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    assert!(
        ls_remote(&remote, "refs/heads/main").contains(&head),
        "origin/main did not move to the release commit"
    );
    assert!(
        ls_remote(&remote, "refs/tags/v0.2.0").contains(&head),
        "origin has no v0.2.0 tag at the release commit"
    );
}

#[test]
fn main_moving_ahead_stops_the_release_before_the_bump() {
    let root = release_fixture();
    let remote = bare_remote(&root);

    let clone = temporary_directory().join("clone");
    git(
        &root,
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    git(&clone, &["config", "user.email", "other@example.invalid"]);
    git(&clone, &["config", "user.name", "Other Author"]);
    write(&clone, "OTHER.md", "ahead\n");
    commit(&clone, "docs: someone else moved main");
    git(&clone, &["push", "-q", "origin", "main"]);

    let output = release(&root, &["--no-verify"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("origin/main has commits"), "{stderr}");
    assert_eq!(
        git_output(&root, &["log", "-1", "--format=%s"]).trim(),
        "feat: a feature",
        "no release commit should exist"
    );
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo.contains(CURRENT), "{cargo}");
    assert!(ls_remote(&remote, "refs/tags/v0.2.0").trim().is_empty());
}
