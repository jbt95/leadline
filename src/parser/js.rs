use super::{
    ParsedDependencies, RawDependency, RawDependencyKind, node_text, push_children_reversed,
};
use tree_sitter::Node;

pub(super) fn extract_javascript_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
    let mut references = Vec::new();
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
        push_children_reversed(node, &mut stack);
    }
    ParsedDependencies {
        references,
        ..ParsedDependencies::default()
    }
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
