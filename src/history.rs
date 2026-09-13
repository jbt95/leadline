//! Git history intelligence.
//!
//! This module turns one streamed `git log` walk into normalized historical
//! facts per file. It is deliberately independent from source parsing: it
//! never imports the metric engine or a parser backend. Consumers join its
//! output with static metrics by repo-relative path.
//!
//! Design decisions:
//!
//! - Two `git` subprocesses per analysis: one HEAD lookup and one
//!   `git log --relative --no-merges --numstat -z -M30%` stream. The log is
//!   parsed as a stream, not buffered whole, so large histories do not need
//!   to fit in memory. `--relative` walks only the requested directory and
//!   prints paths relative to it.
//! - Renames are resolved newest to oldest. Facts accumulated under an old
//!   path move to the file's current path, so a moved file keeps its history.
//!   The 30% similarity threshold is below git's 50% default so small moved
//!   files keep their past; the trade-off is that two unrelated small files
//!   with boilerplate in common can be paired as a rename. A rename that
//!   crosses the analysis scope is an add or a delete, not a move.
//! - Recency windows are relative to the HEAD commit time, never the wall
//!   clock. The same repository snapshot produces the same report.
//! - Merge commits are excluded. They do not represent authored changes, and
//!   `git log --numstat` emits no file records for them by default.
//! - A missing `git` executable or a directory outside any repository yields
//!   an unavailable report instead of an error, so source snapshots keep
//!   working.

use crate::Result;
use crate::coupling::{ProjectCoupling, ProjectCouplingAccumulator};
use crate::discovery::SourceFilter;
use crate::git;
use crate::source_snapshot::SnapshotContext;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;

pub const HISTORY_SCHEMA_VERSION: u32 = 1;

/// Window used for the "recent" contributor count.
pub const RECENT_DAYS: u64 = 90;

const DAY_SECONDS: i64 = 86_400;

/// Selects which precomputed churn window a consumer ranks by.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryWindow {
    Days30,
    Days90,
    Days365,
}

