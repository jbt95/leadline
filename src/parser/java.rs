use super::{ParsedDependencies, RawDependency, RawDependencyKind, node_text};
use tree_sitter::Node;

pub(super) fn extract_java_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    let mut parsed = ParsedDependencies::default();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "package_declaration" => parsed.java_package = java_qualified_name(node, source),
            "import_declaration" => {
                if let Some(reference) = java_import(node, source) {
                    parsed.references.push(reference);
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    parsed
                        .java_top_level_types
                        .push(node_text(name, source).to_owned());
                }
            }
            _ => {}
        }
    }
    parsed
}

fn java_qualified_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| matches!(child.kind(), "identifier" | "scoped_identifier"))
        .map(|name| node_text(name, source).to_owned())
}

fn java_import(node: Node<'_>, source: &[u8]) -> Option<RawDependency> {
    let specifier = java_qualified_name(node, source)?;
    let mut cursor = node.walk();
    let wildcard = node
        .named_children(&mut cursor)
        .any(|child| child.kind() == "asterisk");
    let mut cursor = node.walk();
    let is_static = node
        .children(&mut cursor)
        .any(|child| child.kind() == "static");
    Some(RawDependency {
        kind: if wildcard {
            RawDependencyKind::JavaWildcard
        } else if is_static {
            RawDependencyKind::JavaStatic
        } else {
            RawDependencyKind::Java
        },
        specifier: if wildcard {
            format!("{specifier}.*")
        } else {
            specifier
        },
        line: node.start_position().row as u32 + 1,
    })
}
