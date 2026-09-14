use crate::Result;
use crate::parser::detect_language;
use ignore::{DirEntry, WalkBuilder, WalkState};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    let files = Mutex::new(Vec::new());
    let error = Mutex::new(None::<ignore::Error>);
    let walker = WalkBuilder::new(path)
        .standard_filters(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(move |entry| filter.accepts_entry(entry))
        .build_parallel();
    walker.run(|| {
        let files = &files;
        let error = &error;
        Box::new(move |entry| {
            match entry {
                Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_file()) => {
                    files.lock().unwrap().push(entry.into_path());
                }
                Ok(_) => {}
                Err(walk_error) => {
                    let mut first_error = error.lock().unwrap();
                    if first_error.is_none() {
                        *first_error = Some(walk_error);
                    }
                    return WalkState::Quit;
                }
            }
            WalkState::Continue
        })
    });
    if let Some(error) = error.into_inner().unwrap() {
        return Err(error.into());
    }
    let mut files = files.into_inner().unwrap();
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
    ///
    /// Non-UTF-8 names are accepted when their extension is a supported
    /// source extension; snapshot callers reject such paths explicitly, while
    /// `analyze` keeps its historical lossy display-path behavior.
    pub fn accepts_file(&self, relative: &Path) -> bool {
        if has_ignored_component(relative) {
            return false;
        }
        let supported = match relative.to_str() {
            Some(text) => !text.is_empty() && detect_language(text).is_some(),
            None => supported_extension(relative),
        };
        supported && !self.is_excluded(relative, false)
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

/// Extension check for paths whose name is not valid UTF-8. All supported
/// source extensions are ASCII, so a valid UTF-8 extension is sufficient.
fn supported_extension(relative: &Path) -> bool {
    relative
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| detect_language(&format!("file.{extension}")).is_some())
}

fn has_ignored_component(relative: &Path) -> bool {
    relative.components().any(|component| match component {
        std::path::Component::Normal(name) => name
            .to_str()
            .is_some_and(|name| IGNORED_DIRECTORIES.contains(&name)),
        _ => false,
    })
}
