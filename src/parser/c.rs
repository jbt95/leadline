use super::{ParsedDependencies, RawDependency, RawDependencyKind, node_text};
use tree_sitter::Node;

/// Records quoted preprocessor includes as file-relative references.
pub(super) fn extract_c_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    let mut parsed = ParsedDependencies::default();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "preproc_include"
            && let Some(path) = node.child_by_field_name("path")
            && path.kind() == "string_literal"
        {
            let raw = node_text(path, source);
            if let Some(specifier) = raw
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
            {
                parsed.references.push(RawDependency {
                    kind: RawDependencyKind::LocalInclude,
                    specifier: specifier.to_owned(),
                    line: node.start_position().row as u32 + 1,
                });
            }
        }
        for index in (0..node.child_count()).rev() {
            stack.push(node.child(index).expect("child index is in bounds"));
        }
    }
    parsed
}
