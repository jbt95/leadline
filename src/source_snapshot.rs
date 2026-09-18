//! Read-only source snapshots from worktrees, the index, or revisions.
//!
//! A snapshot enumerates supported source files, loads their bytes from the
//! selected target, and records the target's commit metadata. Object-backed
//! targets stream every blob through one `git cat-file --batch` process.
//! Snapshots never check out, reset, stash, fetch, or write repository state.

use crate::config::{Config, parse_str};
use crate::discovery::SourceFilter;
use crate::git;
use crate::{Result, normalize_path};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const CONFIG_FILE: &str = "leadline.toml";
const MAILMAP_FILE: &str = ".mailmap";

/// Which repository state a snapshot reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotTarget {
    /// Files on disk in the working tree.
    Worktree,
    /// Staged blobs in the index.
    Index,
    /// Blobs in a Git revision, resolved to a commit.
    Revision(String),
}

/// One supported source file with an analysis-root-relative path and bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceEntry {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// Where a snapshot was read from and which configuration governs it.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotContext {
    pub repo_root: Option<PathBuf>,
    pub analysis_root: PathBuf,
    pub scope_prefix: String,
    pub config: Config,
}

/// Rendered repository state with target metadata and source entries.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceSnapshot {
    pub target: SnapshotTarget,
    pub commit: Option<String>,
    pub commit_timestamp: Option<i64>,
    pub config_bytes: Option<Vec<u8>>,
    pub mailmap_bytes: Option<Vec<u8>>,
    pub entries: Vec<SourceEntry>,
}

/// Loads a source snapshot and its context from `path`.
///
/// `path` is a directory or an explicit supported file. Git-backed targets
/// resolve `HEAD` (or the requested revision) and enumerate repository paths;
/// outside Git only [`SnapshotTarget::Worktree`] is available and discovery
/// falls back to the filesystem.
pub fn load(path: &Path, target: SnapshotTarget) -> Result<(SnapshotContext, SourceSnapshot)> {
    let explicit_file = path.is_file();
    if !explicit_file && !path.is_dir() {
        return Err(std::io::Error::new(
            ErrorKind::NotFound,
            format!("analysis path does not exist: {}", path.display()),
        )
        .into());
    }
    let anchor = if explicit_file {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    } else {
        path
    };
    let repo_root = git::repo_root(anchor)?;
    // `std::fs::canonicalize` returns verbatim (`\\?\`) paths on Windows;
    // git prints plain paths, so both sides strip before comparing.
    let analysis_root = crate::strip_verbatim_prefix(&std::fs::canonicalize(anchor)?);
    let scope_prefix = scope_prefix_for(&analysis_root, repo_root.as_deref())?;
    let explicit = if explicit_file {
        if crate::parser::detect_language(&path.to_string_lossy()).is_none() {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "unsupported source extension",
            )
            .into());
        }
        Some(LocatedFile {
            path: path.to_owned(),
            name: source_file_name(path)?,
        })
    } else {
        None
    };

    let parts = match &target {
        SnapshotTarget::Worktree => {
            worktree_parts(&repo_root, &analysis_root, &scope_prefix, explicit.as_ref())?
        }
        SnapshotTarget::Index => {
            index_parts(&repo_root, &analysis_root, &scope_prefix, explicit.as_ref())?
        }
        SnapshotTarget::Revision(revision) => revision_parts(
            &repo_root,
            &analysis_root,
            &scope_prefix,
            revision,
            explicit.as_ref(),
        )?,
    };
    let config = parse_config(parts.config_bytes.as_deref())?;
    let context = SnapshotContext {
        repo_root,
        analysis_root,
        scope_prefix,
        config,
    };
    let snapshot = SourceSnapshot {
        target,
        commit: parts.commit,
        commit_timestamp: parts.commit_timestamp,
        config_bytes: parts.config_bytes,
        mailmap_bytes: parts.mailmap_bytes,
        entries: parts.entries,
    };
    Ok((context, snapshot))
}

