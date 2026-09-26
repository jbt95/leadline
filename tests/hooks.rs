//! Integration hook tests: the changed-code hooks must not spawn the analyzer
//! where it can only fail. Outside a repository, or before the base revision
//! exists, the hook answers immediately and calls nothing.

#![cfg(unix)]

mod common;
use common::{temporary_directory, write_executable};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Every hook that reports changed code after a tool call.
const HOOKS: &[&str] = &[
    "integrations/claude-code/hooks/leadline-changed.sh",
    "integrations/gemini/hooks/leadline-after-tool.sh",
    "integrations/cline/plugin/leadline-post-edit.sh",
];

fn hook_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// A fake `leadline` first on `PATH` that records every call in `$HOOK_LOG`.
fn fake_bin(root: &Path) -> PathBuf {
    let bin = root.join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    write_executable(
        &bin.join("leadline"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOOK_LOG\"\nexit 0\n",
    );
    bin
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit(root: &Path, message: &str) {
    std::fs::write(root.join("file.txt"), format!("{message}\n")).unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", message]);
}

fn run_hook(hook: &str, cwd: &Path, fake: &Path, log: &Path) -> std::process::Output {
    use std::io::Write as _;
    let mut child = Command::new(hook_path(hook))
        .current_dir(cwd)
        .env("PATH", format!("{}:/usr/bin:/bin", fake.display()))
        .env("HOOK_LOG", log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}\n").unwrap();
    child.wait_with_output().unwrap()
}

/// Recorded analyzer invocations. The log is written only when a hook really
/// spawns the analyzer, so a missing file is a failure here, not an empty
/// result: tests that expect no invocation assert `!log.exists()` instead of
/// calling this.
fn calls(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_else(|error| panic!("reading {}: {error}", log.display()))
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn changed_hooks_skip_outside_a_repository() {
    let root = temporary_directory();
    let fake = fake_bin(&root);
    let log = root.join("calls.log");
    for hook in HOOKS {
        let output = run_hook(hook, &root, &fake, &log);
        assert!(output.status.success(), "{hook}: {output:?}");
        assert!(
            !log.exists(),
            "{hook} spawned the analyzer outside a repository: {:?}",
            calls(&log)
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn changed_hooks_skip_before_the_base_commit_exists() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "a@example.invalid"]);
    git(&root, &["config", "user.name", "Author A"]);
    commit(&root, "first");
    let fake = fake_bin(&root);
    let log = root.join("calls.log");
    for hook in HOOKS {
        let output = run_hook(hook, &root, &fake, &log);
        assert!(output.status.success(), "{hook}: {output:?}");
        assert!(
            !log.exists(),
            "{hook} spawned the analyzer without a base revision: {:?}",
            calls(&log)
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn changed_hooks_analyze_once_the_base_commit_exists() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "a@example.invalid"]);
    git(&root, &["config", "user.name", "Author A"]);
    commit(&root, "first");
    commit(&root, "second");
    let fake = fake_bin(&root);
    let log = root.join("calls.log");
    for hook in HOOKS {
        let output = run_hook(hook, &root, &fake, &log);
        assert!(output.status.success(), "{hook}: {output:?}");
    }
    let recorded = calls(&log);
    assert_eq!(
        recorded.len(),
        HOOKS.len(),
        "every hook must run the analyzer once: {recorded:?}"
    );
    for call in &recorded {
        assert!(call.starts_with("changed"), "unexpected call: {call}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}
