//! Secret scanning over an installed `gitleaks`.
//!
//! Leadline never detects secrets itself: `gitleaks` stays the only detector,
//! always invoked with `--redact` and a fixed argument vector, with the
//! scanner's own output suppressed so no secret material reaches a log. This
//! module owns which paths are scanned and which command runs;
//! [`crate::security`] owns parsing, attribution, and gating, so an MCP caller
//! and the shell runner gate the same findings from the same reports.

use crate::diff::{self, ChangeOptions, ComparisonTarget};
use crate::security::ChangeComparison;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

/// Above this many changed paths one full-tree scan beats a per-file loop:
/// fewer processes, at the cost of reading the tree.
const PER_FILE_SCAN_LIMIT: usize = 100;

/// Create attempts for the private scan directory. A crashed run can leave a
/// directory behind under a recycled pid, and a stale name must not fail a scan.
const DIRECTORY_ATTEMPTS: u32 = 8;

static NEXT_SCAN: AtomicU32 = AtomicU32::new(0);

/// Scan scope: paths changed against `HEAD`, or the staged set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretMode {
    Worktree,
    Staged,
}

impl SecretMode {
    /// Parse a caller-supplied mode name.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "worktree" => Some(Self::Worktree),
            "staged" => Some(Self::Staged),
            _ => None,
        }
    }

    /// The mode name, as it appears in results.
    pub fn name(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Staged => "staged",
        }
    }

    /// Git comparison the gate narrows findings to: the same one the shell
    /// runner hands to `leadline security` for this mode.
    pub fn comparison(self) -> ChangeComparison {
        match self {
            Self::Worktree => ChangeComparison::Base("HEAD".to_owned()),
            Self::Staged => ChangeComparison::Staged,
        }
    }
}

/// Why a scan produced no report.
#[derive(Debug)]
pub enum SecretError {
    /// No scanner on `PATH`. Never reported as a clean scan.
    Unavailable(String),
    /// Git, scanner, or report failure.
    Failed(String),
}

impl SecretError {
    /// The underlying failure message.
    pub fn message(&self) -> &str {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => message,
        }
    }
}

/// One scan's SARIF reports and the private directory holding them. The
/// directory goes away with the value, so reports outlive no call.
#[derive(Debug)]
pub struct SecretScan {
    reports: Vec<PathBuf>,
    directory: PathBuf,
}

impl SecretScan {
    /// Reports to assemble, in scan order.
    pub fn reports(&self) -> &[PathBuf] {
        &self.reports
    }

    /// Create the private directory every report for one scan lands in.
    fn create() -> Result<Self, SecretError> {
        let base = std::env::temp_dir();
        for _ in 0..DIRECTORY_ATTEMPTS {
            let directory = base.join(format!(
                "leadline-secrets-{}-{}",
                std::process::id(),
                NEXT_SCAN.fetch_add(1, Ordering::Relaxed)
            ));
            match create_private_directory(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        reports: Vec::new(),
                        directory,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(SecretError::Failed(format!(
                        "cannot create the secret scan directory in {}: {error}",
                        base.display()
                    )));
                }
            }
        }
        Err(SecretError::Failed(format!(
            "cannot create a secret scan directory in {}",
            base.display()
        )))
    }

    /// Record a report and return its path for the next command.
    fn report(&mut self, name: &str) -> PathBuf {
        let path = self.directory.join(name);
        self.reports.push(path.clone());
        path
    }
}