struct LocatedFile {
    path: PathBuf,
    name: String,
}

struct SnapshotParts {
    commit: Option<String>,
    commit_timestamp: Option<i64>,
    config_bytes: Option<Vec<u8>>,
    mailmap_bytes: Option<Vec<u8>>,
    entries: Vec<SourceEntry>,
}

fn worktree_parts(
    repo_root: &Option<PathBuf>,
    analysis_root: &Path,
    scope_prefix: &str,
    explicit: Option<&LocatedFile>,
) -> Result<SnapshotParts> {
    let head = match repo_root {
        Some(root) => head_record(root)?,
        None => None,
    };
    let config_bytes = read_worktree_file(&analysis_root.join(CONFIG_FILE))?;
    let config = parse_config(config_bytes.as_deref())?;
    let entries = match explicit {
        Some(file) => vec![SourceEntry {
            path: file.name.clone(),
            bytes: std::fs::read(&file.path)?,
        }],
        None => match repo_root {
            Some(root) => git_worktree_entries(root, analysis_root, scope_prefix, &config)?,
            None => filesystem_entries(analysis_root, &config)?,
        },
    };
    let mailmap_bytes = read_worktree_file(&analysis_root.join(MAILMAP_FILE))?;
    Ok(SnapshotParts {
        commit: head.as_ref().map(|(commit, _)| commit.clone()),
        commit_timestamp: head.map(|(_, timestamp)| timestamp),
        config_bytes,
        mailmap_bytes,
        entries,
    })
}

fn index_parts(
    repo_root: &Option<PathBuf>,
    analysis_root: &Path,
    scope_prefix: &str,
    explicit: Option<&LocatedFile>,
) -> Result<SnapshotParts> {
    let root = require_git(repo_root, "index")?;
    let head = head_record(root)?;
    let plan = index_plan(root, scope_prefix, explicit)?;
    let (config_bytes, mailmap_bytes, entries) =
        load_objects(root, analysis_root, plan, explicit.is_some())?;
    Ok(SnapshotParts {
        commit: head.as_ref().map(|(commit, _)| commit.clone()),
        commit_timestamp: head.map(|(_, timestamp)| timestamp),
        config_bytes,
        mailmap_bytes,
        entries,
    })
}

fn revision_parts(
    repo_root: &Option<PathBuf>,
    analysis_root: &Path,
    scope_prefix: &str,
    revision: &str,
    explicit: Option<&LocatedFile>,
) -> Result<SnapshotParts> {
    let root = require_git(repo_root, "revision")?;
    let (commit, commit_timestamp) = git::resolve_commit(root, revision)?;
    let plan = revision_plan(root, &commit, scope_prefix, explicit)?;
    let (config_bytes, mailmap_bytes, entries) =
        load_objects(root, analysis_root, plan, explicit.is_some())?;
    Ok(SnapshotParts {
        commit: Some(commit),
        commit_timestamp: Some(commit_timestamp),
        config_bytes,
        mailmap_bytes,
        entries,
    })
}

fn require_git<'a>(repo_root: &'a Option<PathBuf>, target: &str) -> Result<&'a Path> {
    match repo_root {
        Some(root) => Ok(root),
        None => Err(format!("{target} snapshots require a Git repository").into()),
    }
}

fn scope_prefix_for(analysis_root: &Path, repo_root: Option<&Path>) -> Result<String> {
    let Some(root) = repo_root else {
        return Ok(String::new());
    };
    let relative = analysis_root
        .strip_prefix(root)
        .map_err(|_| "analysis path is outside the Git repository")?;
    let mut prefix = normalize_path(relative);
    if !prefix.is_empty() {
        prefix.push('/');
    }
    Ok(prefix)
}

fn source_file_name(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("source path is not valid UTF-8")?;
    validate_text(name)?;
    Ok(name.to_owned())
}

fn validate_text(text: &str) -> Result<()> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return Err("path contains unsupported control characters".into());
    }
    Ok(())
}

