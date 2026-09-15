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

fn run_update(binary: &Path, release: &Path) -> std::process::Output {
    Command::new(binary)
        .arg("update")
        .env("LEADLINE_BASE_URL", format!("file://{}", release.display()))
        .output()
        .unwrap()
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

    let version = Command::new(&binary).arg("--version").output().unwrap();
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