impl HistoryWindow {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "30d" => Some(HistoryWindow::Days30),
            "90d" => Some(HistoryWindow::Days90),
            "365d" => Some(HistoryWindow::Days365),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            HistoryWindow::Days30 => "30d",
            HistoryWindow::Days90 => "90d",
            HistoryWindow::Days365 => "365d",
        }
    }

    pub fn days(self) -> u64 {
        match self {
            HistoryWindow::Days30 => 30,
            HistoryWindow::Days90 => 90,
            HistoryWindow::Days365 => 365,
        }
    }

    pub fn changes(self, file: &FileHistory) -> u64 {
        match self {
            HistoryWindow::Days30 => file.changes_30d,
            HistoryWindow::Days90 => file.changes_90d,
            HistoryWindow::Days365 => file.changes_365d,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FileHistory {
    pub path: String,
    pub commits: u64,
    pub changes_30d: u64,
    pub changes_90d: u64,
    pub changes_365d: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub days_since_last_change: u64,
    pub age_days: u64,
    pub contributors: u64,
    pub recent_contributors: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HistoryReport {
    pub schema_version: u32,
    pub analyzer_version: &'static str,
    pub available: bool,
    /// How recency is anchored; always `head-commit-time`.
    pub reference: &'static str,
    pub head_commit: Option<String>,
    pub head_timestamp: Option<i64>,
    pub files: Vec<FileHistory>,
}

impl HistoryReport {
    pub fn file(&self, path: &str) -> Option<&FileHistory> {
        self.files.iter().find(|file| file.path == path)
    }
}

fn unavailable() -> HistoryReport {
    HistoryReport {
        schema_version: HISTORY_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        available: false,
        reference: "head-commit-time",
        head_commit: None,
        head_timestamp: None,
        files: Vec::new(),
    }
}

/// Anonymous per-file touch counts by contributor identity.
///
/// A touch is one commit by one identity affecting one file, regardless of
/// line count. Identities are never serialized by default; ownership
/// aggregation decides how (or whether) to project them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTouches {
    pub path: String,
    /// `(identity, touches)` sorted by identity.
    pub identities: Vec<(String, u64)>,
}

/// One revision-bounded walk's history, touches, and coupling.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GitAnalyticsSnapshot {
    pub history: HistoryReport,
    #[serde(skip)]
    pub touches: Vec<FileTouches>,
    pub coupling: ProjectCoupling,
}

/// Analyzes the commit `revision` and its ancestors without a mailmap.
pub fn analyze_git_at(context: &SnapshotContext, revision: &str) -> Result<GitAnalyticsSnapshot> {
    analyze_git_at_with_mailmap(context, revision, None)
}

/// Analyzes `revision` and its ancestors with an explicit target `.mailmap`.
///
/// The mailmap bytes come from the selected snapshot (a revision blob or the
/// worktree file); ambient `mailmap.file` configuration is never consulted.
pub fn analyze_git_at_with_mailmap(
    context: &SnapshotContext,
    revision: &str,
    mailmap: Option<&[u8]>,
) -> Result<GitAnalyticsSnapshot> {
    let Some(root) = context.repo_root.as_deref() else {
        return Ok(unavailable_analytics());
    };
    let (commit, commit_timestamp) = git::resolve_commit(root, revision)?;
    let mailmap = mailmap.map(Mailmap::parse).unwrap_or_default();
    let filter = SourceFilter::new(&context.analysis_root, &context.config.analysis_excludes)?;

    let mut history = HistoryAccumulator::new(commit_timestamp);
    let mut touches = TouchAccumulator::default();
    let mut coupling = ProjectCouplingAccumulator::new();
    walk_commits(&context.analysis_root, &commit, |meta, files| {
        let identity = mailmap.identity(meta);
        let kept: Vec<CommitFile> = files
            .iter()
            .filter(|file| {
                !file.path.starts_with("../") && filter.accepts_file(Path::new(&file.path))
            })
            .cloned()
            .collect();
        history.record(meta, &identity, &kept);
        touches.record(&identity, &kept);
        coupling.record(&kept);
    })?;

    Ok(GitAnalyticsSnapshot {
        history: HistoryReport {
            schema_version: HISTORY_SCHEMA_VERSION,
            analyzer_version: env!("CARGO_PKG_VERSION"),
            available: true,
            reference: "head-commit-time",
            head_commit: Some(commit),
            head_timestamp: Some(commit_timestamp),
            files: history.finish(),
        },
        touches: touches.finish(),
        coupling: coupling.finish(),
    })
}

fn unavailable_analytics() -> GitAnalyticsSnapshot {
    GitAnalyticsSnapshot {
        history: unavailable(),
        touches: Vec::new(),
        coupling: ProjectCoupling::unavailable("git_unavailable"),
    }
}

#[derive(Default)]
struct TouchAccumulator {
    files: BTreeMap<String, BTreeMap<String, u64>>,
}

impl TouchAccumulator {
    fn record(&mut self, identity: &str, files: &[CommitFile]) {
        let mut seen: HashSet<&str> = HashSet::new();
        for file in files {
            if !seen.insert(file.path.as_str()) {
                continue;
            }
            *self
                .files
                .entry(file.path.clone())
                .or_default()
                .entry(identity.to_owned())
                .or_default() += 1;
        }
    }

    fn finish(self) -> Vec<FileTouches> {
        self.files
            .into_iter()
            .map(|(path, identities)| FileTouches {
                path,
                identities: identities.into_iter().collect(),
            })
            .collect()
    }
}

/// Deterministic `.mailmap` application for one target blob.
#[derive(Clone, Debug, Default)]
struct Mailmap {
    rules: Vec<MailmapRule>,
}

#[derive(Clone, Debug)]
struct MailmapRule {
    old_name: Option<String>,
    old_email: String,
    new_email: Option<String>,
}

impl Mailmap {
    /// Parses mailmap lines; comments and malformed lines are ignored.
    fn parse(bytes: &[u8]) -> Mailmap {
        let mut rules = Vec::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() || line.starts_with(b"#") {
                continue;
            }
            if let Some(rule) = parse_mailmap_line(line) {
                rules.push(rule);
            }
        }
        Mailmap { rules }
    }

    /// Resolves an identity like git's mailmap: a name-specific rule wins over
    /// an email-only rule for the same address, regardless of file order.
    fn identity(&self, meta: &CommitMeta) -> String {
        if meta.email.is_empty() {
            return meta.author.clone();
        }
        let mut email_only: Option<&MailmapRule> = None;
        for rule in &self.rules {
            if !rule.old_email.eq_ignore_ascii_case(&meta.email) {
                continue;
            }
            match &rule.old_name {
                Some(old_name) if old_name.eq_ignore_ascii_case(&meta.author) => {
                    return resolved_email(rule, meta);
                }
                Some(_) => {}
                None if email_only.is_none() => email_only = Some(rule),
                None => {}
            }
        }
        match email_only {
            Some(rule) => resolved_email(rule, meta),
            None => meta.email.to_lowercase(),
        }
    }
}

