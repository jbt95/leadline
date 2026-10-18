use super::{ParsedDependencies, RawDependency, RawDependencyKind, node_text};
use tree_sitter::Node;

pub(super) fn extract_go_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    let mut parsed = ParsedDependencies::default();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "import_spec"
            && let Some(path) = node.child_by_field_name("path")
        {
            let raw = node_text(path, source);
            let specifier = raw
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
                .or_else(|| {
                    raw.strip_prefix('`')
                        .and_then(|text| text.strip_suffix('`'))
                })
                .unwrap_or(raw);
            parsed.references.push(RawDependency {
                kind: RawDependencyKind::GoImport,
                specifier: specifier.to_owned(),
                line: node.start_position().row as u32 + 1,
            });
        }
        for index in (0..node.child_count()).rev() {
            stack.push(node.child(index).expect("child index is in bounds"));
        }
    }
    parsed
}

/// True when the file declares `package main`.
///
/// The Go toolchain builds and runs a `main` package by name, so the file is
/// reachable without an importer: `go build ./...`, `go run ./cmd/x`, and a
/// bare `go run file.go` all take that path. A library package is only reached
/// through an import edge, and Go import paths are module-qualified, so no
/// other package is reachable by name.
///
/// The clause is looked up among the named children rather than assumed to be
/// the first: a doc comment above `package main` is a named node too. The
/// grammar exposes no `name` field on `package_clause`, so the identifier is
/// read from its named child.
pub(crate) fn has_main_package(root: Node<'_>, source: &[u8]) -> bool {
    let mut cursor = root.walk();
    root.named_children(&mut cursor).any(|child| {
        child.kind() == "package_clause"
            && child.named_child(0).is_some_and(|name| {
                name.kind() == "package_identifier"
                    && name.utf8_text(source).is_ok_and(|text| text == "main")
            })
    })
}