impl Drop for SecretScan {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Create the scan directory private at creation, mirroring the shell runner's
/// `umask 077`. A `chmod` after `mkdir` would leave a window in which the
/// directory is world-readable, and a name that already exists as a symlink
/// makes `create` fail instead of following it.
#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

/// Scan `root` and return the reports to gate, or `None` when there is
/// nothing changed to scan and the scanner never ran.
pub fn scan(root: &Path, mode: SecretMode) -> Result<Option<SecretScan>, SecretError> {
    scan_with(root, mode, Path::new("gitleaks"))
}

/// [`scan`] against an explicit scanner path, so tests need no real
/// `gitleaks` on `PATH`.
fn scan_with(
    root: &Path,
    mode: SecretMode,
    scanner: &Path,
) -> Result<Option<SecretScan>, SecretError> {
    match mode {
        SecretMode::Staged => staged_scan(root, scanner).map(Some),
        SecretMode::Worktree => worktree_scan(root, scanner),
    }
}

/// Scan the paths changed against `HEAD`, untracked files included.
fn worktree_scan(root: &Path, scanner: &Path) -> Result<Option<SecretScan>, SecretError> {
    let changed = diff::changed_paths(
        root,
        &ChangeOptions {
            base: "HEAD".to_owned(),
            target: ComparisonTarget::Worktree,
            detect_renames: false,
        },
    )
    .map_err(|error| {
        SecretError::Failed(format!(
            "cannot list the paths changed against HEAD: {error}"
        ))
    })?;
    if changed.is_empty() {
        return Ok(None);
    }
    let mut scan = SecretScan::create()?;
    if changed.len() > PER_FILE_SCAN_LIMIT {
        // One full-tree scan: fewer processes than a per-file loop, and the
        // gate narrows the extra findings back to the changed paths.
        let report = scan.report("tree.sarif");
        run_scanner(scanner, root, &["dir"], &["."], &report)?;
        return Ok(Some(scan));
    }
    for (index, relative) in changed.iter().enumerate() {
        // Paths stay relative to the analysis root: the gate attributes
        // findings by the same root-relative paths Git reports. A deletion
        // cannot leak, and a non-file is not scannable.
        if !root.join(relative).is_file() {
            continue;
        }
        let report = scan.report(&format!("report.{index}.sarif"));
        run_scanner(scanner, root, &["dir"], &[relative], &report)?;
    }
    Ok(Some(scan))
}

/// Scan the staged set. Gitleaks reads the index itself, so there is no
/// changed-path list to build and no empty-set short-circuit here.
fn staged_scan(root: &Path, scanner: &Path) -> Result<SecretScan, SecretError> {
    let mut scan = SecretScan::create()?;
    let report = scan.report("staged.sarif");
    run_scanner(scanner, root, &["git", "--staged"], &[], &report)?;
    Ok(scan)
}

/// Run the scanner once and leave its report at `report`.
///
/// `subcommand` is fixed by this module and `--redact` is on every run, but
/// `paths` comes from the repository, so each one follows a `--` and stays a
/// value: a changed file named `-r` is a path, not a flag. Exit `0` is clean
/// and `1` is leaks found; anything higher is a scanner failure, and a missing
/// report is a failure too, so a broken run can never read as a clean one.
fn run_scanner(
    scanner: &Path,
    root: &Path,
    subcommand: &[&str],
    paths: &[&str],
    report: &Path,
) -> Result<(), SecretError> {
    let mut command = Command::new(scanner);
    command
        .args(subcommand)
        .args([
            "--no-banner",
            "--redact",
            "--report-format",
            "sarif",
            "--report-path",
        ])
        .arg(report);
    if !paths.is_empty() {
        command.arg("--").args(paths);
    }
    let status = command
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => SecretError::Unavailable(
                "gitleaks not found; install gitleaks to enable secret gating".to_owned(),
            ),
            _ => SecretError::Failed(format!("gitleaks could not run: {error}")),
        })?;
    // A signalled or otherwise uncoded exit has no code; treat it as failure
    // rather than as a clean scan.
    let code = status.code().unwrap_or(-1);
    if code > 1 {
        return Err(SecretError::Failed(format!(
            "gitleaks scan failed with exit {code}"
        )));
    }
    if !report.exists() {
        return Err(SecretError::Failed(format!(
            "gitleaks wrote no report to {}",
            report.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_names_parse_exactly_and_pick_their_comparison() {
        assert_eq!(SecretMode::parse("worktree"), Some(SecretMode::Worktree));
        assert_eq!(SecretMode::parse("staged"), Some(SecretMode::Staged));
        // Case-sensitive, and nothing outside the two documented modes.
        assert_eq!(SecretMode::parse("Worktree"), None);
        assert_eq!(SecretMode::parse("index"), None);
        assert_eq!(SecretMode::parse(""), None);
        assert_eq!(
            SecretMode::Worktree.comparison(),
            ChangeComparison::Base("HEAD".to_owned())
        );
        assert!(matches!(
            SecretMode::Staged.comparison(),
            ChangeComparison::Staged
        ));
    }

    #[cfg(unix)]
    mod with_fake_scanner {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU32, Ordering};

        static NEXT_FIXTURE: AtomicU32 = AtomicU32::new(0);

        /// One fixture: a Git repository to scan, plus a fake scanner beside
        /// it. The scanner lives outside the repository on purpose — inside it
        /// it would show up as an untracked changed path and be scanned.
        struct Fixture {
            base: PathBuf,
            repo: PathBuf,
            scanner: PathBuf,
        }

        impl Fixture {
            /// Fake scanner records each argv, writes the report it was asked
            /// for, and exits with the code in `exit-code` (default `0`).
            fn new(name: &str) -> Self {
                Self::build(name, true)
            }

            /// Fake scanner that never writes a report.
            fn new_without_report(name: &str) -> Self {
                Self::build(name, false)
            }

            fn build(name: &str, writes_report: bool) -> Self {
                let base = std::env::temp_dir().join(format!(
                    "leadline-secrets-test-{}-{}-{name}",
                    std::process::id(),
                    NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
                ));
                let _ = std::fs::remove_dir_all(&base);
                let repo = base.join("repo");
                std::fs::create_dir_all(&repo).unwrap();
                let fixture = Self {
                    scanner: base.join("fake-gitleaks"),
                    repo,
                    base,
                };
                fixture.set_exit_code(0);
                let write_report = if writes_report {
                    "  if [ \"$previous\" = \"--report-path\" ]; then printf '{}' > \"$argument\"; fi\n"
                } else {
                    ""
                };
                let log = fixture.args_log();
                std::fs::write(
                    &fixture.scanner,
                    format!(
                        "#!/bin/sh\n\
                         printf '%s\\n' \"$*\" >> '{log}'\n\
                         previous=\"\"\n\
                         for argument in \"$@\"; do\n\
                         {write_report}  previous=\"$argument\"\n\
                         done\n\
                         exit \"$(cat '{exit}')\"\n",
                        log = log.display(),
                        write_report = write_report,
                        exit = fixture.exit_code_file().display(),
                    ),
                )
                .unwrap();
                std::fs::set_permissions(&fixture.scanner, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
                fixture.git(&["init", "-q", "."]);
                fixture.commit(&[("empty.txt", "empty\n")], "empty");
                fixture
            }

            fn args_log(&self) -> PathBuf {
                self.base.join("scanner-args.log")
            }

            fn exit_code_file(&self) -> PathBuf {
                self.base.join("exit-code")
            }

            /// Force the fake scanner's exit code for the next scan.
            fn set_exit_code(&self, code: i32) {
                std::fs::write(self.exit_code_file(), format!("{code}\n")).unwrap();
            }

            fn git(&self, args: &[&str]) {
                crate::git::run(&self.repo, args).unwrap();
            }

            /// Write `files` (path, contents) and commit them.
            fn commit(&self, files: &[(&str, &str)], message: &str) {
                self.write(files);
                self.git(&["add", "-A", "--", "."]);
                self.git(&[
                    "-c",
                    "user.name=leadline",
                    "-c",
                    "user.email=leadline@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-q",
                    "-m",
                    message,
                ]);
            }

            fn write(&self, files: &[(&str, &str)]) {
                for (name, contents) in files {
                    let path = self.repo.join(name);
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).unwrap();
                    }
                    std::fs::write(path, contents).unwrap();
                }
            }

            /// Every argv the scanner was called with, in order.
            fn calls(&self) -> Vec<String> {
                let Ok(log) = std::fs::read_to_string(self.args_log()) else {
                    return Vec::new();
                };
                log.lines().map(str::to_owned).collect()
            }

            fn scan(&self, mode: SecretMode) -> Result<Option<SecretScan>, SecretError> {
                scan_with(&self.repo, mode, &self.scanner)
            }

            /// A repository with one committed, then modified, file: the
            /// smallest change set that still has something to scan.
            fn with_one_changed_file(name: &str) -> Self {
                let fixture = Self::new(name);
                fixture.commit(&[("app/config.txt", "clean\n")], "add config");
                fixture.write(&[("app/config.txt", "changed\n")]);
                fixture
            }
        }

        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.base);
            }
        }

        #[test]
        fn worktree_scans_each_changed_file_redacted_and_root_relative() {
            let fixture = Fixture::with_one_changed_file("per-file");

            let scan = fixture.scan(SecretMode::Worktree).unwrap().unwrap();
            let calls = fixture.calls();
            assert_eq!(calls.len(), 1, "{calls:?}");
            let call = &calls[0];
            assert!(call.starts_with("dir --no-banner --redact"), "{call}");
            assert!(call.contains("--report-format sarif"), "{call}");
            // Root-relative and behind `--`, so the gate attributes findings
            // the way Git does and the path stays a value.
            assert!(call.ends_with("-- app/config.txt"), "{call}");
            // No shell metacharacter ever reaches the argument vector.
            assert!(!call.contains(';') && !call.contains('|'), "{call}");
            assert_eq!(scan.reports().len(), 1);
            assert!(scan.reports()[0].exists());
        }

        #[test]
        fn staged_asks_the_scanner_for_the_index() {
            let fixture = Fixture::new("staged");
            fixture.commit(&[("app/config.txt", "token\n")], "add config");
            fixture.git(&["add", "-A", "--", "."]);

            fixture.scan(SecretMode::Staged).unwrap().unwrap();
            let calls = fixture.calls();
            assert_eq!(calls.len(), 1, "{calls:?}");
            assert!(calls[0].starts_with("git --staged "), "{calls:?}");
        }

        #[test]
        fn a_clean_tree_skips_the_scanner_entirely() {
            let fixture = Fixture::new("clean");

            // Nothing changed against HEAD: no scan, and no scanner run.
            assert!(fixture.scan(SecretMode::Worktree).unwrap().is_none());
            assert!(fixture.calls().is_empty(), "{:?}", fixture.calls());
        }

        #[test]
        fn many_changed_paths_fall_back_to_one_full_tree_scan() {
            let fixture = Fixture::new("full-tree");
            let names = (0..=PER_FILE_SCAN_LIMIT)
                .map(|index| format!("f{index}.txt"))
                .collect::<Vec<_>>();
            let added = names
                .iter()
                .map(|name| (name.as_str(), "x\n"))
                .collect::<Vec<_>>();
            fixture.commit(&added, "many files");
            // One past the per-file limit, so this cannot be a per-file loop.
            let changed = names
                .iter()
                .map(|name| (name.as_str(), "changed\n"))
                .collect::<Vec<_>>();
            fixture.write(&changed);

            fixture.scan(SecretMode::Worktree).unwrap().unwrap();
            let calls = fixture.calls();
            assert_eq!(calls.len(), 1, "{calls:?}");
            assert!(calls[0].ends_with("-- ."), "{calls:?}");
        }

        #[test]
        fn a_changed_path_that_looks_like_a_flag_stays_a_value() {
            let fixture = Fixture::new("flag-name");
            // `-r` is gitleaks' own short flag for --report-path. A bare
            // positional would be parsed as that flag and consume the next
            // argument, leaving the file unscanned.
            fixture.commit(&[("-r", "clean\n")], "add flag-named file");
            fixture.write(&[("-r", "changed\n")]);

            fixture.scan(SecretMode::Worktree).unwrap().unwrap();
            let calls = fixture.calls();
            assert_eq!(calls.len(), 1, "{calls:?}");
            assert!(
                calls[0].ends_with(" -- -r"),
                "the path must follow a `--`: {calls:?}"
            );
        }

        #[test]
        fn the_scan_directory_is_private_from_the_start() {
            let fixture = Fixture::with_one_changed_file("permissions");

            let scan = fixture.scan(SecretMode::Worktree).unwrap().unwrap();
            let directory = scan.reports()[0].parent().unwrap();
            let mode = std::fs::metadata(directory).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{directory:?} is not private");
        }

        #[test]
        fn a_missing_scanner_is_unavailable_and_never_a_clean_scan() {
            let fixture = Fixture::with_one_changed_file("missing");
            let absent = fixture.base.join("not-installed");

            let error = scan_with(&fixture.repo, SecretMode::Worktree, &absent).unwrap_err();
            assert!(matches!(error, SecretError::Unavailable(_)), "{error:?}");
            assert!(error.message().contains("gitleaks"), "{}", error.message());
        }

        #[test]
        fn a_failing_scanner_fails_the_scan() {
            let fixture = Fixture::with_one_changed_file("failing");
            // Exit 3 is a scanner failure, not a clean scan.
            fixture.set_exit_code(3);

            let error = fixture.scan(SecretMode::Worktree).unwrap_err();
            assert!(matches!(error, SecretError::Failed(_)), "{error:?}");
            assert!(error.message().contains('3'), "{}", error.message());
        }

        #[test]
        fn leaks_found_is_not_a_failure() {
            let fixture = Fixture::with_one_changed_file("leaks");
            // Exit 1 is gitleaks' findings exit code: a scan that ran.
            fixture.set_exit_code(1);

            let scan = fixture.scan(SecretMode::Worktree).unwrap().unwrap();
            assert_eq!(scan.reports().len(), 1);
        }

        #[test]
        fn a_scanner_that_writes_no_report_fails_the_scan() {
            let fixture = Fixture::new_without_report("no-report");
            fixture.commit(&[("app/config.txt", "clean\n")], "add config");
            fixture.write(&[("app/config.txt", "changed\n")]);

            let error = fixture.scan(SecretMode::Worktree).unwrap_err();
            assert!(matches!(error, SecretError::Failed(_)), "{error:?}");
            assert!(error.message().contains("no report"), "{}", error.message());
        }

        #[test]
        fn reports_outlive_no_call() {
            let fixture = Fixture::with_one_changed_file("cleanup");

            let scan = fixture.scan(SecretMode::Worktree).unwrap().unwrap();
            let report = scan.reports()[0].clone();
            assert!(report.exists());
            drop(scan);
            assert!(
                !report.exists(),
                "the scan directory must not outlive the scan"
            );
        }

        #[test]
        fn a_directory_without_commits_cannot_be_compared() {
            let fixture = Fixture::build("no-commits", true);
            // Drop the only commit: there is no HEAD to compare against.
            fixture.git(&["update-ref", "-d", "refs/heads/master"]);
            fixture.git(&["update-ref", "-d", "refs/heads/main"]);
            fixture.write(&[("empty.txt", "changed\n")]);

            let error = fixture.scan(SecretMode::Worktree).unwrap_err();
            assert!(matches!(error, SecretError::Failed(_)), "{error:?}");
            assert!(!error.message().is_empty());
        }
    }
}