fn resolved_email(rule: &MailmapRule, meta: &CommitMeta) -> String {
    rule.new_email
        .clone()
        .unwrap_or_else(|| meta.email.to_lowercase())
}

/// Parses the four documented git-mailmap forms:
///
/// 1. `Proper Name <commit@email>`
/// 2. `<proper@email> <commit@email>`
/// 3. `Proper Name <proper@email> <commit@email>`
/// 4. `Proper Name <proper@email> Commit Name <commit@email>`
fn parse_mailmap_line(line: &[u8]) -> Option<MailmapRule> {
    let text = std::str::from_utf8(line).ok()?;
    let mut names: Vec<&str> = Vec::new();
    let mut emails: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let name = rest[..open].trim();
        let after = &rest[open + 1..];
        let close = after.find('>')?;
        let email = after[..close].trim();
        if email.is_empty() {
            return None;
        }
        if !name.is_empty() {
            names.push(name);
        }
        emails.push(email);
        rest = &after[close + 1..];
    }
    // Git ignores content after the final `>`; do the same.
    match (names.len(), emails.len()) {
        // `Proper Name <commit@email>`: name replacement only.
        (1, 1) => Some(MailmapRule {
            old_name: None,
            old_email: emails[0].to_owned(),
            new_email: None,
        }),
        // `<proper@email> <commit@email>`.
        (0, 2) => Some(MailmapRule {
            old_name: None,
            old_email: emails[1].to_owned(),
            new_email: Some(emails[0].to_owned()),
        }),
        // `Proper Name <proper@email> <commit@email>`.
        (1, 2) => Some(MailmapRule {
            old_name: None,
            old_email: emails[1].to_owned(),
            new_email: Some(emails[0].to_owned()),
        }),
        // `Proper Name <proper@email> Commit Name <commit@email>`.
        (2, 2) => Some(MailmapRule {
            old_name: Some(names[1].to_owned()),
            old_email: emails[1].to_owned(),
            new_email: Some(emails[0].to_owned()),
        }),
        _ => None,
    }
}

/// Reads the worktree `.mailmap` at the enclosing repository root, if any.
fn worktree_mailmap(start: &Path) -> Option<Vec<u8>> {
    for ancestor in start.ancestors() {
        if ancestor.join(".git").exists() {
            let bytes = std::fs::read(ancestor.join(".mailmap")).ok()?;
            return (!bytes.is_empty()).then_some(bytes);
        }
    }
    None
}

/// Analyzes Git history for every file under `scope`.
///
/// `scope` is a repository directory; results are keyed relative to it and
/// match the paths produced by `leadline::analyze_path` for the same
/// directory. Git performs the scoping: `git log --relative` walks only the
/// requested directory and prints scope-relative paths, so subdirectories do
/// not pay for whole-repository path filtering. A missing repository, missing
/// `git`, or unborn HEAD produces an unavailable report with no files.
///
/// A rename that crosses the scope boundary is reported as an add or delete,
/// because only one side of the pair is inside the scope.
pub fn analyze_history(scope: &Path) -> Result<HistoryReport> {
    let workdir = workdir_for(scope);
    let Some((head_commit, head_timestamp)) = repository_head(workdir)? else {
        return Ok(unavailable());
    };
    let Some(head_timestamp) = head_timestamp else {
        let mut report = unavailable();
        report.available = true;
        return Ok(report);
    };
    let commit = head_commit.clone().unwrap_or_else(|| "HEAD".to_owned());
    let mailmap = worktree_mailmap(workdir)
        .map(|bytes| Mailmap::parse(&bytes))
        .unwrap_or_default();
    let mut accumulator = HistoryAccumulator::new(head_timestamp);
    walk_commits(workdir, &commit, |meta, files| {
        accumulator.record(meta, &mailmap.identity(meta), files)
    })?;
    Ok(HistoryReport {
        schema_version: HISTORY_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        available: true,
        reference: "head-commit-time",
        head_commit: Some(head_commit.unwrap_or_default()),
        head_timestamp: Some(head_timestamp),
        files: accumulator.finish(),
    })
}

