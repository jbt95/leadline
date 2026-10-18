use super::{
    ExportKind, ExportedSymbol, ImportedSymbol, ParsedDependencies, RawDependency,
    RawDependencyKind, node_text, push_children_reversed,
};
use tree_sitter::Node;

pub(super) fn extract_javascript_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    let mut references = Vec::new();
    let mut exports = Vec::new();
    let mut imports = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let found = match node.kind() {
            "import_statement" | "export_statement" => node
                .child_by_field_name("source")
                .map(|literal| (RawDependencyKind::JavaScriptImport, literal)),
            "call_expression" if is_dependency_call(node, source) => node
                .child_by_field_name("arguments")
                .and_then(single_string_argument)
                .map(|literal| (RawDependencyKind::JavaScriptCall, literal)),
            _ => None,
        };
        if let Some((kind, literal)) = found {
            let line = node.start_position().row as u32 + 1;
            match decode_javascript_string(literal, source) {
                Some(specifier) => references.push(RawDependency {
                    kind,
                    specifier,
                    line,
                }),
                // A literal in dependency position that cannot be decoded
                // (bad hex, lone surrogate, unclosed escape) still names a
                // reference; keep the raw text so graph reports it instead
                // of silently dropping it.
                None => references.push(RawDependency {
                    kind: RawDependencyKind::JavaScriptUndecodable,
                    specifier: raw_string_inner_text(literal, source),
                    line,
                }),
            }
        }
        match node.kind() {
            "export_statement" => {
                collect_exported_symbols(node, source, &mut exports, &mut imports);
            }
            "import_statement" => collect_imported_symbols(node, source, &mut imports),
            _ => {}
        }
        push_children_reversed(node, &mut stack);
    }
    ParsedDependencies {
        references,
        exports,
        imports,
        ..ParsedDependencies::default()
    }
}

/// Exported names of one `export_statement`, plus the names it re-exports
/// when it names another module.
///
/// A star re-export (`export * from "./x"`, `export * as ns from "./x"`)
/// exports no single name; its entry carries the module specifier so callers
/// can find the file whose exports it re-exports wholesale.
fn collect_exported_symbols(
    node: Node<'_>,
    source: &[u8],
    exports: &mut Vec<ExportedSymbol>,
    imports: &mut Vec<ImportedSymbol>,
) {
    let line = node.start_position().row as u32 + 1;
    if node.child_by_field_name("value").is_some() || child_of_kind(node, "default").is_some() {
        exports.push(ExportedSymbol {
            name: "default".to_owned(),
            line,
            kind: ExportKind::Default,
        });
        return;
    }
    if let Some(clause) = child_of_kind(node, "export_clause") {
        let from = node.child_by_field_name("source").is_some();
        let mut cursor = clause.walk();
        for specifier in clause.named_children(&mut cursor) {
            if specifier.kind() != "export_specifier" {
                continue;
            }
            let exported = specifier
                .child_by_field_name("alias")
                .or_else(|| specifier.child_by_field_name("name"));
            if let Some(exported) = exported {
                exports.push(ExportedSymbol {
                    name: name_text(exported, source),
                    line,
                    kind: ExportKind::Named,
                });
            }
            let Some(imported) = specifier.child_by_field_name("name") else {
                continue;
            };
            if from {
                imports.push(ImportedSymbol {
                    name: name_text(imported, source),
                    line,
                    star: false,
                });
            }
        }
        return;
    }
    if let Some(literal) = node.child_by_field_name("source") {
        exports.push(ExportedSymbol {
            name: specifier_text(literal, source),
            line,
            kind: ExportKind::Star,
        });
        imports.push(ImportedSymbol {
            name: String::new(),
            line,
            star: true,
        });
        return;
    }
    let Some(declaration) = node.child_by_field_name("declaration") else {
        return;
    };
    for name in declaration_names(declaration, source) {
        exports.push(ExportedSymbol {
            name,
            line,
            kind: ExportKind::Named,
        });
    }
}

/// Names an exported declaration introduces. Destructuring patterns, and
/// declarations outside `function`/`class`/`var` bindings, introduce no
/// single name and yield none.
fn declaration_names(declaration: Node<'_>, source: &[u8]) -> Vec<String> {
    match declaration.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "class_declaration"
        | "abstract_class_declaration" => declaration
            .child_by_field_name("name")
            .map(|name| vec![name_text(name, source)])
            .unwrap_or_default(),
        "lexical_declaration" | "variable_declaration" => {
            let mut names = Vec::new();
            let mut cursor = declaration.walk();
            for declarator in declaration.named_children(&mut cursor) {
                if declarator.kind() != "variable_declarator" {
                    continue;
                }
                let Some(name) = declarator.child_by_field_name("name") else {
                    continue;
                };
                if name.kind() == "identifier" {
                    names.push(name_text(name, source));
                }
            }
            names
        }
        _ => Vec::new(),
    }
}

/// Names one `import_statement` reads from its module.
fn collect_imported_symbols(node: Node<'_>, source: &[u8], imports: &mut Vec<ImportedSymbol>) {
    let line = node.start_position().row as u32 + 1;
    let Some(clause) = child_of_kind(node, "import_clause") else {
        // `import "./side-effect"` binds nothing.
        return;
    };
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        match child.kind() {
            // The default binding reads the module's `default` export; the
            // local alias it is bound to proves nothing about the module.
            "identifier" => imports.push(ImportedSymbol {
                name: "default".to_owned(),
                line,
                star: false,
            }),
            // A namespace binds the whole module, so no single name is read.
            "namespace_import" => imports.push(ImportedSymbol {
                name: String::new(),
                line,
                star: true,
            }),
            "named_imports" => {
                let mut specs = child.walk();
                for specifier in child.named_children(&mut specs) {
                    if specifier.kind() != "import_specifier" {
                        continue;
                    }
                    let Some(name) = specifier.child_by_field_name("name") else {
                        continue;
                    };
                    imports.push(ImportedSymbol {
                        name: name_text(name, source),
                        line,
                        star: false,
                    });
                }
            }
            _ => {}
        }
    }
}

