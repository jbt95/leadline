//! Shared integration-test helpers: temporary directories, file writers, and
//! tool probes.
//!
//! Every helper stays offline and deterministic; directory names carry the
//! process id plus a per-binary atomic counter so concurrently running test
//! targets never collide.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// Fresh empty directory under the system temp dir.
#[allow(dead_code)]
pub fn temporary_directory() -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-test-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// Command for the built binary with the ambient local-metrics store switched
/// off: a test run must never write into the developer's
/// `LEADLINE_METRICS_DIR`. Tests that assert recording set the variable
/// themselves.
#[allow(dead_code)]
pub fn leadline() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_leadline"));
    command.env_remove("LEADLINE_METRICS_DIR");
    command
}

/// True when `program` resolves to a runnable executable.
#[allow(dead_code)]
pub fn tool_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Write `contents` to `dir/name` and return the full path.
#[allow(dead_code)]
pub fn write_temp(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

/// Write `body` to `path` and mark it executable (Unix-only).
#[cfg(unix)]
#[allow(dead_code)]
pub fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}
