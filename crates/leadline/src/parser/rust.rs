use super::{ParsedDependencies, RawDependency, RawDependencyKind, node_text};
use tree_sitter::Node;

/// True for the files Cargo compiles as crate roots.
///
/// Cargo names them by convention: `lib.rs`, `main.rs`, and `build.rs`
/// anywhere, plus every `.rs` file inside a `bin`, `benches`, `examples`, or
/// `tests` directory, which covers `src/bin` and workspace members' target
/// directories alike.
pub(crate) fn is_crate_root(path: &str) -> bool {
    let (directory, name) = split_path(path);
    if matches!(name, "lib.rs" | "main.rs" | "build.rs") {
        return true;
    }
    name.ends_with(".rs")
        && matches!(
            directory.rsplit('/').next().unwrap_or_default(),
            "bin" | "benches" | "examples" | "tests"
        )
}

/// Directory a `mod` declaration inside `path` resolves against.
///
/// A subtree module lives beside the declaring file when that file is named
/// `mod.rs` or is a crate root; every other module file owns the directory
/// named after it, so `src/telemetry.rs` declares `src/telemetry/counters.rs`.
pub(crate) fn module_directory(path: &str) -> String {
    let (directory, name) = split_path(path);
    if name == "mod.rs" || is_crate_root(path) {
        return directory.to_owned();
    }
    match name.strip_suffix(".rs") {
        Some(stem) if directory.is_empty() => stem.to_owned(),
        Some(stem) => format!("{directory}/{stem}"),
        None => directory.to_owned(),
    }
}

/// Directory and file name of an analysis-root-relative path.
fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// Records `mod foo;` declarations as file references.
///
/// An inline `mod foo { .. }` body lives in the declaring file, so it is not
/// a reference; its inner declarations are searched under a `foo/` prefix,
/// because `mod bar;` inside it names `foo/bar.rs` or `foo/bar/mod.rs`.
/// `use` paths and `extern crate` items are module-qualified, never
/// file-relative, so they produce no reference at all.
pub(super) fn extract_rust_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    let mut parsed = ParsedDependencies::default();
    let mut stack = vec![(root, String::new())];
    while let Some((node, prefix)) = stack.pop() {
        if node.kind() == "mod_item" {
            let name = node
                .child_by_field_name("name")
                .map(|name| node_text(name, source));
            if let Some(body) = node.child_by_field_name("body") {
                if let Some(name) = name {
                    let inner = format!("{prefix}{name}/");
                    for index in (0..body.child_count()).rev() {
                        stack.push((
                            body.child(index).expect("child index is in bounds"),
                            inner.clone(),
                        ));
                    }
                }
            } else if let Some(name) = name {
                parsed.references.push(RawDependency {
                    kind: RawDependencyKind::RustModule,
                    specifier: format!("{prefix}{name}"),
                    line: node.start_position().row as u32 + 1,
                });
            }
            continue;
        }
        for index in (0..node.child_count()).rev() {
            stack.push((
                node.child(index).expect("child index is in bounds"),
                prefix.clone(),
            ));
        }
    }
    parsed
}