fn validate_path(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "git returned a non-UTF-8 path")?;
    validate_text(text)?;
    Ok(text.to_owned())
}

fn strip_scope<'a>(scope_prefix: &str, path: &'a str) -> Option<&'a str> {
    if scope_prefix.is_empty() {
        Some(path)
    } else {
        path.strip_prefix(scope_prefix)
    }
}

fn is_hex_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn head_record(root: &Path) -> Result<Option<(String, i64)>> {
    match git::repository_head_optional(root)? {
        Some(bytes) => Ok(Some(parse_head_record(&bytes)?)),
        None => Ok(None),
    }
}

fn parse_head_record(bytes: &[u8]) -> Result<(String, i64)> {
    let text = std::str::from_utf8(bytes).map_err(|_| "git returned malformed HEAD metadata")?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let Some((commit, timestamp)) = text.split_once('\0') else {
        return Err("git returned malformed HEAD metadata".into());
    };
    if !is_hex_oid(commit) {
        return Err("git returned malformed HEAD metadata".into());
    }
    let timestamp = timestamp
        .parse()
        .map_err(|_| "git returned an invalid commit timestamp")?;
    Ok((commit.to_owned(), timestamp))
}

fn parse_config(bytes: Option<&[u8]>) -> Result<Config> {
    match bytes {
        Some(bytes) => {
            let text =
                std::str::from_utf8(bytes).map_err(|_| "leadline.toml is not valid UTF-8")?;
            Ok(parse_str(text)?)
        }
        None => Ok(Config::default()),
    }
}

fn read_worktree_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(std::fs::read(path)?)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn git_worktree_entries(
    root: &Path,
    analysis_root: &Path,
    scope_prefix: &str,
    config: &Config,
) -> Result<Vec<SourceEntry>> {
    let cached = git::run(root, &["ls-files", "--cached", "-z"])?;
    let untracked = git::run(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut candidates = z_paths(&cached.stdout)?;
    candidates.extend(z_paths(&untracked.stdout)?);
    candidates.sort();
    candidates.dedup();

    let filter = SourceFilter::new(analysis_root, &config.analysis_excludes)?;
    let mut entries = Vec::new();
    for repo_path in candidates {
        let Some(relative) = strip_scope(scope_prefix, &repo_path) else {
            continue;
        };
        if relative.is_empty() || !filter.accepts_file(Path::new(relative)) {
            continue;
        }
        let file = analysis_root.join(relative);
        match std::fs::symlink_metadata(&file) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => continue,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        entries.push(SourceEntry {
            path: relative.to_owned(),
            bytes: std::fs::read(&file)?,
        });
    }
    Ok(entries)
}

fn filesystem_entries(analysis_root: &Path, config: &Config) -> Result<Vec<SourceEntry>> {
    let files = crate::discovery::discover_with_excludes(analysis_root, &config.analysis_excludes)?;
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        let relative = file
            .strip_prefix(analysis_root)
            .map_err(|_| "discovered path escaped the analysis root")?;
        let Some(relative) = relative.to_str() else {
            return Err("discovered path is not valid UTF-8".into());
        };
        let relative = normalize_path(Path::new(relative));
        validate_text(&relative)?;
        entries.push(SourceEntry {
            path: relative,
            bytes: std::fs::read(&file)?,
        });
    }
    Ok(entries)
}

fn z_records(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let Some(records) = bytes.strip_suffix(&[0]) else {
        return Err("git returned an unterminated entry list".into());
    };
    Ok(records.split(|byte| *byte == 0).collect())
}

fn z_paths(bytes: &[u8]) -> Result<Vec<String>> {
    z_records(bytes)?.into_iter().map(validate_path).collect()
}

struct GitRecord {
    path: String,
    oid: String,
    mode: u32,
    stage: u32,
    /// `ls-files --debug` reports intent-to-add entries; they are not content.
    intent_to_add: bool,
}

