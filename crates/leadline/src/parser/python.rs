//! Python imports and the conventions that make a module runnable.
//!
//! Only relative imports name a file from inside the source: `from ..pkg.mod
//! import x` is resolved against the declaring file's package, so the extractor
//! records the module as a path specifier with the import level encoded as
//! leading `../` segments (level 1 stays in the declaring file's own directory,
//! each deeper level climbs one directory). Absolute imports name a module on
//! `sys.path`, which a parser cannot see, so they are recorded as
//! [`RawDependencyKind::PythonAbsoluteImport`] and reported as unresolved
//! instead of disappearing: silently dropping them would leave a graph that
//! looks complete while the analysis is not.
//!
//! A module is also runnable without any importer: `python -m package` runs
//! `__main__.py`, the framework loaders run `manage.py`, `conftest.py`,
//! `setup.py`, `wsgi.py`, and `asgi.py` by name, and a module whose top level
//! holds an `if __name__ == "__main__":` guard executes when it is run
//! directly. [`is_conventional_entry`] and [`has_main_guard`] name those two
//! conventions for `unused`.

use super::{
    ParsedDependencies, RawDependency, RawDependencyKind, node_text, push_children_reversed,
};
use tree_sitter::Node;

/// File names Python projects run without an import edge: `__main__.py` is
/// what `python -m package` executes, the web and tooling frameworks load
/// their own `manage.py`, `wsgi.py`, `asgi.py`, `conftest.py`, and `setup.py`
/// by name.
const CONVENTIONAL_ENTRY_NAMES: [&str; 6] = [
    "__main__.py",
    "manage.py",
    "conftest.py",
    "setup.py",
    "wsgi.py",
    "asgi.py",
];

/// True for the file names a host loads or runs by name rather than by import.
pub(crate) fn is_conventional_entry(path: &str) -> bool {
    let (_, name) = path.rsplit_once('/').unwrap_or(("", path));
    CONVENTIONAL_ENTRY_NAMES.contains(&name)
}

/// True when the module body holds an `if __name__ == "__main__":` guard.
///
/// The guard is how a module states that running it directly executes the code
/// below, so the file is reachable without an importer: `python mod.py`,
/// `python -m pkg.mod`, and a console-script target all take that path.
pub(crate) fn has_main_guard(root: Node<'_>, source: &[u8]) -> bool {
    let mut cursor = root.walk();
    for statement in root.named_children(&mut cursor) {
        if statement.kind() == "if_statement" && is_main_guard(statement, source) {
            return true;
        }
    }
    false
}

/// True when an `if` statement's condition compares `__name__` with the
/// literal `"__main__"`.
fn is_main_guard(statement: Node<'_>, source: &[u8]) -> bool {
    let Some(condition) = statement.child_by_field_name("condition") else {
        return false;
    };
    contains_main_comparison(condition, source)
}

/// True when `node` or a descendant is a `==` comparison of `__name__` with
/// `"__main__"`.
///
/// The comparison may sit below a Boolean operator (`if __name__ == "__main__"
/// and sys.argv[1:]`), so the search covers the whole condition.
fn contains_main_comparison(node: Node<'_>, source: &[u8]) -> bool {
    if node.kind() == "comparison_operator" && is_main_comparison(node, source) {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if contains_main_comparison(child, source) {
            return true;
        }
    }
    false
}

/// True when one `comparison_operator` carries `__name__`, `==`, and
/// `"__main__"` as its own operands.
fn is_main_comparison(node: Node<'_>, source: &[u8]) -> bool {
    let mut names = false;
    let mut mains = false;
    let mut equal = false;
    for index in 0..node.child_count() {
        let Some(child) = node.child(index) else {
            continue;
        };
        let operand = unparenthesized(child);
        match operand.kind() {
            "identifier" => names |= node_text(operand, source) == "__name__",
            "string" => mains |= string_content(operand, source) == Some("__main__"),
            "==" => equal = true,
            _ => {}
        }
    }
    names && mains && equal
}

/// The operand inside any parentheses wrapping it, so `(__name__)` compares
/// like `__name__`.
fn unparenthesized(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    while current.kind() == "parenthesized_expression" {
        let Some(inner) = current.named_child(0) else {
            break;
        };
        current = inner;
    }
    current
}