fn workdir_for(scope: &Path) -> &Path {
    if scope.is_dir() {
        scope
    } else {
        scope.parent().unwrap_or(Path::new("."))
    }
}

/// HEAD metadata for `workdir`; `None` when Git or a repository is absent.
///
/// The tuple is `(commit, timestamp)`; both are `None` for an unborn HEAD.
pub(crate) fn repository_head(workdir: &Path) -> Result<Option<(Option<String>, Option<i64>)>> {
    let Some(head) = git::repository_head_optional(workdir)? else {
        return Ok(None);
    };
    Ok(Some(parse_head(&head)))
}

/// Parses `--format=%H%x00%ct` output. Returns `(None, None)` for an unborn HEAD.
fn parse_head(bytes: &[u8]) -> (Option<String>, Option<i64>) {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    let Some((hash, timestamp)) = text.split_once('\0') else {
        return (None, None);
    };
    let hash = hash.trim();
    let timestamp = timestamp.trim().parse().ok();
    if hash.is_empty() {
        (None, timestamp)
    } else {
        (Some(hash.to_owned()), timestamp)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CommitMeta {
    pub author: String,
    pub email: String,
    pub timestamp: i64,
}

/// One file touched by one commit, keyed by its resolved current path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitFile {
    pub path: String,
    pub lines_added: u64,
    pub lines_deleted: u64,
}

/// Streams the repository history once, newest first, calling `sink` for each
/// commit that touched at least one in-scope file.
///
/// Both file-history aggregation and temporal-coupling indexing run through
/// this single walk, so the rename framing and config hardening exist once.
///
/// `--date-order` guarantees that a commit is emitted before its parents even
/// when commit dates are skewed, so a rename record is always seen before the
/// older records it must claim. `--no-show-signature` and `--no-notes` keep
/// the byte stream to the requested format regardless of user config.
pub(crate) fn walk_commits(
    workdir: &Path,
    commit: &str,
    mut sink: impl FnMut(&CommitMeta, &[CommitFile]),
) -> Result<()> {
    let mut child = git::command(
        workdir,
        &[
            "log",
            commit,
            "--relative",
            "--date-order",
            "--no-merges",
            "--no-show-signature",
            "--no-notes",
            "--numstat",
            "-z",
            "-M30%",
            // Raw author fields: mailmap application is deterministic and
            // target-scoped, never ambient Git configuration.
            "--format=%x01%H%x00%an%x00%ae%x00%ct%x00",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
    let stdout = child.stdout.take().ok_or("git log stdout unavailable")?;
    // Drain stderr on its own thread: a full stderr pipe would block git and
    // stall stdout parsing.
    let mut stderr = child.stderr.take().ok_or("git log stderr unavailable")?;
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut bytes);
        bytes
    });
    let parsed = parse_stream(BufReader::new(stdout), &mut sink);
    let status = child.wait()?;
    let stderr = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(format!("git log failed: {}", stderr.trim()).into());
    }
    parsed
}

struct HistoryAccumulator {
    head_timestamp: i64,
    files: BTreeMap<String, FileFacts>,
}

impl HistoryAccumulator {
    fn new(head_timestamp: i64) -> Self {
        Self {
            head_timestamp,
            files: BTreeMap::new(),
        }
    }

    fn record(&mut self, meta: &CommitMeta, identity: &str, files: &[CommitFile]) {
        for file in files {
            let facts = self.files.entry(file.path.clone()).or_default();
            facts.record(
                meta,
                identity,
                self.head_timestamp,
                file.lines_added,
                file.lines_deleted,
            );
        }
    }

    fn finish(self) -> Vec<FileHistory> {
        let mut result = Vec::new();
        for (path, facts) in self.files {
            // Defensive: `--relative` can print `../` pre-images for
            // cross-scope renames on some git versions; those paths are
            // outside the scope.
            if path.starts_with("../") {
                continue;
            }
            result.push(facts.finish(path, self.head_timestamp));
        }
        result
    }
}

