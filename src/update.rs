//! Self-update: fetch the latest release, verify it, replace this binary.
//!
//! This is the only networked code path in the project and it runs only for
//! the explicit `leadline update` command. Everything else stays offline.

use crate::Result;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Default download origin; override with `LEADLINE_BASE_URL` (same variable
/// the installer uses).
pub const DEFAULT_BASE_URL: &str = "https://github.com/jbt95/leadline";

/// A parsed `X.Y.Z` release version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl Version {
    /// Parses `X.Y.Z`, tolerating a leading `v` and surrounding whitespace.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        parts.next().is_none().then_some(Self(major, minor, patch))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Release archive name for a platform, or `None` when unsupported.
pub fn archive_for(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("macos", "aarch64") => "leadline-aarch64-apple-darwin.tar.gz",
        ("macos", "x86_64") => "leadline-x86_64-apple-darwin.tar.gz",
        ("linux", "aarch64") => "leadline-aarch64-unknown-linux-gnu.tar.gz",
        ("linux", "x86_64") => "leadline-x86_64-unknown-linux-gnu.tar.gz",
        ("windows", "x86_64") => "leadline-x86_64-pc-windows-msvc.zip",
        _ => return None,
    })
}

/// Release archive name for the running platform.
pub fn target_archive() -> Result<&'static str> {
    archive_for(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        format!(
            "unsupported platform {}/{}: download a release archive from {DEFAULT_BASE_URL}/releases",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
        .into()
    })
}

/// Finds the SHA256 for `archive` in a `SHA256SUMS` file.
///
/// Lines are `<hash>  <path>`; binary-mode markers (`*`) are tolerated.
pub fn checksum_for(archive: &str, sums: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let name = fields.next()?;
        let name = name.strip_prefix('*').unwrap_or(name);
        if name != archive || !is_sha256(hash) {
            return None;
        }
        Some(hash.to_ascii_lowercase())
    })
}

fn is_sha256(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Runs a system tool with a fixed argument list, no shell, and no stdin.
fn output_of(program: &str, args: &[&str]) -> std::io::Result<Output> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env("LANG", "C");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.output()
}

/// Requires success and returns stdout, or an error naming the tool.
fn stdout_of(program: &str, output: &Output, action: &str) -> Result<String> {
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let reason = String::from_utf8_lossy(&output.stderr);
    let reason = reason.lines().next().unwrap_or("").trim();
    if reason.is_empty() {
        Err(format!("{program} failed to {action}").into())
    } else {
        Err(format!("{program} failed to {action}: {reason}").into())
    }
}

/// Runs the first available tool from `candidates` and returns its stdout.
///
/// A missing tool falls through to the next candidate; a tool that runs and
/// fails is an error naming it. When every candidate is missing, `missing`
/// is the error.
fn output_of_first(candidates: &[(&str, &[&str])], action: &str, missing: &str) -> Result<String> {
    for &(program, args) in candidates {
        match output_of(program, args) {
            Ok(output) => return stdout_of(program, &output, action),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot run {program}: {error}").into()),
        }
    }
    Err(missing.into())
}

/// Downloads `url` to `dest` with `curl`, falling back to `wget`.
pub fn download(url: &str, dest: &Path) -> Result<()> {
    let dest = dest
        .to_str()
        .ok_or("download destination is not valid UTF-8")?;
    let curl_args: &[&str] = &["-fsSL", url, "-o", dest];
    let wget_args: &[&str] = &["-q", url, "-O", dest];
    output_of_first(
        &[("curl", curl_args), ("wget", wget_args)],
        "download the release",
        "curl or wget is required to download the release",
    )
    .map(|_| ())
}

/// Returns the lowercase SHA256 of `path` using the first available tool.
pub fn sha256_file(path: &Path) -> Result<String> {
    let path = path.to_str().ok_or("checksum path is not valid UTF-8")?;
    #[cfg(unix)]
    {
        let sha256sum_args: &[&str] = &[path];
        let shasum_args: &[&str] = &["-a", "256", path];
        output_of_first(
            &[("sha256sum", sha256sum_args), ("shasum", shasum_args)],
            "hash the release archive",
            "sha256sum or shasum is required to verify the download",
        )
        .and_then(|stdout| first_hash(&stdout))
    }
    #[cfg(windows)]
    {
        let script = format!(
            "(Get-FileHash -Algorithm SHA256 -LiteralPath {}).Hash",
            ps_quote(path)?
        );
        let sha256sum_args: &[&str] = &[path];
        let shasum_args: &[&str] = &["-a", "256", path];
        let powershell_args: &[&str] = &["-NoProfile", "-Command", &script];
        output_of_first(
            &[
                ("sha256sum", sha256sum_args),
                ("shasum", shasum_args),
                ("powershell", powershell_args),
            ],
            "hash the release archive",
            "sha256sum or shasum is required to verify the download",
        )
        .and_then(|stdout| first_hash(&stdout))
    }
}

