use super::{
    ParsedDependencies, RawDependency, RawDependencyKind, node_text, push_children_reversed,
};
use tree_sitter::Node;

/// True for Zig files a build or project convention runs without an import edge.
///
/// `build.zig` is a convention basename at any depth. The three source roots
/// are recognized only at the analysis root or at an exact `/src/...` suffix;
/// arbitrary Zig modules remain subject to graph reachability.
pub(crate) fn is_conventional_entry(path: &str) -> bool {
    let path = path.replace('\\', "/");
    if path.rsplit('/').next() == Some("build.zig") {
        return true;
    }
    ["src/main.zig", "src/lib.zig", "src/root.zig"]
        .iter()
        .any(|entry| {
            path.strip_suffix(entry)
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('/'))
        })
}

/// Records string arguments passed to Zig's `@import` builtin.
pub(super) fn extract_imports(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    let mut parsed = ParsedDependencies::default();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "builtin_function"
            && let Some(reference) = import_reference(node, source)
        {
            parsed.references.push(reference);
        }
        push_children_reversed(node, &mut stack);
    }
    parsed
}

fn import_reference(node: Node<'_>, source: &[u8]) -> Option<RawDependency> {
    let identifier = child_of_kind(node, "builtin_identifier")?;
    if node_text(identifier, source) != "@import" {
        return None;
    }
    let arguments = child_of_kind(node, "arguments")?;
    let mut cursor = arguments.walk();
    let value = arguments
        .named_children(&mut cursor)
        .find(|child| !child.is_extra())?;
    if value.kind() != "string" {
        return None;
    }
    let specifier = node_text(value, source)
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))?;
    if specifier.is_empty() {
        return None;
    }
    Some(RawDependency {
        kind: RawDependencyKind::ZigImport,
        specifier: specifier.to_owned(),
        line: node.start_position().row as u32 + 1,
    })
}

fn child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}