#[derive(Default)]
struct FileFacts {
    commits: u64,
    changes_30d: u64,
    changes_90d: u64,
    changes_365d: u64,
    lines_added: u64,
    lines_deleted: u64,
    first_timestamp: Option<i64>,
    last_timestamp: Option<i64>,
    contributors: HashSet<String>,
    recent_contributors: HashSet<String>,
}

impl FileFacts {
    fn record(
        &mut self,
        meta: &CommitMeta,
        identity: &str,
        head_timestamp: i64,
        lines_added: u64,
        lines_deleted: u64,
    ) {
        self.commits += 1;
        self.lines_added += lines_added;
        self.lines_deleted += lines_deleted;
        self.first_timestamp = Some(
            self.first_timestamp
                .map_or(meta.timestamp, |first| first.min(meta.timestamp)),
        );
        self.last_timestamp = Some(
            self.last_timestamp
                .map_or(meta.timestamp, |last| last.max(meta.timestamp)),
        );
        let age = (head_timestamp - meta.timestamp).max(0);
        if age <= 30 * DAY_SECONDS {
            self.changes_30d += 1;
        }
        if age <= 90 * DAY_SECONDS {
            self.changes_90d += 1;
        }
        if age <= 365 * DAY_SECONDS {
            self.changes_365d += 1;
        }
        if age <= (RECENT_DAYS as i64) * DAY_SECONDS {
            self.recent_contributors.insert(identity.to_owned());
        }
        self.contributors.insert(identity.to_owned());
    }

    fn finish(self, path: String, head_timestamp: i64) -> FileHistory {
        let days = |timestamp: Option<i64>| {
            timestamp
                .map(|value| ((head_timestamp - value).max(0) / DAY_SECONDS) as u64)
                .unwrap_or(0)
        };
        FileHistory {
            path,
            commits: self.commits,
            changes_30d: self.changes_30d,
            changes_90d: self.changes_90d,
            changes_365d: self.changes_365d,
            lines_added: self.lines_added,
            lines_deleted: self.lines_deleted,
            days_since_last_change: days(self.last_timestamp),
            age_days: days(self.first_timestamp),
            contributors: self.contributors.len() as u64,
            recent_contributors: self.recent_contributors.len() as u64,
        }
    }
}

/// Streaming parser for the NUL-delimited `git log --numstat -z` layout.
///
/// Chunks look like:
///
/// ```text
/// \x01<hash>  <author>  <email>  <unix timestamp>  (empty)  \n<added>\t<deleted>\t<path>
/// ```
///
/// A rename record arrives as `added\tdeleted\t` followed by the old path and
/// then the new path. Everything else that starts with a number is a regular
/// record. `-` marks binary files and counts as zero lines.
fn parse_stream<R: BufRead>(
    mut reader: R,
    sink: &mut impl FnMut(&CommitMeta, &[CommitFile]),
) -> Result<()> {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut meta: Option<CommitMeta> = None;
    let mut state = 0_u8; // 0 = hash, 1 = author, 2 = email, 3 = timestamp, 4 = records
    let mut rename: Option<(u64, u64, String)> = None;
    let mut chunk = Vec::new();
    let mut files: Vec<CommitFile> = Vec::new();

    loop {
        chunk.clear();
        if reader.read_until(0, &mut chunk)? == 0 {
            break;
        }
        if chunk.last() == Some(&0) {
            chunk.pop();
        }
        if chunk.is_empty() {
            continue;
        }
        if chunk[0] == 1 && chunk.len() >= 8 && chunk[1..].iter().all(u8::is_ascii_hexdigit) {
            flush_commit(sink, &meta, &mut files);
            meta = Some(CommitMeta::default());
            state = 1;
            rename = None;
            continue;
        }
        match state {
            1 => {
                if let Some(meta) = meta.as_mut() {
                    meta.author = String::from_utf8_lossy(&chunk).into_owned();
                }
                state = 2;
            }
            2 => {
                if let Some(meta) = meta.as_mut() {
                    meta.email = String::from_utf8_lossy(&chunk).into_owned();
                }
                state = 3;
            }
            3 => {
                let timestamp = String::from_utf8_lossy(&chunk).trim().parse::<i64>().ok();
                match timestamp {
                    Some(value) => {
                        if let Some(meta) = meta.as_mut() {
                            meta.timestamp = value;
                        }
                    }
                    // A malformed timestamp would corrupt every age in the
                    // report; drop the commit's records instead.
                    None => meta = None,
                }
                state = 4;
            }
            _ => {
                if meta.is_none() {
                    continue;
                }
                let record_bytes: &[u8] = chunk.strip_prefix(b"\n").unwrap_or(&chunk);
                if let Some((added, deleted, old)) = rename.as_mut() {
                    if old.is_empty() {
                        *old = String::from_utf8_lossy(record_bytes).into_owned();
                    } else {
                        let new = String::from_utf8_lossy(record_bytes).into_owned();
                        let resolved = resolve_alias(&aliases, &new);
                        aliases.insert(old.clone(), resolved.clone());
                        files.push(CommitFile {
                            path: resolved,
                            lines_added: *added,
                            lines_deleted: *deleted,
                        });
                        rename = None;
                    }
                    continue;
                }
                let mut parts = record_bytes.splitn(3, |byte| *byte == b'\t');
                let Some(added) = parts.next() else { continue };
                let Some(deleted) = parts.next() else {
                    continue;
                };
                let Some(path) = parts.next() else { continue };
                let added = numstat_value(added);
                let deleted = numstat_value(deleted);
                let path = String::from_utf8_lossy(path).into_owned();
                if path.is_empty() {
                    rename = Some((added, deleted, String::new()));
                    continue;
                }
                let resolved = resolve_alias(&aliases, &path);
                files.push(CommitFile {
                    path: resolved,
                    lines_added: added,
                    lines_deleted: deleted,
                });
            }
        }
    }

    flush_commit(sink, &meta, &mut files);
    Ok(())
}

