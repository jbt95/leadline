//! Hardened subprocess seam for read-only Git access.

use crate::{Result, strip_verbatim_prefix};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Rejects revisions that could be interpreted as options or object paths.
pub fn validate_revision(revision: &str) -> Result<()> {
    if revision.is_empty()
        || revision.starts_with('-')
        || revision.contains(':')
        || revision.chars().any(char::is_control)
    {
        return Err("base revision contains unsupported characters".into());
    }
    Ok(())
}

/// Runs a mandatory Git command and returns its complete output.
pub fn run(cwd: &Path, args: &[&str]) -> Result<Output> {
    let output = command(cwd, args).output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(command_error(&output))
    }
}

/// Runs a Git command that is optional only when Git itself is unavailable.
pub fn run_optional(cwd: &Path, args: &[&str]) -> Result<Option<Vec<u8>>> {
    run_optional_when(cwd, args, |_| false)
}

/// Finds and canonicalizes the repository containing `start`.
pub fn repo_root(start: &Path) -> Result<Option<PathBuf>> {
    let has_marker = has_git_marker(start)?;
    let Some(bytes) = run_optional_when(start, &["rev-parse", "--show-toplevel"], |output| {
        !has_marker
            && output.status.code() == Some(128)
            && String::from_utf8_lossy(&output.stderr).contains("not a git repository")
    })?
    else {
        return Ok(None);
    };
    let path = String::from_utf8(bytes)?;
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(strip_verbatim_prefix(&std::fs::canonicalize(path)?)))
}

/// Resolves a revision to its commit ID and commit timestamp.
pub fn resolve_commit(root: &Path, revision: &str) -> Result<(String, i64)> {
    validate_revision(revision)?;
    let peeled = format!("{revision}^{{commit}}");
    let resolved = run(
        root,
        &["rev-parse", "--verify", "--end-of-options", &peeled],
    )?;
    let commit = parse_hash_record(&resolved.stdout)?;
    let metadata = run(
        root,
        &[
            "show",
            "-s",
            "--no-show-signature",
            "--no-notes",
            "--format=%H%x00%ct",
            &commit,
            "--",
        ],
    )?;
    let text = one_record(&metadata.stdout, "git returned malformed commit metadata")?;
    let Some((reported_commit, timestamp)) = text.split_once('\0') else {
        return Err("git returned malformed commit metadata".into());
    };
    if reported_commit != commit || timestamp.contains('\0') {
        return Err("git returned malformed commit metadata".into());
    }
    let timestamp = timestamp
        .parse()
        .map_err(|_| "git returned an invalid commit timestamp")?;
    Ok((commit, timestamp))
}

pub(crate) fn read_revision_blob_optional(
    root: &Path,
    revision: &str,
    path: &str,
) -> Result<Option<Vec<u8>>> {
    let entry = run(
        root,
        &["--literal-pathspecs", "ls-tree", "-z", revision, "--", path],
    )?;
    if entry.stdout.is_empty() {
        return Ok(None);
    }
    run(root, &["show", &format!("{revision}:{path}")]).map(|output| Some(output.stdout))
}

pub(crate) fn read_index_blob_optional(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
    let entry = run(
        root,
        &[
            "--literal-pathspecs",
            "ls-files",
            "--stage",
            "-z",
            "--",
            path,
        ],
    )?;
    if entry.stdout.is_empty() || index_entry_has_zero_oid(&entry.stdout)? {
        return Ok(None);
    }
    run(root, &["show", &format!(":{path}")]).map(|output| Some(output.stdout))
}

pub(crate) fn repository_head_optional(cwd: &Path) -> Result<Option<Vec<u8>>> {
    let has_marker = has_git_marker(cwd)?;
    run_optional_when(
        cwd,
        &[
            "log",
            "-1",
            "--no-show-signature",
            "--no-notes",
            "--format=%H%x00%ct",
        ],
        |output| {
            if output.status.code() != Some(128) {
                return false;
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            stderr.contains("does not have any commits yet")
                || stderr.contains("bad default revision 'HEAD'")
                || (!has_marker && stderr.contains("not a git repository"))
        },
    )
}

fn has_git_marker(start: &Path) -> Result<bool> {
    let start = if start.is_dir() {
        start
    } else {
        start.parent().unwrap_or(Path::new("."))
    };
    for ancestor in start.ancestors() {
        match std::fs::symlink_metadata(ancestor.join(".git")) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

fn run_optional_when(
    cwd: &Path,
    args: &[&str],
    expected_absence: impl FnOnce(&Output) -> bool,
) -> Result<Option<Vec<u8>>> {
    match command(cwd, args).output() {
        Ok(output) if output.status.success() => Ok(Some(output.stdout)),
        Ok(output) if expected_absence(&output) => Ok(None),
        Ok(output) => Err(command_error(&output)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn command_error(output: &Output) -> crate::Error {
    String::from_utf8_lossy(&output.stderr)
        .trim()
        .to_owned()
        .into()
}

fn parse_hash_record(bytes: &[u8]) -> Result<String> {
    let hash = one_record(bytes, "git returned malformed resolved commit")?;
    if !matches!(hash.len(), 40 | 64) || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("git returned malformed resolved commit".into());
    }
    Ok(hash.to_owned())
}

fn one_record<'a>(bytes: &'a [u8], error: &'static str) -> Result<&'a str> {
    let text = std::str::from_utf8(bytes)?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() || text.contains(['\n', '\r']) {
        return Err(error.into());
    }
    Ok(text)
}

fn index_entry_has_zero_oid(bytes: &[u8]) -> Result<bool> {
    let metadata = bytes
        .split(|byte| *byte == b'\t')
        .next()
        .ok_or("git returned malformed index entry")?;
    let metadata = std::str::from_utf8(metadata)?;
    let oid = metadata
        .split_whitespace()
        .nth(1)
        .ok_or("git returned malformed index entry")?;
    Ok(!oid.is_empty() && oid.bytes().all(|byte| byte == b'0'))
}

pub(crate) fn command(cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .args(args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null());
    command
}
