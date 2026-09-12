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
    if !path.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("analysis path does not exist: {}", path.display()),
        )
        .into());
    }

    let mut files = Vec::new();
    let walker = WalkBuilder::new(path)
        .standard_filters(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(not_generated)
        .build();
    for entry in walker {
        let entry = entry?;
        if entry.file_type().is_some_and(|kind| kind.is_file())
            && detect_language(&entry.path().to_string_lossy()).is_some()
        {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

/// Like [`discover`], but additionally skips paths matching any of `excludes`.
///
/// Patterns use gitignore semantics (`**` supported) relative to `path`.
/// An empty `excludes` behaves exactly like [`discover`].
pub fn discover_with_excludes(path: &Path, excludes: &[String]) -> Result<Vec<PathBuf>> {
    if excludes.is_empty() {
        return discover(path);
    }
    if path.is_file() {
        return discover(path);
    }
    if !path.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("analysis path does not exist: {}", path.display()),
        )
        .into());
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(path);
    for pattern in excludes {
        builder.add_line(None, pattern)?;
    }
    let matcher = builder.build()?;
    let mut files = Vec::new();
    let walker = WalkBuilder::new(path)
        .standard_filters(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            not_generated(entry)
                && !matcher
                    .matched(
                        entry.path(),
                        entry.file_type().is_some_and(|kind| kind.is_dir()),
                    )
                    .is_ignore()
        })
        .build();
    for entry in walker {
        let entry = entry?;
        if entry.file_type().is_some_and(|kind| kind.is_file())
            && detect_language(&entry.path().to_string_lossy()).is_some()
        {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

fn not_generated(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !IGNORED_DIRECTORIES.contains(&name.as_ref())
}