/// First direct child of `node` with `kind`, anonymous children included.
fn child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

/// Text of a symbol node: a string form (`export { a as "b" }`) loses its
/// quotes so it matches the name it declares.
fn name_text(node: Node<'_>, source: &[u8]) -> String {
    if node.kind() == "string" {
        raw_string_inner_text(node, source)
    } else {
        node_text(node, source).to_owned()
    }
}

/// Module specifier of a `from` clause, decoded when the literal allows it.
fn specifier_text(literal: Node<'_>, source: &[u8]) -> String {
    decode_javascript_string(literal, source)
        .unwrap_or_else(|| raw_string_inner_text(literal, source))
}

fn is_dependency_call(node: Node<'_>, source: &[u8]) -> bool {
    node.child_by_field_name("function")
        .is_some_and(|function| {
            function.kind() == "import"
                || (function.kind() == "identifier" && node_text(function, source) == "require")
        })
}

fn single_string_argument(arguments: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = arguments.walk();
    let mut named = arguments.named_children(&mut cursor);
    let argument = named.next()?;
    (argument.kind() == "string" && named.next().is_none()).then_some(argument)
}

fn raw_string_inner_text(node: Node<'_>, source: &[u8]) -> String {
    let text = node_text(node, source);
    let bytes = text.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'\'' || bytes[0] == b'"')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        text[1..text.len() - 1].to_owned()
    } else {
        text.to_owned()
    }
}

fn decode_javascript_string(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let text = node_text(node, source);
    let quote = text.chars().next()?;
    if !matches!(quote, '\'' | '"') || !text.ends_with(quote) || text.len() < 2 {
        return None;
    }
    decode_javascript_escapes(&text[1..text.len() - 1])
}

fn decode_javascript_escapes(text: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            '\n' | '\r' => {
                if escaped == '\r' && chars.clone().next() == Some('\n') {
                    chars.next();
                }
            }
            'b' => decoded.push('\u{0008}'),
            'f' => decoded.push('\u{000c}'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            'v' => decoded.push('\u{000b}'),
            '0' => decoded.push('\0'),
            'x' => decoded.push(char::from_u32(decode_hex(&mut chars, 2)?)?),
            'u' if chars.clone().next() == Some('{') => {
                chars.next();
                let mut value = String::new();
                let mut closed = false;
                for digit in chars.by_ref() {
                    if digit == '}' {
                        closed = true;
                        break;
                    }
                    value.push(digit);
                }
                if !closed {
                    return None;
                }
                decoded.push(char::from_u32(u32::from_str_radix(&value, 16).ok()?)?);
            }
            'u' => {
                let first = decode_hex(&mut chars, 4)?;
                let value = if (0xd800..=0xdbff).contains(&first) {
                    if chars.next()? != '\\' || chars.next()? != 'u' {
                        return None;
                    }
                    let second = decode_hex(&mut chars, 4)?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return None;
                    }
                    0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
                } else {
                    first
                };
                decoded.push(char::from_u32(value)?);
            }
            other => decoded.push(other),
        }
    }
    Some(decoded)
}

fn decode_hex(chars: &mut impl Iterator<Item = char>, digits: usize) -> Option<u32> {
    let mut value = 0_u32;
    for _ in 0..digits {
        value = value.checked_mul(16)? + chars.next()?.to_digit(16)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::decode_javascript_escapes;

    /// Every escape form a specifier can carry. The decoder decides which
    /// file an import names, so each arm needs its own case.
    #[test]
    fn decodes_every_escape_form() {
        for (literal, expected) in [
            ("plain", "plain"),
            (r"\b\f\n\r\t\v\0", "\u{0008}\u{000c}\n\r\t\u{000b}\0"),
            (r"a\r\nb", "a\r\nb"),
            ("a\\\r\nb", "ab"),
            ("a\\\rb", "ab"),
            ("a\\\nb", "ab"),
            (r"\x41\x7a", "Az"),
            (r"\u0041\u{42}", "AB"),
            (r"\u{1F600}", "\u{1F600}"),
            (r"\uD83D\uDE00", "\u{1F600}"),
            (r"\\ \/ \. \!", "\\ / . !"),
        ] {
            assert_eq!(
                decode_javascript_escapes(literal).as_deref(),
                Some(expected),
                "{literal:?}"
            );
        }
    }

    /// A malformed form must refuse the specifier. Decoding it partially
    /// would resolve the import to a file that does not exist, so every
    /// rejection path needs a case.
    #[test]
    fn rejects_malformed_escapes() {
        for literal in [
            r"\",
            r"\xZZ",
            r"\x4",
            r"\u00",
            r"\u{41",
            r"\u{}",
            r"\u{110000}",
            r"\uD83D",
            r"\uD83Dx",
            r"\uD83D\u0041",
        ] {
            assert_eq!(
                decode_javascript_escapes(literal),
                None,
                "{literal:?} must not decode"
            );
        }
    }
}