/// Emits the buffered files of a finished commit and resets the buffer.
///
/// Files are sorted and merged by path so a commit contributes one record per
/// file; duplicate records (possible with unusual rename detection output)
/// sum their line deltas instead of dropping them.
fn flush_commit(
    sink: &mut impl FnMut(&CommitMeta, &[CommitFile]),
    meta: &Option<CommitMeta>,
    files: &mut Vec<CommitFile>,
) {
    if files.is_empty() {
        return;
    }
    let Some(meta) = meta else {
        files.clear();
        return;
    };
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut merged: Vec<CommitFile> = Vec::with_capacity(files.len());
    for file in files.drain(..) {
        if let Some(last) = merged.last_mut().filter(|last| last.path == file.path) {
            last.lines_added = last.lines_added.saturating_add(file.lines_added);
            last.lines_deleted = last.lines_deleted.saturating_add(file.lines_deleted);
            continue;
        }
        merged.push(file);
    }
    sink(meta, &merged);
}

/// Test seam: aggregates a synthetic byte stream exactly like production does.
#[cfg(test)]
fn parse_log<R: BufRead>(reader: R, head_timestamp: i64) -> Result<BTreeMap<String, FileHistory>> {
    let mut accumulator = HistoryAccumulator::new(head_timestamp);
    parse_stream(reader, &mut |meta, files| {
        let identity = if meta.email.is_empty() {
            meta.author.clone()
        } else {
            meta.email.to_lowercase()
        };
        accumulator.record(meta, &identity, files)
    })?;
    let mut files = BTreeMap::new();
    for file in accumulator.finish() {
        files.insert(file.path.clone(), file);
    }
    Ok(files)
}

fn resolve_alias(aliases: &BTreeMap<String, String>, path: &str) -> String {
    let mut current = path.to_owned();
    // Rename chains are short; the cap only guards against malformed input.
    for _ in 0..64 {
        match aliases.get(&current) {
            Some(next) => current = next.clone(),
            None => break,
        }
    }
    current
}