/// Text of a `string` literal, or `None` when it interpolates a value.
fn string_content<'a>(node: Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    let mut content = None;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            // An f-string computes its text, so it never stands for a literal.
            "interpolation" => return None,
            "string_content" if content.is_none() => content = Some(node_text(child, source)),
            _ => {}
        }
    }
    content
}

/// Records Python imports as dependency references.
///
/// A `from` statement whose module is relative emits one reference for the
/// module it names; the bare form (`from . import a, b`) names submodules
/// instead, so it emits one reference per imported name. Every absolute
/// import emits one reference for the module it names, with the dotted path
/// left as written: only the graph knows that such a specifier cannot be
/// resolved without `sys.path`.
pub(super) fn extract_relative_imports(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    let mut parsed = ParsedDependencies::default();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "import_from_statement" => collect_from_import(node, source, &mut parsed),
            "import_statement" => collect_import(node, source, &mut parsed),
            _ => {}
        }
        push_children_reversed(node, &mut stack);
    }
    parsed
}

/// Records `from <module> import <names>`.
fn collect_from_import(node: Node<'_>, source: &[u8], parsed: &mut ParsedDependencies) {
    let Some(module) = node.child_by_field_name("module_name") else {
        return;
    };
    if module.kind() != "relative_import" {
        parsed.references.push(RawDependency {
            kind: RawDependencyKind::PythonAbsoluteImport,
            specifier: node_text(module, source).to_owned(),
            line: node.start_position().row as u32 + 1,
        });
        return;
    }
    let prefix = level_prefix(relative_level(module));
    let Some(dotted) = child_of_kind(module, "dotted_name") else {
        // `from . import a, b` imports submodules of the package the level
        // names, so each name is its own reference.
        for_each_field_child(node, "name", |name| {
            parsed.references.push(RawDependency {
                kind: RawDependencyKind::PythonImport,
                specifier: format!("{prefix}{}", node_text(imported_module(name), source)),
                line: name.start_position().row as u32 + 1,
            });
        });
        return;
    };
    parsed.references.push(RawDependency {
        kind: RawDependencyKind::PythonImport,
        specifier: format!("{prefix}{}", module_path(node_text(dotted, source))),
        line: dotted.start_position().row as u32 + 1,
    });
}

/// Records `import a.b, c as d`.
fn collect_import(node: Node<'_>, source: &[u8], parsed: &mut ParsedDependencies) {
    for_each_field_child(node, "name", |name| {
        parsed.references.push(RawDependency {
            kind: RawDependencyKind::PythonAbsoluteImport,
            specifier: node_text(imported_module(name), source).to_owned(),
            line: name.start_position().row as u32 + 1,
        });
    });
}

/// The module path a name imports: `os.path` for both `os.path` and
/// `os.path as osp`.
fn imported_module(node: Node<'_>) -> Node<'_> {
    if node.kind() == "aliased_import" {
        return node.child_by_field_name("name").unwrap_or(node);
    }
    node
}

/// Import level of a `relative_import`: one `.` per level, so `.` is 1 and
/// `..` is 2.
fn relative_level(module: Node<'_>) -> usize {
    let Some(prefix) = child_of_kind(module, "import_prefix") else {
        return 0;
    };
    (0..prefix.child_count())
        .filter(|index| {
            prefix
                .child(*index)
                .is_some_and(|child| child.kind() == ".")
        })
        .count()
}

/// Leading `../` segments that carry an import level in a specifier: level 1
/// names a module beside the declaring file, each deeper level climbs one
/// directory before the module path applies.
fn level_prefix(level: usize) -> String {
    "../".repeat(level.saturating_sub(1))
}

/// A dotted module path as a specifier path: `pkg.sub` names `pkg/sub.py` or
/// `pkg/sub/__init__.py`.
fn module_path(dotted: &str) -> String {
    dotted.replace('.', "/")
}

/// First named child of `kind`.
fn child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

/// Every child carrying `field`, in source order.
///
/// A node repeats a field name once per value (`from . import a, b` carries
/// two `name` fields), which `child_by_field_name` alone does not reach.
fn for_each_field_child<'tree>(
    node: Node<'tree>,
    field: &str,
    mut action: impl FnMut(Node<'tree>),
) {
    for index in 0..node.child_count() {
        if node.field_name_for_child(index) != Some(field) {
            continue;
        }
        if let Some(child) = node.child(index) {
            action(child);
        }
    }
}