fn first_hash(stdout: &str) -> Result<String> {
    let hash = stdout
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if is_sha256(&hash) {
        Ok(hash)
    } else {
        Err("checksum tool returned an unexpected value".into())
    }
}

/// Extracts `archive` into `dest` (Unix `tar`, Windows PowerShell).
pub fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let archive = archive.to_str().ok_or("archive path is not valid UTF-8")?;
    let dest = dest
        .to_str()
        .ok_or("extraction directory is not valid UTF-8")?;
    #[cfg(unix)]
    {
        let tar_args: &[&str] = &["-xzf", archive, "-C", dest];
        output_of_first(
            &[("tar", tar_args)],
            "extract the release archive",
            "tar is required to extract the release archive",
        )
        .map(|_| ())
    }
    #[cfg(windows)]
    {
        let script = format!(
            "Expand-Archive -LiteralPath {} -DestinationPath {} -Force",
            ps_quote(archive)?,
            ps_quote(dest)?
        );
        let powershell_args: &[&str] = &["-NoProfile", "-Command", &script];
        output_of_first(
            &[("powershell", powershell_args)],
            "extract the release archive",
            "powershell is required to extract the release archive",
        )
        .map(|_| ())
    }
}

/// Quotes a path for a PowerShell `-Command` string. Our paths are generated
/// temporary names; refusing a quote is safer than escaping one.
#[cfg(windows)]
fn ps_quote(text: &str) -> Result<String> {
    if text.contains('\'') {
        return Err("temporary path contains a single quote".into());
    }
    Ok(format!("'{text}'"))
}

/// Replaces `target` with `new_binary`.
///
/// Unix: stage a sibling copy and rename it over the target (atomic, same
/// filesystem). Windows: a running `.exe` cannot be overwritten, so the
/// target is renamed aside, the new binary is copied in, and the old file is
/// deleted best-effort (a leftover `.old` is removed on the next run).
pub fn replace_at(new_binary: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = target
            .parent()
            .ok_or("cannot determine the executable directory")?;
        let file_name = target
            .file_name()
            .ok_or("cannot determine the executable name")?;
        let staging = directory.join(format!(
            ".{}.new.{}",
            file_name.to_string_lossy(),
            std::process::id()
        ));
        let outcome = (|| -> Result<()> {
            std::fs::copy(new_binary, &staging)?;
            std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(&staging, target)?;
            Ok(())
        })();
        if outcome.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
        outcome
    }
    #[cfg(windows)]
    {
        let backup = backup_path(target);
        // A leftover from a previous failed update would block the rename.
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(target, &backup)?;
        match std::fs::copy(new_binary, target) {
            Ok(_) => {
                let _ = std::fs::remove_file(&backup);
                Ok(())
            }
            Err(error) => {
                let _ = std::fs::rename(&backup, target);
                Err(error.into())
            }
        }
    }
}

#[cfg(windows)]
fn backup_path(target: &Path) -> std::path::PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    target.with_file_name(name)
}

/// Removes a leftover `.old` binary from a previous update (Windows only).
#[cfg(windows)]
pub fn cleanup_stale_backup() {
    if let Ok(target) = std::env::current_exe() {
        remove_backup_for(&target);
    }
}

#[cfg(windows)]
fn remove_backup_for(exe: &Path) {
    let _ = std::fs::remove_file(backup_path(exe));
}

/// Binary name inside the release archive.
const BINARY_NAME: &str = if cfg!(windows) {
    "leadline.exe"
} else {
    "leadline"
};

/// What `run` did.
#[derive(Debug)]
pub enum Outcome {
    /// The running version is the latest release.
    Current(Version),
    /// The binary was replaced.
    Updated {
        from: Version,
        to: Version,
        path: PathBuf,
    },
}