fn numstat_value(field: &[u8]) -> u64 {
    std::str::from_utf8(field)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: i64 = 1_000_000_000;
    const DAY: i64 = 86_400;

    fn commit(hash: &str, author: &str, email: &str, timestamp: i64, records: &[&str]) -> Vec<u8> {
        let hash = format!("{hash:0>40}");
        let mut bytes = format!("\u{1}{hash}\0{author}\0{email}\0{timestamp}\0").into_bytes();
        bytes.push(0);
        for record in records {
            if !record.is_empty() {
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(record.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    fn parse(bytes: Vec<u8>) -> BTreeMap<String, FileHistory> {
        parse_log(std::io::Cursor::new(bytes), HEAD).unwrap()
    }

    #[test]
    fn parses_records_authors_binary_and_windows() {
        let mut bytes = commit(
            "a",
            "Alice",
            "alice@example.invalid",
            HEAD - 30 * DAY,
            &["5\t2\tsrc/a.ts", "-\t-\tassets/logo.png"],
        );
        bytes.extend(commit(
            "b",
            "Bob",
            "bob@example.invalid",
            HEAD,
            &["1\t0\tsrc/a.ts"],
        ));
        let files = parse(bytes);

        let a = &files["src/a.ts"];
        assert_eq!(a.commits, 2);
        assert_eq!(a.changes_30d, 2, "a change exactly 30 days old is included");
        assert_eq!(a.changes_90d, 2);
        assert_eq!(a.changes_365d, 2);
        assert_eq!(a.lines_added, 6);
        assert_eq!(a.lines_deleted, 2);
        assert_eq!(a.contributors, 2);
        assert_eq!(a.recent_contributors, 2);
        assert_eq!(a.days_since_last_change, 0);
        assert_eq!(a.age_days, 30);

        let binary = &files["assets/logo.png"];
        assert_eq!(binary.commits, 1);
        assert_eq!(binary.lines_added, 0);
        assert_eq!(binary.lines_deleted, 0);
    }

    #[test]
    fn excludes_changes_just_outside_a_window() {
        let mut bytes = commit(
            "a",
            "Alice",
            "a@x",
            HEAD - 30 * DAY - 1,
            &["1\t0\tjust_out.ts"],
        );
        bytes.extend(commit("b", "Alice", "a@x", HEAD, &["1\t0\tsrc/a.ts"]));
        let files = parse(bytes);
        assert_eq!(files["just_out.ts"].changes_30d, 0);
        assert_eq!(files["just_out.ts"].changes_90d, 1);
    }

    #[test]
    fn duplicate_path_records_merge_deltas_in_one_commit() {
        let bytes = commit(
            "a",
            "Alice",
            "a@x",
            HEAD,
            &["3\t1\tsrc/a.ts", "2\t2\tsrc/a.ts"],
        );
        let files = parse(bytes);
        let a = &files["src/a.ts"];
        assert_eq!(a.commits, 1);
        assert_eq!(a.lines_added, 5);
        assert_eq!(a.lines_deleted, 3);
    }

    #[test]
    fn rename_records_move_older_history_to_the_new_path() {
        // git log is newest first; the alias map depends on that order.
        let mut bytes = commit("c", "Alice", "a@x", HEAD, &["1\t0\tsrc/new.ts"]);
        bytes.extend(commit(
            "b",
            "Alice",
            "a@x",
            HEAD,
            &["2\t1\t\0src/old.ts\0src/new.ts"],
        ));
        bytes.extend(commit(
            "a",
            "Alice",
            "a@x",
            HEAD - 10 * DAY,
            &["3\t0\tsrc/old.ts"],
        ));
        let files = parse(bytes);
        assert!(!files.contains_key("src/old.ts"));
        let new = &files["src/new.ts"];
        assert_eq!(new.commits, 3);
        assert_eq!(new.lines_added, 6);
        assert_eq!(new.lines_deleted, 1);
        assert_eq!(new.age_days, 10);
    }

    #[test]
    fn rename_chains_resolve_to_the_newest_path() {
        let mut bytes = commit("c", "Alice", "a@x", HEAD, &["1\t0\t\0two.ts\0three.ts"]);
        bytes.extend(commit(
            "b",
            "Alice",
            "a@x",
            HEAD - 4 * DAY,
            &["1\t0\t\0one.ts\0two.ts"],
        ));
        bytes.extend(commit(
            "a",
            "Alice",
            "a@x",
            HEAD - 5 * DAY,
            &["1\t0\tone.ts"],
        ));
        let files = parse(bytes);
        assert_eq!(files.len(), 1);
        assert_eq!(files["three.ts"].commits, 3);
    }

    #[test]
    fn malformed_record_is_skipped_without_panicking() {
        let mut bytes = commit("a", "Alice", "a@x", HEAD, &["not-a-record"]);
        bytes.extend(commit("b", "Alice", "a@x", HEAD, &["1\t0\tsrc/a.ts"]));
        let files = parse(bytes);
        assert_eq!(files["src/a.ts"].commits, 1);
    }

    #[test]
    fn malformed_timestamp_drops_the_commit_records() {
        let bytes =
            b"\x01aaaaaaaa\x00Alice\x00a@x\x00not-a-number\x00\x00\n1\t0\tsrc/a.ts\x00".to_vec();
        let files = parse(bytes);
        assert!(files.is_empty());
    }

    #[test]
    fn path_starting_with_marker_byte_is_not_a_commit_header() {
        // Newest first: the rename record precedes the old-path records.
        let mut bytes = commit(
            "b",
            "Alice",
            "a@x",
            HEAD,
            &["1\t0\t\0src/old.ts\0\u{1}weird.ts"],
        );
        bytes.extend(commit(
            "a",
            "Alice",
            "a@x",
            HEAD - DAY,
            &["1\t0\tsrc/old.ts"],
        ));
        let files = parse(bytes);
        assert!(
            files.contains_key("\u{1}weird.ts"),
            "a path that starts with 0x01 must survive as a rename target"
        );
        assert_eq!(files["\u{1}weird.ts"].commits, 2);
    }

    fn meta(author: &str, email: &str) -> CommitMeta {
        CommitMeta {
            author: author.to_owned(),
            email: email.to_owned(),
            timestamp: HEAD,
        }
    }

    #[test]
    fn mailmap_parses_all_four_documented_forms() {
        // Form 1: name replacement only; identity keeps the email.
        let form_one = Mailmap::parse(b"Proper Name <same@x>\n");
        assert_eq!(form_one.identity(&meta("Raw", "same@x")), "same@x");
        // Form 2: email-only replacement.
        let form_two = Mailmap::parse(b"<proper@x> <commit@x>\n");
        assert_eq!(form_two.identity(&meta("Raw", "commit@x")), "proper@x");
        // Form 3: proper name plus email-only old side.
        let form_three = Mailmap::parse(b"Proper Name <proper@x> <commit@x>\n");
        assert_eq!(form_three.identity(&meta("Raw", "commit@x")), "proper@x");
        // Form 4: name-specific old side.
        let form_four = Mailmap::parse(b"Proper <proper@x> Old Name <commit@x>\n");
        assert_eq!(
            form_four.identity(&meta("Old Name", "commit@x")),
            "proper@x"
        );
        assert_eq!(
            form_four.identity(&meta("Different Name", "commit@x")),
            "commit@x"
        );
    }

    #[test]
    fn mailmap_matching_is_case_insensitive() {
        let mailmap = Mailmap::parse(b"Proper <proper@x> Old Name <Commit@X>\n");
        assert_eq!(mailmap.identity(&meta("old name", "commit@x")), "proper@x");
        assert_eq!(mailmap.identity(&meta("OLD NAME", "COMMIT@X")), "proper@x");
    }

    #[test]
    fn mailmap_name_specific_rule_wins_over_email_only_rule_in_any_order() {
        // Git merges a simple email entry with name-specific overrides; the
        // name-specific rule must win regardless of .mailmap line order.
        let lines = b"Canon <canon@x> <shared@x>\nOther <other@x> Alias <shared@x>\n";
        let mailmap = Mailmap::parse(lines);
        assert_eq!(mailmap.identity(&meta("Alias", "shared@x")), "other@x");
        assert_eq!(mailmap.identity(&meta("Someone", "shared@x")), "canon@x");

        let reversed = b"Other <other@x> Alias <shared@x>\nCanon <canon@x> <shared@x>\n";
        let mailmap = Mailmap::parse(reversed);
        assert_eq!(mailmap.identity(&meta("Alias", "shared@x")), "other@x");
        assert_eq!(mailmap.identity(&meta("Someone", "shared@x")), "canon@x");
    }

    #[test]
    fn mailmap_ignores_comments_crlf_and_malformed_lines() {
        let mailmap = Mailmap::parse(
            b"# comment\r\nnot a mapping\r\n\r\nProper <proper@x> <commit@x> trailing\r\n",
        );
        assert_eq!(mailmap.identity(&meta("Raw", "commit@x")), "proper@x");
        assert_eq!(mailmap.identity(&meta("Raw", "other@x")), "other@x");
    }
}
