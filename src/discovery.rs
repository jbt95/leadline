use crate::Result;
use crate::parser::detect_language;
use ignore::{DirEntry, WalkBuilder};
use std::path::{Path, PathBuf};

const IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "coverage",
    ".next",
    ".gradle",
    "vendor",
    "generated",
];

pub fn discover(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        let display = path.to_string_lossy();
        return if detect_language(&display).is_some() {
            Ok(vec![path.to_owned()])
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unsupported source extension",
            )
            .into())
        };
    }
    discover_directory(path, &[])
}

/// Like [`discover`], but additionally skips paths matching any of `excludes`.
///
/// Patterns use gitignore semantics (`**` supported) relative to `path`.
/// An empty `excludes` behaves exactly like [`discover`].
pub fn discover_with_excludes(path: &Path, excludes: &[String]) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return discover(path);
    }
    discover_directory(path, excludes)
}

fn discover_directory(path: &Path, excludes: &[String]) -> Result<Vec<PathBuf>> {
    if !path.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("analysis path does not exist: {}", path.display()),
        )
        .into());
    }
    let filter = SourceFilter::new(path, excludes)?;
    let mut files = Vec::new();
    let walker = WalkBuilder::new(path)
        .standard_filters(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(move |entry| filter.accepts_entry(entry))
        .build();
    for entry in walker {
        let entry = entry?;
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

/// Shared source-file predicate for filesystem walks and Git enumeration.
///
/// Paths are analysis-root-relative. The filter applies the fixed ignored
/// directory list, supported-language detection, and configured excludes.
/// Filesystem callers remain responsible for the regular-file and symlink
/// checks; Git callers restrict records to regular blob modes.
pub struct SourceFilter {
    root: PathBuf,
    matcher: ignore::gitignore::Gitignore,
}

impl SourceFilter {
    /// Builds a filter whose excludes are relative to `root`.
    pub fn new(root: &Path, excludes: &[String]) -> Result<SourceFilter> {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
        for pattern in excludes {
            builder.add_line(None, pattern)?;
        }
        Ok(SourceFilter {
            root: root.to_owned(),
            matcher: builder.build()?,
        })
    }

    /// Returns whether an analysis-root-relative path is a supported source.
    pub fn accepts_file(&self, relative: &Path) -> bool {
        let Some(text) = relative.to_str() else {
            return false;
        };
        if text.is_empty() || has_ignored_component(relative) {
            return false;
        }
        detect_language(text).is_some() && !self.is_excluded(relative, false)
    }

    fn accepts_entry(&self, entry: &DirEntry) -> bool {
        let relative = entry
            .path()
            .strip_prefix(&self.root)
            .unwrap_or_else(|_| entry.path());
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            relative.as_os_str().is_empty() || self.accepts_directory(relative)
        } else {
            self.accepts_file(relative)
        }
    }

    fn accepts_directory(&self, relative: &Path) -> bool {
        !has_ignored_component(relative) && !self.is_excluded(relative, true)
    }

    fn is_excluded(&self, relative: &Path, is_dir: bool) -> bool {
        self.matcher
            .matched(self.root.join(relative), is_dir)
            .is_ignore()
    }
}

fn has_ignored_component(relative: &Path) -> bool {
    relative.components().any(|component| match component {
        std::path::Component::Normal(name) => name
            .to_str()
            .is_some_and(|name| IGNORED_DIRECTORIES.contains(&name)),
        _ => false,
    })
}
