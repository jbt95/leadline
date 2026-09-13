use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn revision_validation_rejects_option_and_object_syntax() {
    for value in ["", "--help", "HEAD:file", "bad\nref", ":HEAD"] {
        assert!(
            leadline::git::validate_revision(value).is_err(),
            "{value:?}"
        );
    }
    assert!(leadline::git::validate_revision("HEAD~1").is_ok());
}

#[cfg(unix)]
#[test]
fn adapter_and_existing_consumers_disable_fetch_locks_and_stdin() {
    let _lock = env_lock();
    let root = temporary_directory("fake-repo");
    let bin = root.join("bin");
    let log = root.join("git.log");
    std::fs::create_dir(&bin).unwrap();
    write_fake_git(&bin.join("git"));

    {
        let _env = FakeGitEnv::install(&bin, &log, &root);

        let output = leadline::git::run(&root, &["status"]).unwrap();
        assert!(output.status.success());
        assert_eq!(
            leadline::git::run_optional(&root, &["optional-missing"]).unwrap(),
            None
        );
        assert_eq!(
            leadline::git::repo_root(&root).unwrap(),
            Some(std::fs::canonicalize(&root).unwrap())
        );

        let changed = leadline::diff::analyze_changed(&root, "HEAD").unwrap();
        assert_eq!(changed.base, "HEAD");
        assert!(changed.functions.is_empty());
        assert!(changed.parse_errors.is_empty());

        let history = leadline::history::analyze_history(&root).unwrap();
        assert!(history.available);
        assert_eq!(history.head_commit.as_deref(), Some(FAKE_COMMIT));
        assert_eq!(history.head_timestamp, Some(123));
        assert!(history.files.is_empty());
    }

    let invocations = std::fs::read_to_string(&log).unwrap();
    let command_count = invocations.matches("command=").count();
    assert!(command_count >= 7, "{invocations}");
    assert_eq!(
        invocations.matches("GIT_NO_LAZY_FETCH=1").count(),
        command_count,
        "{invocations}"
    );
    assert_eq!(
        invocations.matches("GIT_OPTIONAL_LOCKS=0").count(),
        command_count,
        "{invocations}"
    );
    assert_eq!(
        invocations.matches("stdin=eof").count(),
        command_count,
        "{invocations}"
    );
    assert!(invocations.contains("command=diff "), "{invocations}");
    assert!(invocations.contains("command=log "), "{invocations}");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn repo_root_returns_none_outside_a_repository() {
    let _lock = env_lock();
    let root = temporary_directory("not-a-repo");
    assert_eq!(leadline::git::repo_root(&root).unwrap(), None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn repo_root_finds_the_canonical_repository_root() {
    let _lock = env_lock();
    let root = temporary_directory("repo-root");
    git(&root, &["init", "-q"]);
    let nested = root.join("one/two");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(
        leadline::git::repo_root(&nested).unwrap(),
        Some(std::fs::canonicalize(&root).unwrap())
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolve_commit_returns_metadata_and_errors_for_a_missing_object() {
    let _lock = env_lock();
    let root = temporary_directory("resolve");
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git_dated(&root, &["commit", "-qm", "one"], "2025-01-02T03:04:05Z");

    let expected_commit = git_stdout(&root, &["rev-parse", "HEAD"]);
    let expected_timestamp = git_stdout(&root, &["show", "-s", "--format=%ct", "HEAD"])
        .parse::<i64>()
        .unwrap();
    assert_eq!(
        leadline::git::resolve_commit(&root, "HEAD").unwrap(),
        (expected_commit, expected_timestamp)
    );
    assert!(leadline::git::resolve_commit(&root, "definitely-missing").is_err());

    std::fs::remove_dir_all(root).unwrap();
}

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn git(root: &Path, args: &[&str]) {
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
}

fn git_dated(root: &Path, args: &[&str], date: &str) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn temporary_directory(label: &str) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "leadline-git-adapter-{}-{id}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[cfg(unix)]
const FAKE_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[cfg(unix)]
fn write_fake_git(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        r#"#!/bin/sh
{{
  printf 'command=%s\n' "$*"
  printf 'GIT_NO_LAZY_FETCH=%s\n' "$GIT_NO_LAZY_FETCH"
  printf 'GIT_OPTIONAL_LOCKS=%s\n' "$GIT_OPTIONAL_LOCKS"
  if IFS= read -r value; then
    printf 'stdin=%s\n' "$value"
  else
    printf 'stdin=eof\n'
  fi
}} >> "$FAKE_GIT_LOG"

case "$1" in
  status) exit 0 ;;
  optional-missing) printf 'missing object\n' >&2; exit 1 ;;
  rev-parse)
    if [ "$2" = "--show-toplevel" ]; then
      printf '%s\n' "$FAKE_GIT_ROOT"
      exit 0
    fi
    ;;
  diff) exit 0 ;;
  ls-files) exit 0 ;;
  log)
    if [ "$2" = "-1" ]; then
      printf '{FAKE_COMMIT}'
      printf '\000'
      printf '123\n'
    fi
    exit 0
    ;;
esac

printf 'unexpected fake git command: %s\n' "$*" >&2
exit 42
"#
    );
    std::fs::write(path, script).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
struct FakeGitEnv {
    path: Option<OsString>,
    log: Option<OsString>,
    root: Option<OsString>,
}

#[cfg(unix)]
impl FakeGitEnv {
    fn install(bin: &Path, log: &Path, root: &Path) -> Self {
        let previous = Self {
            path: std::env::var_os("PATH"),
            log: std::env::var_os("FAKE_GIT_LOG"),
            root: std::env::var_os("FAKE_GIT_ROOT"),
        };
        // SAFETY: every test in this binary that starts git holds ENV_LOCK.
        unsafe {
            std::env::set_var("PATH", bin);
            std::env::set_var("FAKE_GIT_LOG", log);
            std::env::set_var("FAKE_GIT_ROOT", root);
        }
        previous
    }
}

#[cfg(unix)]
impl Drop for FakeGitEnv {
    fn drop(&mut self) {
        // SAFETY: the guard is dropped while the caller still holds ENV_LOCK.
        unsafe {
            restore_var("PATH", self.path.take());
            restore_var("FAKE_GIT_LOG", self.log.take());
            restore_var("FAKE_GIT_ROOT", self.root.take());
        }
    }
}

#[cfg(unix)]
unsafe fn restore_var(name: &str, value: Option<OsString>) {
    if let Some(value) = value {
        // SAFETY: callers serialize environment access with ENV_LOCK.
        unsafe { std::env::set_var(name, value) };
    } else {
        // SAFETY: callers serialize environment access with ENV_LOCK.
        unsafe { std::env::remove_var(name) };
    }
}