#[derive(Default)]
struct ObjectPlan {
    config: Option<String>,
    mailmap: Option<String>,
    sources: BTreeMap<String, String>,
}

fn index_plan(
    root: &Path,
    scope_prefix: &str,
    explicit: Option<&LocatedFile>,
) -> Result<ObjectPlan> {
    let config_path = format!("{scope_prefix}{CONFIG_FILE}");
    let mailmap_path = format!("{scope_prefix}{MAILMAP_FILE}");
    let requested = explicit.map(|file| format!("{scope_prefix}{}", file.name));
    let mut args = vec![
        "--literal-pathspecs",
        "ls-files",
        "--stage",
        "--debug",
        "-z",
    ];
    if let Some(requested) = &requested {
        args.push("--");
        args.push(requested);
        args.push(&config_path);
        args.push(&mailmap_path);
    }
    let output = git::run(root, &args)?;
    let plan = assemble_records(
        parse_index_records(&output.stdout)?,
        scope_prefix,
        &requested,
        &config_path,
        &mailmap_path,
    )?;
    if explicit.is_some() && plan.sources.is_empty() {
        return Err("explicit file is not present in the index".into());
    }
    Ok(plan)
}

fn revision_plan(
    root: &Path,
    commit: &str,
    scope_prefix: &str,
    explicit: Option<&LocatedFile>,
) -> Result<ObjectPlan> {
    let config_path = format!("{scope_prefix}{CONFIG_FILE}");
    let mailmap_path = format!("{scope_prefix}{MAILMAP_FILE}");
    let requested = explicit.map(|file| format!("{scope_prefix}{}", file.name));
    let mut args = vec!["--literal-pathspecs", "ls-tree", "-r", "-z", commit];
    match &requested {
        Some(requested) => {
            args.push("--");
            args.push(requested);
            args.push(&config_path);
            args.push(&mailmap_path);
        }
        None if !scope_prefix.is_empty() => {
            args.push("--");
            args.push(scope_prefix.trim_end_matches('/'));
        }
        None => {}
    }
    let output = git::run(root, &args)?;
    let plan = assemble_records(
        parse_tree_records(&output.stdout)?,
        scope_prefix,
        &requested,
        &config_path,
        &mailmap_path,
    )?;
    if explicit.is_some() && plan.sources.is_empty() {
        return Err("explicit file is not present in the revision".into());
    }
    Ok(plan)
}

fn assemble_records(
    records: Vec<GitRecord>,
    scope_prefix: &str,
    requested: &Option<String>,
    config_path: &str,
    mailmap_path: &str,
) -> Result<ObjectPlan> {
    let mut plan = ObjectPlan::default();
    for record in records {
        if let Some(requested) = requested
            && record.path.as_str() != requested.as_str()
            && record.path != config_path
            && record.path != mailmap_path
        {
            continue;
        }
        let Some(relative) = strip_scope(scope_prefix, &record.path) else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        if record.intent_to_add || record.oid.bytes().all(|byte| byte == b'0') {
            continue;
        }
        if record.stage != 0 {
            return Err(format!("unmerged index entry: {}", record.path).into());
        }
        if !matches!(record.mode, 0o100644 | 0o100755) {
            continue;
        }
        if record.path == config_path {
            plan.config = Some(record.oid);
            continue;
        }
        if record.path == mailmap_path {
            plan.mailmap = Some(record.oid);
            continue;
        }
        plan.sources.insert(relative.to_owned(), record.oid);
    }
    Ok(plan)
}

