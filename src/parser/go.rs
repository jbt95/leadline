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