/// Runs the full update flow against `base_url`.
pub fn run(base_url: &str) -> Result<Outcome> {
    let base_url = base_url.trim_end_matches('/');
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .ok_or("compiled version is not an X.Y.Z version")?;
    let directory = TempDir::new()?;

    let version_url = format!("{base_url}/releases/latest/download/VERSION");
    let version_path = directory.path().join("VERSION");
    download(&version_url, &version_path)?;
    let latest_text = std::fs::read_to_string(&version_path)?;
    let latest =
        Version::parse(&latest_text).ok_or("the latest release VERSION is not an X.Y.Z version")?;
    if latest <= current {
        return Ok(Outcome::Current(current));
    }

    let archive = target_archive()?;
    let sums_path = directory.path().join("SHA256SUMS");
    download(
        &format!("{base_url}/releases/latest/download/SHA256SUMS"),
        &sums_path,
    )?;
    let archive_path = directory.path().join(archive);
    download(
        &format!("{base_url}/releases/latest/download/{archive}"),
        &archive_path,
    )?;

    let sums = std::fs::read_to_string(&sums_path)?;
    let expected = checksum_for(archive, &sums)
        .ok_or_else(|| format!("{archive} is not listed in SHA256SUMS"))?;
    let actual = sha256_file(&archive_path)?;
    if expected != actual {
        return Err(format!(
            "checksum mismatch for {archive}\nexpected {expected}\nactual   {actual}"
        )
        .into());
    }

    let extracted = directory.path().join("extracted");
    std::fs::create_dir_all(&extracted)?;
    extract(&archive_path, &extracted)?;
    let new_binary = extracted.join(BINARY_NAME);
    if !new_binary.is_file() {
        return Err("the release archive does not contain the leadline binary".into());
    }

    let target = std::env::current_exe()?;
    replace_at(&new_binary, &target).map_err(|error| {
        format!(
            "cannot replace {}: {error}; reinstall with install.sh or from {DEFAULT_BASE_URL}/releases",
            target.display()
        )
    })?;

    let output = Command::new(&target)
        .arg("--version")
        .stdin(Stdio::null())
        .output()?;
    let installed = if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .trim()
            .strip_prefix("leadline ")
            .and_then(Version::parse)
    } else {
        None
    }
    .ok_or_else(|| {
        format!(
            "the updated binary failed its self-check; reinstall from {DEFAULT_BASE_URL}/releases"
        )
    })?;

    Ok(Outcome::Updated {
        from: current,
        to: installed,
        path: target,
    })
}