/// Parses `git ls-files --stage --debug -z`: each NUL-terminated stage record
/// is followed by a five-line debug block whose last line carries the index
/// flags. Intent-to-add entries are the `CE_INTENT_TO_ADD` flag bit; modern
/// Git stores them with the empty-blob OID, so the flag is the reliable
/// signal (a zero OID is still skipped defensively).
fn parse_index_records(bytes: &[u8]) -> Result<Vec<GitRecord>> {
    const INTENT_TO_ADD: u32 = 0x2000_0000;
    let mut records = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let Some(nul) = rest.iter().position(|byte| *byte == 0) else {
            return Err("git returned an unterminated entry list".into());
        };
        let record = &rest[..nul];
        rest = &rest[nul + 1..];

        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            return Err("git returned a malformed index entry".into());
        };
        let path = validate_path(&record[tab + 1..])?;
        let metadata = std::str::from_utf8(&record[..tab])
            .map_err(|_| "git returned a malformed index entry")?;
        let mut fields = metadata.split_whitespace();
        let (Some(mode), Some(oid), Some(stage), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err("git returned a malformed index entry".into());
        };
        let mode =
            u32::from_str_radix(mode, 8).map_err(|_| "git returned a malformed index mode")?;
        let stage = stage
            .parse::<u32>()
            .map_err(|_| "git returned a malformed index stage")?;
        if !is_hex_oid(oid) {
            return Err("git returned a malformed object ID".into());
        }

        let mut flags_line: &[u8] = &[];
        for _ in 0..5 {
            let Some(newline) = rest.iter().position(|byte| *byte == b'\n') else {
                return Err("git returned a malformed index debug block".into());
            };
            flags_line = &rest[..newline];
            rest = &rest[newline + 1..];
        }
        let Some(flags_at) = flags_line
            .windows(7)
            .position(|window| window == b"flags: ")
        else {
            return Err("git returned a malformed index debug block".into());
        };
        let flags = std::str::from_utf8(&flags_line[flags_at + 7..])
            .map_err(|_| "git returned malformed index flags")?;
        let flags =
            u32::from_str_radix(flags, 16).map_err(|_| "git returned malformed index flags")?;

        records.push(GitRecord {
            path,
            oid: oid.to_owned(),
            mode,
            stage,
            intent_to_add: flags & INTENT_TO_ADD != 0,
        });
    }
    Ok(records)
}

fn parse_tree_records(bytes: &[u8]) -> Result<Vec<GitRecord>> {
    let mut records = Vec::new();
    for record in z_records(bytes)? {
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            return Err("git returned a malformed tree entry".into());
        };
        let path = validate_path(&record[tab + 1..])?;
        let metadata = std::str::from_utf8(&record[..tab])
            .map_err(|_| "git returned a malformed tree entry")?;
        let mut fields = metadata.split(' ');
        let (Some(mode), Some(kind), Some(oid), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err("git returned a malformed tree entry".into());
        };
        if kind != "blob" {
            continue;
        }
        let mode =
            u32::from_str_radix(mode, 8).map_err(|_| "git returned a malformed tree mode")?;
        if !is_hex_oid(oid) {
            return Err("git returned a malformed object ID".into());
        }
        records.push(GitRecord {
            path,
            oid: oid.to_owned(),
            mode,
            stage: 0,
            intent_to_add: false,
        });
    }
    Ok(records)
}

type ObjectContents = (Option<Vec<u8>>, Option<Vec<u8>>, Vec<SourceEntry>);

fn load_objects(
    root: &Path,
    analysis_root: &Path,
    plan: ObjectPlan,
    explicit: bool,
) -> Result<ObjectContents> {
    let mut batch = git::ObjectBatch::open(root)?;
    let mailmap_bytes = match &plan.mailmap {
        Some(oid) => Some(batch.read(oid)?),
        None => None,
    };
    let config_bytes = match &plan.config {
        Some(oid) => Some(batch.read(oid)?),
        None => None,
    };
    let config = parse_config(config_bytes.as_deref())?;
    let filter = SourceFilter::new(analysis_root, &config.analysis_excludes)?;
    let mut entries = Vec::with_capacity(plan.sources.len());
    for (path, oid) in plan.sources {
        if explicit || filter.accepts_file(Path::new(&path)) {
            entries.push(SourceEntry {
                path,
                bytes: batch.read(&oid)?,
            });
        }
    }
    batch.finish()?;
    Ok((config_bytes, mailmap_bytes, entries))
}
