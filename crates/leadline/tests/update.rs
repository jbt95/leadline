//! Offline end-to-end tests for `leadline update`.
//!
//! Each test copies the built binary into a temporary directory and runs the
//! copy against a fake release tree served over `file://`, so the real build
//! output is never replaced and no network is used.
#![cfg(unix)]

mod common;

use common::{temporary_directory, tool_available, write_executable};
use std::path::{Path, PathBuf};
use std::process::Command;

fn checksum_tool_available() -> bool {
    tool_available("sha256sum") || tool_available("shasum")
}

fn prerequisites_missing() -> bool {
    if !tool_available("curl") || !tool_available("tar") || !checksum_tool_available() {
        eprintln!("skipping: curl, tar, or a checksum tool is unavailable");
        return true;
    }
    false
}

/// Builds a fake release tree mirroring the layout of `releases/latest/download`.
fn build_release(root: &Path, version: &str, binary_body: &str, corrupt_sums: bool) -> PathBuf {
    let download = root.join("releases/latest/download");
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(download.join("VERSION"), format!("v{version}\n")).unwrap();

    let payload = root.join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    write_executable(&payload.join("leadline"), binary_body);

    let archive = leadline::update::archive_for(std::env::consts::OS, std::env::consts::ARCH)
        .expect("test platform is supported");
    assert!(
        Command::new("tar")
            .args(["-czf"])
            .arg(download.join(archive))
            .arg("-C")
            .arg(&payload)
            .arg("leadline")
            .status()
            .unwrap()
            .success()
    );

    let sum = if corrupt_sums {
        "0".repeat(64)
    } else {
        leadline::update::sha256_file(&download.join(archive)).unwrap()
    };
    std::fs::write(download.join("SHA256SUMS"), format!("{sum}  {archive}\n")).unwrap();

    root.to_owned()
}

/// Copies the binary under test into a temporary bin directory.
fn copied_binary(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let copy = bin.join("leadline");
    std::fs::copy(env!("CARGO_BIN_EXE_leadline"), &copy).unwrap();
    copy
}

/// Spawns `command` to completion, retrying when the kernel reports the
/// freshly copied test binary as busy (`ETXTBSY`). Under parallel load a
/// copy immediately followed by an exec can transiently observe the file as
/// still open for writing; the next attempt succeeds.
fn spawn_output(command: &mut Command) -> std::process::Output {
    let mut attempts = 0;
    loop {
        match command.output() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 10 =>
            {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            outcome => return outcome.unwrap(),
        }
    }
}

/// Command for a copied binary with the ambient local-metrics store removed:
/// `cargo test` must never write into the developer's store.
fn binary_command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command.env_remove("LEADLINE_METRICS_DIR");
    command
}

fn run_update(binary: &Path, release: &Path) -> std::process::Output {
    let mut command = binary_command(binary);
    command
        .arg("update")
        .env("LEADLINE_BASE_URL", format!("file://{}", release.display()));
    spawn_output(&mut command)
}

const PI_LISTING: &str = "User packages:\n  npm:pi-web-access\n    /tmp/x\n  git:github.com/jbt95/leadline\n    /tmp/leadline\n";
const OMP_LISTING: &str = r#"{"npm":[{"name":"ponytail","version":"4.9.0"},{"name":"leadline","version":"0.9.0"}],"marketplace":[]}"#;
const CLAUDE_LISTING: &str = r#"[{"id":"claude-hud@claude-hud","version":"0.8.0"},{"id":"leadline@leadline","version":"0.9.0"}]"#;

/// Command PATH with `fake_bin` first and only the system directories the
/// update flow needs, so fake harnesses always win and harnesses installed
/// into user directories stay out of reach.
fn fake_path(fake_bin: &Path) -> String {
    format!("{}:/usr/bin:/bin", fake_bin.display())
}

/// Writes a fake harness that records every call and answers the probe and
/// update commands: a `list` argument cats `listing`, anything else exits
/// with `update_exit`.
fn write_fake_harness(dir: &Path, name: &str, listing: &str, update_exit: i32) {
    let listing_file = dir.join(format!("{name}-list.txt"));
    std::fs::write(&listing_file, listing).unwrap();
    let body = format!(
        "#!/bin/sh\nprintf '%s %s\\n' {name} \"$*\" >> \"$FAKE_LOG\"\nif [ \"$1\" = \"list\" ] || [ \"$2\" = \"list\" ]; then\n  cat \"{}\"\n  exit 0\nfi\nexit {update_exit}\n",
        listing_file.display(),
    );
    write_executable(&dir.join(name), &body);
}

