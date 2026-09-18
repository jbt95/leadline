use super::{ParsedDependencies, RawDependencyKind};
use tree_sitter::Node;

pub(super) fn extract_go_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    // Full import_spec walk lands in Task 3; empty keeps Task 2 green.
    let _ = (root, source, RawDependencyKind::GoImport);
    ParsedDependencies::default()
}
