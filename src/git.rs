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
        Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_owned()
            .into())
    }
}

/// Runs a Git command whose non-success result means the requested data is absent.
pub fn run_optional(cwd: &Path, args: &[&str]) -> Result<Option<Vec<u8>>> {
    match command(cwd, args).output() {
        Ok(output) if output.status.success() => Ok(Some(output.stdout)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Finds and canonicalizes the repository containing `start`.
pub fn repo_root(start: &Path) -> Result<Option<PathBuf>> {
    let Some(bytes) = run_optional(start, &["rev-parse", "--show-toplevel"])? else {
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
    let output = run(
        root,
        &[
            "show",
            "-s",
            "--no-show-signature",
            "--no-notes",
            "--format=%H%x00%ct",
            revision,
            "--",
        ],
    )?;
    let text = String::from_utf8(output.stdout)?;
    let Some((commit, timestamp)) = text.trim().split_once('\0') else {
        return Err("git returned malformed commit metadata".into());
    };
    let timestamp = timestamp
        .trim()
        .parse()
        .map_err(|_| "git returned an invalid commit timestamp")?;
    Ok((commit.to_owned(), timestamp))
}

pub(crate) fn command(cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .args(args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null());
    command
}
