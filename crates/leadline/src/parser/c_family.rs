//! Facts shared by the C and C++ grammars.

use super::{ParsedDependencies, RawDependency, RawDependencyKind, node_text};
use tree_sitter::Node;

/// Records quoted preprocessor includes as file-relative references.
///
/// Both grammars spell a quoted include as a `string_literal` path, so one walk
/// serves both languages. `#include "path"` is file-relative and becomes a
/// graph edge; angle-bracket includes and macro-built paths do not, because the
/// system search path and the macro expansion environment are unknown to a
/// parser.
pub(super) fn extract_local_includes(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
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

/// True for a file a build system compiles as a translation unit.
///
/// `.c`, `.cpp`, `.cc`, and `.cxx` are compiled rather than included, so no
/// include edge can ever reach them; `unused` treats them as entry points for
/// the same reason Cargo crate roots are entry points for Rust.
pub(crate) fn is_translation_unit(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| matches!(extension, "c" | "cpp" | "cc" | "cxx"))
}