fn recorded_calls(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn run_binary(binary: &Path, args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
    let mut command = binary_command(binary);
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    spawn_output(&mut command)
}

/// Shared setup for `--integrations` tests: a current-version release tree,
/// a binary copy, a fake harness bin, a call log, and an isolated config dir.
///
/// `harnesses` lists `(name, probe listing, update exit status)` fakes to
/// install; every fake records its invocations in the log. The temp root is
/// removed on drop, even when an assertion fails.
struct IntegrationFixture {
    root: PathBuf,
    binary: PathBuf,
    log: PathBuf,
    config: PathBuf,
    path: String,
    base_url: String,
}

impl IntegrationFixture {
    fn new(harnesses: &[(&str, &str, i32)]) -> Self {
        let root = temporary_directory();
        let release = build_release(
            &root,
            env!("CARGO_PKG_VERSION"),
            "#!/bin/sh\nexit 1\n",
            false,
        );
        let binary = copied_binary(&root);
        let fake_bin = root.join("fake-bin");
        std::fs::create_dir_all(&fake_bin).unwrap();
        for (name, listing, update_exit) in harnesses {
            write_fake_harness(&fake_bin, name, listing, *update_exit);
        }
        let log = root.join("calls.log");
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        let path = fake_path(&fake_bin);
        let base_url = format!("file://{}", release.display());
        Self {
            root,
            binary,
            log,
            config,
            path,
            base_url,
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        run_binary(
            &self.binary,
            args,
            &[
                ("PATH", self.path.as_str()),
                ("FAKE_LOG", self.log.to_str().unwrap()),
                ("XDG_CONFIG_HOME", self.config.to_str().unwrap()),
                ("LEADLINE_BASE_URL", self.base_url.as_str()),
            ],
        )
    }
}

impl Drop for IntegrationFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn update_replaces_the_binary_with_the_latest_release() {
    if prerequisites_missing() {
        return;
    }
    let root = temporary_directory();
    let release = build_release(
        &root,
        "99.0.0",
        "#!/bin/sh\necho \"leadline 99.0.0\"\n",
        false,
    );
    let binary = copied_binary(&root);

    let output = run_update(&binary, &release);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("99.0.0"), "stdout: {stdout}");

    let version = spawn_output(binary_command(&binary).arg("--version"));
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "leadline 99.0.0"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn update_is_a_no_op_when_the_binary_is_current() {
    if prerequisites_missing() {
        return;
    }
    let root = temporary_directory();
    let release = build_release(
        &root,
        env!("CARGO_PKG_VERSION"),
        "#!/bin/sh\nexit 1\n",
        false,
    );
    let binary = copied_binary(&root);
    let before = std::fs::read(&binary).unwrap();

    let output = run_update(&binary, &release);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("is the latest release"));
    assert_eq!(std::fs::read(&binary).unwrap(), before);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn update_rejects_a_checksum_mismatch_without_touching_the_binary() {
    if prerequisites_missing() {
        return;
    }
    let root = temporary_directory();
    let release = build_release(
        &root,
        "99.0.0",
        "#!/bin/sh\necho \"leadline 99.0.0\"\n",
        true,
    );
    let binary = copied_binary(&root);
    let before = std::fs::read(&binary).unwrap();

    let output = run_update(&binary, &release);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"));
    assert_eq!(std::fs::read(&binary).unwrap(), before);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn integrations_refresh_every_detected_harness() {
    if prerequisites_missing() {
        return;
    }
    let fixture = IntegrationFixture::new(&[
        ("pi", PI_LISTING, 0),
        ("omp", OMP_LISTING, 0),
        ("claude", CLAUDE_LISTING, 0),
    ]);

    let output = fixture.run(&["update", "--integrations"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        recorded_calls(&fixture.log),
        vec![
            "pi list",
            "pi update git:github.com/jbt95/leadline",
            "omp plugin list --json",
            "omp plugin install git:github.com/jbt95/leadline --force",
            "claude plugin list --json",
            "claude plugin update leadline@leadline",
        ]
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Pi: integration refreshed"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("OMP: integration refreshed"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("Claude Code: integration refreshed"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("Restart required: Pi, OMP, Claude Code."),
        "stdout: {stdout}"
    );
}

#[test]
fn integrations_are_skipped_when_not_installed() {
    if prerequisites_missing() {
        return;
    }
    let fixture = IntegrationFixture::new(&[
        ("pi", "User packages:\n  npm:pi-web-access\n", 0),
        (
            "omp",
            r#"{"npm":[{"name":"ponytail"}],"marketplace":[]}"#,
            0,
        ),
        ("claude", "[]", 0),
    ]);

    let output = fixture.run(&["update", "--integrations"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        recorded_calls(&fixture.log),
        vec![
            "pi list",
            "omp plugin list --json",
            "claude plugin list --json"
        ]
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("integration refreshed"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("Restart required"), "stdout: {stdout}");
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn missing_harness_executables_are_skipped() {
    if prerequisites_missing() {
        return;
    }
    let root = temporary_directory();
    let release = build_release(
        &root,
        env!("CARGO_PKG_VERSION"),
        "#!/bin/sh\nexit 1\n",
        false,
    );
    let binary = copied_binary(&root);
    let config = root.join("config");
    std::fs::create_dir_all(&config).unwrap();
    let base_url = format!("file://{}", release.display());

    let output = run_binary(
        &binary,
        &["update", "--integrations"],
        &[
            ("PATH", "/usr/bin:/bin"),
            ("XDG_CONFIG_HOME", config.to_str().unwrap()),
            ("LEADLINE_BASE_URL", &base_url),
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("integration refreshed"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("Restart required"), "stdout: {stdout}");
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn integration_failures_do_not_stop_later_updates() {
    if prerequisites_missing() {
        return;
    }
    let fixture = IntegrationFixture::new(&[
        ("pi", PI_LISTING, 1),
        ("omp", OMP_LISTING, 0),
        ("claude", CLAUDE_LISTING, 0),
    ]);

    let output = fixture.run(&["update", "--integrations"]);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        recorded_calls(&fixture.log),
        vec![
            "pi list",
            "pi update git:github.com/jbt95/leadline",
            "omp plugin list --json",
            "omp plugin install git:github.com/jbt95/leadline --force",
            "claude plugin list --json",
            "claude plugin update leadline@leadline",
        ]
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("OMP: integration refreshed"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("Claude Code: integration refreshed"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("Restart required: OMP, Claude Code."),
        "stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Pi: pi failed to update the Leadline package"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("binary update succeeded; failed harness integrations: Pi"),
        "stderr: {stderr}"
    );
}

#[test]
fn malformed_probe_output_is_a_detection_failure() {
    if prerequisites_missing() {
        return;
    }
    let fixture = IntegrationFixture::new(&[
        ("pi", PI_LISTING, 0),
        ("omp", "not json", 0),
        ("claude", CLAUDE_LISTING, 0),
    ]);

    let output = fixture.run(&["update", "--integrations"]);

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("OMP: detection failed:"),
        "stderr: {stderr}"
    );
    assert_eq!(
        recorded_calls(&fixture.log),
        vec![
            "pi list",
            "pi update git:github.com/jbt95/leadline",
            "omp plugin list --json",
            "claude plugin list --json",
            "claude plugin update leadline@leadline",
        ]
    );
    assert!(
        stderr.contains("failed harness integrations: OMP"),
        "stderr: {stderr}"
    );
}

#[test]
fn claude_malformed_probe_output_is_a_detection_failure() {
    if prerequisites_missing() {
        return;
    }
    let fixture = IntegrationFixture::new(&[
        ("pi", PI_LISTING, 0),
        ("omp", OMP_LISTING, 0),
        ("claude", "not json", 0),
    ]);

    let output = fixture.run(&["update", "--integrations"]);

    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("OMP: integration refreshed"),
        "stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Claude Code: detection failed:"),
        "stderr: {stderr}"
    );
    assert_eq!(
        recorded_calls(&fixture.log),
        vec![
            "pi list",
            "pi update git:github.com/jbt95/leadline",
            "omp plugin list --json",
            "omp plugin install git:github.com/jbt95/leadline --force",
            "claude plugin list --json",
        ]
    );
    assert!(
        stderr.contains("binary update succeeded; failed harness integrations: Claude Code"),
        "stderr: {stderr}"
    );
}

#[test]
fn opencode_local_plugin_requires_manual_restart() {
    if prerequisites_missing() {
        return;
    }
    // The other harnesses get absent-integration fakes so the test is hermetic;
    // the `opencode2` fake records calls to prove it is never invoked.
    let fixture = IntegrationFixture::new(&[
        ("pi", "User packages:\n  npm:pi-web-access\n", 0),
        (
            "omp",
            r#"{"npm":[{"name":"ponytail"}],"marketplace":[]}"#,
            0,
        ),
        ("claude", "[]", 0),
        ("opencode2", "", 0),
    ]);
    let plugin = fixture.config.join("opencode/plugins/leadline");
    std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
    let checkout = fixture.root.join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(checkout.join("marker.txt"), "untouched").unwrap();
    std::os::unix::fs::symlink(&checkout, &plugin).unwrap();

    let output = fixture.run(&["update", "--integrations"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        recorded_calls(&fixture.log),
        vec![
            "pi list",
            "omp plugin list --json",
            "claude plugin list --json"
        ],
        "opencode2 must never be invoked"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("OpenCode: plugin is a local checkout"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("opencode2 service restart"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("Restart required: OpenCode."),
        "stdout: {stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(checkout.join("marker.txt")).unwrap(),
        "untouched"
    );
    assert!(plugin.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn plain_update_does_not_probe_harnesses() {
    if prerequisites_missing() {
        return;
    }
    let root = temporary_directory();
    let release = build_release(
        &root,
        env!("CARGO_PKG_VERSION"),
        "#!/bin/sh\nexit 1\n",
        false,
    );
    let binary = copied_binary(&root);
    let fake_bin = root.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).unwrap();
    write_fake_harness(&fake_bin, "pi", PI_LISTING, 0);
    let log = root.join("calls.log");
    let path = fake_path(&fake_bin);
    let base_url = format!("file://{}", release.display());

    let output = run_binary(
        &binary,
        &["update"],
        &[
            ("PATH", &path),
            ("FAKE_LOG", log.to_str().unwrap()),
            ("LEADLINE_BASE_URL", &base_url),
        ],
    );

    assert!(output.status.success());
    assert!(!log.exists(), "plain update probed a harness");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn plain_update_with_a_new_release_does_not_probe_harnesses() {
    if prerequisites_missing() {
        return;
    }
    let root = temporary_directory();
    let release = build_release(
        &root,
        "99.0.0",
        "#!/bin/sh\necho \"leadline 99.0.0\"\n",
        false,
    );
    let binary = copied_binary(&root);
    let fake_bin = root.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).unwrap();
    write_fake_harness(&fake_bin, "pi", PI_LISTING, 0);
    let log = root.join("calls.log");
    let path = fake_path(&fake_bin);
    let base_url = format!("file://{}", release.display());

    let output = run_binary(
        &binary,
        &["update"],
        &[
            ("PATH", &path),
            ("FAKE_LOG", log.to_str().unwrap()),
            ("LEADLINE_BASE_URL", &base_url),
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!log.exists(), "plain update probed a harness");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn spawn_retries_while_the_binary_is_busy() {
    // Holding the binary open for writing makes exec fail with `ETXTBSY` on
    // Linux; releasing it from another thread lets the retry succeed. (macOS
    // permits the first exec, so this passes there without retrying.)
    let root = temporary_directory();
    let binary = copied_binary(&root);
    let handle = std::fs::OpenOptions::new()
        .write(true)
        .open(&binary)
        .unwrap();
    let closer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(handle);
    });
    let output = spawn_output(binary_command(&binary).arg("--version"));
    closer.join().unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("leadline "),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn update_rejects_unknown_options() {
    let root = temporary_directory();
    let binary = copied_binary(&root);

    let output = run_binary(&binary, &["update", "--bogus"], &[]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown update option '--bogus'"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(&root).unwrap();
}