/// Private temporary directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("leadline-update-{}-{unique}", std::process::id()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        }
        #[cfg(windows)]
        {
            std::fs::create_dir_all(&path)?;
        }
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_with_prefix_and_whitespace() {
        assert_eq!(Version::parse("1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse(" v0.5.3\n"), Some(Version(0, 5, 3)));
        assert_eq!(Version::parse("10.200.3000"), Some(Version(10, 200, 3000)));
    }

    #[test]
    fn rejects_malformed_versions() {
        for text in ["", "1", "1.2", "1.2.3.4", "1.2.x", "v", "1..2", "1.2.3-rc1"] {
            assert_eq!(Version::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn orders_versions_numerically() {
        assert!(Version::parse("1.10.0").unwrap() > Version::parse("1.9.9").unwrap());
        assert!(Version::parse("2.0.0").unwrap() > Version::parse("1.99.99").unwrap());
        assert_eq!(
            Version::parse("1.2.3").unwrap(),
            Version::parse("v1.2.3").unwrap()
        );
    }

    #[test]
    fn displays_versions() {
        assert_eq!(Version(0, 5, 3).to_string(), "0.5.3");
    }

    #[test]
    fn maps_platforms_to_release_archives() {
        assert_eq!(
            archive_for("macos", "aarch64"),
            Some("leadline-aarch64-apple-darwin.tar.gz")
        );
        assert_eq!(
            archive_for("macos", "x86_64"),
            Some("leadline-x86_64-apple-darwin.tar.gz")
        );
        assert_eq!(
            archive_for("linux", "aarch64"),
            Some("leadline-aarch64-unknown-linux-gnu.tar.gz")
        );
        assert_eq!(
            archive_for("linux", "x86_64"),
            Some("leadline-x86_64-unknown-linux-gnu.tar.gz")
        );
        assert_eq!(
            archive_for("windows", "x86_64"),
            Some("leadline-x86_64-pc-windows-msvc.zip")
        );
        assert_eq!(archive_for("linux", "riscv64"), None);
    }

    #[test]
    fn finds_checksum_lines() {
        let archive = "leadline-aarch64-apple-darwin.tar.gz";
        let other = "leadline-x86_64-unknown-linux-gnu.tar.gz";
        let upper = "A".repeat(64);
        let expected = "a".repeat(64);
        let sums = format!(
            "{}  {other}\n{upper}  *{archive}\n{}  SHA256SUMS\n",
            "b".repeat(64),
            "c".repeat(64)
        );
        assert_eq!(checksum_for(archive, &sums), Some(expected));
        assert_eq!(checksum_for(other, &sums), Some("b".repeat(64)));
        assert_eq!(checksum_for("missing.tar.gz", &sums), None);
    }

    #[test]
    fn ignores_directory_prefixed_entries() {
        let archive = "leadline-aarch64-apple-darwin.tar.gz";
        let sums = format!("{}  dist/{archive}\n", "a".repeat(64));
        assert_eq!(checksum_for(archive, &sums), None);
    }

    #[test]
    fn rejects_checksums_that_are_not_sha256() {
        let archive = "leadline-aarch64-apple-darwin.tar.gz";
        assert_eq!(checksum_for(archive, &format!("abc  {archive}\n")), None);
        assert_eq!(
            checksum_for(archive, &format!("{}  {archive}\n", "z".repeat(64))),
            None
        );
    }

    fn test_directory(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "leadline-update-unit-{label}-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// True when an error reports that a required external tool is absent.
    fn missing_tool(error: &crate::Error) -> bool {
        error.to_string().contains("is required")
    }

    #[test]
    fn hashes_a_known_file() {
        let dir = test_directory("hash");
        let file = dir.join("archive.tar.gz");
        std::fs::write(&file, b"leadline-update-test\n").unwrap();
        match sha256_file(&file) {
            Ok(hash) => assert_eq!(
                hash,
                "8dde6f74b5006cbeba6856f93407c0c45182e3d8c6b3e6dd8d1e4ee5d40d69d4"
            ),
            Err(error) if missing_tool(&error) => eprintln!("skipping: no checksum tool"),
            Err(error) => panic!("{error}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn downloads_a_file_url() {
        let dir = test_directory("download");
        let source = dir.join("VERSION");
        std::fs::write(&source, b"v9.9.9\n").unwrap();
        let dest = dir.join("VERSION.copy");
        match download(&format!("file://{}", source.display()), &dest) {
            Ok(()) => assert_eq!(std::fs::read(&dest).unwrap(), b"v9.9.9\n"),
            Err(error) if missing_tool(&error) => eprintln!("skipping: no curl or wget"),
            Err(error) => panic!("{error}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn extracts_a_tar_archive() {
        let dir = test_directory("extract");
        let payload = dir.join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(payload.join("leadline"), b"payload").unwrap();
        let archive = dir.join("release.tar.gz");
        let created = match Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg("leadline")
            .status()
        {
            Ok(status) => status.success(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("skipping: no tar");
                std::fs::remove_dir_all(&dir).unwrap();
                return;
            }
            Err(error) => panic!("cannot run tar: {error}"),
        };
        assert!(created);
        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        match extract(&archive, &dest) {
            Ok(()) => assert_eq!(std::fs::read(dest.join("leadline")).unwrap(), b"payload"),
            Err(error) if missing_tool(&error) => eprintln!("skipping: no tar"),
            Err(error) => panic!("{error}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn quotes_powershell_paths() {
        assert_eq!(
            ps_quote("C:\\Temp\\leadline.exe").unwrap(),
            "'C:\\Temp\\leadline.exe'"
        );
        assert!(ps_quote("C:\\Temp\\o'brien").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn backs_up_beside_the_executable() {
        let target = Path::new("C:\\Tools\\leadline.exe");
        assert_eq!(
            backup_path(target).to_str().unwrap(),
            "C:\\Tools\\leadline.exe.old"
        );
    }

    #[cfg(windows)]
    #[test]
    fn removes_a_stale_backup() {
        let dir = test_directory("stale-backup");
        let exe = dir.join("leadline.exe");
        std::fs::write(&exe, b"exe").unwrap();
        let backup = backup_path(&exe);
        std::fs::write(&backup, b"old").unwrap();
        remove_backup_for(&exe);
        assert!(!backup.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn extracts_a_zip_archive() {
        let dir = test_directory("extract-zip");
        let payload = dir.join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        let file = payload.join("leadline.exe");
        std::fs::write(&file, b"payload").unwrap();
        let archive = dir.join("release.zip");
        let script = format!(
            "Compress-Archive -LiteralPath {} -DestinationPath {} -Force",
            ps_quote(file.to_str().unwrap()).unwrap(),
            ps_quote(archive.to_str().unwrap()).unwrap()
        );
        assert!(
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &script])
                .status()
                .unwrap()
                .success()
        );
        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        extract(&archive, &dest).unwrap();
        assert_eq!(
            std::fs::read(dest.join("leadline.exe")).unwrap(),
            b"payload"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replaces_a_file_in_place() {
        let dir = test_directory("replace");
        let target = dir.join("leadline");
        std::fs::write(&target, b"old").unwrap();
        let fresh = dir.join("fresh");
        std::fs::write(&fresh, b"new").unwrap();
        replace_at(&fresh, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_replacement_leaves_the_target_untouched() {
        let dir = test_directory("replace-failure");
        let target = dir.join("leadline");
        std::fs::write(&target, b"old").unwrap();
        assert!(replace_at(&dir.join("missing"), &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
