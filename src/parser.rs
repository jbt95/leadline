use crate::Result;
use crate::core::{
    DecisionKind, Event, FileAnalysis, FunctionInput, FunctionKind, Language, LogicalOperator,
    ParseDiagnostic, Span, analyze_function,
};
use std::cell::RefCell;
use std::path::Path;
use tree_sitter::{Language as TsLanguage, Node, Parser, Tree};

thread_local! {
    static THREAD_PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RawDependencyKind {
    JavaScriptImport,
    JavaScriptCall,
    JavaScriptUndecodable,
    Java,
    JavaStatic,
    JavaWildcard,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawDependency {
    pub(crate) kind: RawDependencyKind,
    pub(crate) specifier: String,
    pub(crate) line: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ParsedDependencies {
    pub(crate) references: Vec<RawDependency>,
    pub(crate) java_package: Option<String>,
    pub(crate) java_top_level_types: Vec<String>,
}

pub trait ParserBackend: Sync {
    fn analyze(&self, path: &str, source: &[u8]) -> Result<FileAnalysis>;
}

pub struct TreeSitterBackend;

impl ParserBackend for TreeSitterBackend {
    fn analyze(&self, path: &str, source: &[u8]) -> Result<FileAnalysis> {
        let (language, tree) = parse_tree(path, source)?;
        let mut nodes = Vec::new();
        let mut parse_errors = Vec::new();
        discover_tree(tree.root_node(), language, &mut nodes, &mut parse_errors);
        debug_assert!(
            nodes
                .windows(2)
                .all(|pair| pair[0].start_byte() <= pair[1].start_byte()),
            "discover_tree must visit functions in source order"
        );
        let functions = nodes
            .into_iter()
            .map(|node| analyze_node(path, node, language, source))
            .collect();
        Ok(FileAnalysis {
            path: path.to_owned(),
            language,
            functions,
            parse_errors,
        })
    }
}

fn parse_tree(path: &str, source: &[u8]) -> Result<(Language, Tree)> {
    let language = detect_language(path).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsupported source extension",
        )
    })?;
    let tree = THREAD_PARSER.with(|cell| -> Result<Tree> {
        let mut parser = cell.borrow_mut();
        parser.set_language(&grammar(language))?;
        parser.parse(source, None).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Tree-sitter returned no tree",
            )
            .into()
        })
    })?;
    Ok((language, tree))
}

/// One normalized leaf token for duplication analysis (`tokens`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NormalizedToken {
    /// `<id>`, `<str>`, `<num>`, or the exact keyword/punctuation text.
    pub(crate) text: String,
    pub(crate) line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TokenizedSource {
    pub(crate) language: Language,
    pub(crate) tokens: Vec<NormalizedToken>,
    /// `ERROR`/`MISSING` leaf count; non-zero excludes the file from clones.
    pub(crate) parse_errors: usize,
    pub(crate) line_count: u32,
}

/// Extracts `tokens` normalized leaf tokens from `source`.
pub(crate) fn normalized_tokens(path: &str, source: &[u8]) -> Result<TokenizedSource> {
    let (language, tree) = parse_tree(path, source)?;
    let root = tree.root_node();
    let mut tokens = Vec::new();
    let mut parse_errors = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            parse_errors += 1;
        }
        if node.child_count() == 0 {
            if let Some(text) = normalize_leaf(node, source) {
                tokens.push(NormalizedToken {
                    text,
                    line: node.start_position().row as u32 + 1,
                });
            }
        } else {
            for index in (0..node.child_count()).rev() {
                stack.push(node.child(index).expect("child index is in bounds"));
            }
        }
    }
    debug_assert!(
        tokens.windows(2).all(|pair| pair[0].line <= pair[1].line),
        "token walk must visit leaves in line order"
    );
    Ok(TokenizedSource {
        language,
        tokens,
        parse_errors,
        line_count: source.iter().filter(|byte| **byte == b'\n').count() as u32 + 1,
    })
}

fn normalize_leaf(node: Node<'_>, source: &[u8]) -> Option<String> {
    let kind = node.kind();
    if kind.contains("comment") {
        return None;
    }
    if matches!(
        kind,
        "identifier"
            | "property_identifier"
            | "type_identifier"
            | "field_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern"
            | "statement_identifier"
    ) {
        return Some("<id>".to_owned());
    }
    if kind.contains("string") || kind == "template_chars" || kind == "string_fragment" {
        return Some("<str>".to_owned());
    }
    if kind.contains("number")
        || kind == "integer"
        || kind == "float"
        || kind == "decimal_integer_literal"
    {
        return Some("<num>".to_owned());
    }
    let text = node.utf8_text(source).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_owned())
}

pub(crate) fn extract_dependencies(path: &str, source: &[u8]) -> Result<ParsedDependencies> {
    let (language, tree) = parse_tree(path, source)?;
    let root = tree.root_node();
    Ok(match language {
        Language::Java => extract_java_dependencies(root, source),
        Language::JavaScript | Language::TypeScript | Language::Tsx => {
            extract_javascript_dependencies(root, source)
        }
    })
}

fn extract_javascript_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
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
        for index in (0..node.child_count()).rev() {
            stack.push(node.child(index).expect("child index is in bounds"));
        }
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

fn extract_java_dependencies(root: Node<'_>, source: &[u8]) -> ParsedDependencies {
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

pub fn detect_language(path: &str) -> Option<Language> {
    match Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "java" => Some(Language::Java),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
        "ts" | "mts" | "cts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        _ => None,
    }
}

fn grammar(language: Language) -> TsLanguage {
    match language {
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

fn discover_tree<'tree>(
    root: Node<'tree>,
    language: Language,
    functions: &mut Vec<Node<'tree>>,
    parse_errors: &mut Vec<ParseDiagnostic>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if is_function(node.kind(), language) {
            functions.push(node);
        }
        if node.is_error() || node.is_missing() {
            let start = node.start_position();
            let end = node.end_position();
            parse_errors.push(ParseDiagnostic {
                kind: if node.is_missing() {
                    "missing"
                } else {
                    "error"
                },
                start_line: start.row as u32 + 1,
                start_column: start.column as u32 + 1,
                end_line: end.row as u32 + 1,
                end_column: end.column as u32 + 1,
            });
        }
        for index in (0..node.child_count()).rev() {
            stack.push(node.child(index).expect("child index is in bounds"));
        }
    }
}
fn is_function(kind: &str, language: Language) -> bool {
    match language {
        Language::Java => matches!(
            kind,
            "method_declaration"
                | "constructor_declaration"
                | "compact_constructor_declaration"
                | "lambda_expression"
        ),
        Language::JavaScript | Language::TypeScript | Language::Tsx => matches!(
            kind,
            "function_declaration"
                | "function_expression"
                | "generator_function"
                | "generator_function_declaration"
                | "arrow_function"
                | "method_definition"
        ),
    }
}

fn analyze_node(
    display_path: &str,
    node: Node<'_>,
    language: Language,
    source: &[u8],
) -> crate::core::FunctionAnalysis {
    let mut events = Vec::new();
    let mut logical_loc = 0;
    walk_function(node, language, source, &mut logical_loc, &mut events);
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let kind = function_kind(node);
    let start_byte = node.start_byte() as u64;
    let end_byte = node.end_byte() as u64;
    analyze_function(
        FunctionInput {
            name: function_name(node, source),
            id: crate::core::function_id(display_path, kind, start_byte, end_byte),
            kind,
            start_line,
            end_line,
            start_byte,
            end_byte,
            parameters: parameter_count(node),
            logical_loc,
            events,
            source_fingerprint: fingerprint(&source[node.byte_range()]),
        },
        source,
    )
}

fn function_kind(node: Node<'_>) -> FunctionKind {
    match node.kind() {
        "method_definition" => FunctionKind::Method,
        "constructor_declaration" | "compact_constructor_declaration" => FunctionKind::Constructor,
        "lambda_expression" => FunctionKind::Lambda,
        "arrow_function" => FunctionKind::Arrow,
        "function_expression" if node.child_by_field_name("name").is_none() => {
            FunctionKind::Anonymous
        }
        _ => FunctionKind::Function,
    }
}

fn walk_function(
    root: Node<'_>,
    language: Language,
    source: &[u8],
    logical_loc: &mut u32,
    events: &mut Vec<Event>,
) {
    let mut next_sequence = 0;
    let mut stack = vec![(root, 0, false)];
    while let Some((node, nesting, inside_logical)) = stack.pop() {
        if node.id() != root.id() && is_function(node.kind(), language) {
            continue;
        }

        if is_logical_loc(node.kind()) {
            *logical_loc += 1;
        }

        let else_if = is_else_if(node);
        let decision = decision_kind(node, source);
        if let Some(kind) = decision {
            events.push(Event::Decision {
                kind,
                nesting,
                else_if,
                line: node.start_position().row as u32 + 1,
            });
        }
        if node.kind() == "if_statement" {
            if let Some(alternative) = node.child_by_field_name("alternative")
                && alternative.kind() != "if_statement"
                && alternative.kind() != "else_clause"
            {
                events.push(Event::Else {
                    line: node.start_position().row as u32 + 1,
                    nesting,
                });
            }
        } else if node.kind() == "else_clause" && !has_if_child(node) {
            events.push(Event::Else {
                line: node.start_position().row as u32 + 1,
                nesting,
            });
        }
        if matches!(node.kind(), "break_statement" | "continue_statement")
            && node.named_child_count() > 0
        {
            events.push(Event::LabeledJump {
                line: node.start_position().row as u32 + 1,
                nesting,
            });
        }

        let this_logical = logical_operator(node, source).is_some();
        if this_logical && !inside_logical {
            collect_logical(node, source, next_sequence, nesting, events);
            next_sequence += 1;
        }

        if node.child_count() == 0 {
            let span = Span {
                start: node.start_byte(),
                end: node.end_byte(),
            };
            if is_operator(node.kind()) {
                events.push(Event::Operator(span));
            } else if node.is_named() && is_operand(node.kind()) {
                events.push(Event::Operand(span));
            }
            continue;
        }

        let structural = matches!(
            decision,
            Some(
                DecisionKind::If
                    | DecisionKind::Loop
                    | DecisionKind::Catch
                    | DecisionKind::Switch
                    | DecisionKind::Ternary
            )
        );
        let child_nesting = if structural && !else_if {
            nesting + 1
        } else {
            nesting
        };
        if structural {
            events.push(Event::NestingDepth(child_nesting));
        }

        for index in (0..node.child_count()).rev() {
            stack.push((
                node.child(index).expect("child index is in bounds"),
                child_nesting,
                inside_logical || this_logical,
            ));
        }
    }
}

fn decision_kind(node: Node<'_>, source: &[u8]) -> Option<DecisionKind> {
    match node.kind() {
        "if_statement" => Some(DecisionKind::If),
        "for_statement"
        | "for_in_statement"
        | "enhanced_for_statement"
        | "while_statement"
        | "do_statement" => Some(DecisionKind::Loop),
        "catch_clause" => Some(DecisionKind::Catch),
        "switch_statement" | "switch_expression" => Some(DecisionKind::Switch),
        "switch_case" => Some(DecisionKind::Case),
        "switch_label" if node_text(node, source).trim_start().starts_with("case") => {
            Some(DecisionKind::Case)
        }
        "ternary_expression" => Some(DecisionKind::Ternary),
        _ => None,
    }
}

fn is_else_if(node: Node<'_>) -> bool {
    if node.kind() != "if_statement" {
        return false;
    }
    node.parent().is_some_and(|parent| {
        parent.kind() == "else_clause"
            || parent
                .child_by_field_name("alternative")
                .is_some_and(|alternative| alternative.id() == node.id())
    })
}

fn has_if_child(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| child.kind() == "if_statement")
}

fn logical_operator(node: Node<'_>, source: &[u8]) -> Option<LogicalOperator> {
    if !matches!(node.kind(), "binary_expression" | "binary_expression2") {
        return None;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| match node_text(child, source) {
            "&&" => Some(LogicalOperator::And),
            "||" => Some(LogicalOperator::Or),
            "??" => Some(LogicalOperator::Nullish),
            _ => None,
        })
}

fn collect_logical(
    node: Node<'_>,
    source: &[u8],
    sequence: u32,
    nesting: u32,
    events: &mut Vec<Event>,
) {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.child_count() == 0 {
            let operator = match node_text(current, source) {
                "&&" => Some(LogicalOperator::And),
                "||" => Some(LogicalOperator::Or),
                "??" => Some(LogicalOperator::Nullish),
                _ => None,
            };
            if let Some(operator) = operator {
                events.push(Event::Logical {
                    operator,
                    sequence,
                    line: current.start_position().row as u32 + 1,
                    nesting,
                });
            }
            continue;
        }
        for index in (0..current.child_count()).rev() {
            stack.push(current.child(index).expect("child index is in bounds"));
        }
    }
}

fn function_name(node: Node<'_>, source: &[u8]) -> String {
    if let Some(name) = node.child_by_field_name("name") {
        return node_text(name, source).to_owned();
    }
    let mut parent = node.parent();
    for _ in 0..2 {
        if let Some(current) = parent {
            if matches!(
                current.kind(),
                "variable_declarator" | "pair" | "field_definition" | "assignment_expression"
            ) && let Some(name) = current
                .child_by_field_name("name")
                .or_else(|| current.child_by_field_name("left"))
                .or_else(|| current.child_by_field_name("key"))
            {
                return node_text(name, source).to_owned();
            }
            parent = current.parent();
        }
    }
    let point = node.start_position();
    format!("<anonymous@{}:{}>", point.row + 1, point.column + 1)
}

fn parameter_count(node: Node<'_>) -> u32 {
    if let Some(parameters) = node.child_by_field_name("parameters") {
        return if matches!(
            parameters.kind(),
            "identifier" | "formal_parameter" | "spread_parameter"
        ) {
            1
        } else {
            parameters.named_child_count() as u32
        };
    }
    if node.kind() == "compact_constructor_declaration" {
        let mut ancestor = node.parent();
        while let Some(current) = ancestor {
            if current.kind() == "record_declaration" {
                return current
                    .child_by_field_name("parameters")
                    .map_or(0, |parameters| parameters.named_child_count() as u32);
            }
            ancestor = current.parent();
        }
    }
    if node.kind() == "lambda_expression" {
        return node.named_child(0).map_or(0, |parameters| {
            if matches!(
                parameters.kind(),
                "formal_parameters" | "inferred_parameters"
            ) {
                parameters.named_child_count() as u32
            } else {
                1
            }
        });
    }
    node.child_by_field_name("parameter").map_or(0, |_| 1)
}

fn is_logical_loc(kind: &str) -> bool {
    matches!(
        kind,
        "expression_statement"
            | "return_statement"
            | "throw_statement"
            | "variable_declaration"
            | "lexical_declaration"
            | "local_variable_declaration"
            | "if_statement"
            | "for_statement"
            | "for_in_statement"
            | "enhanced_for_statement"
            | "while_statement"
            | "do_statement"
            | "switch_statement"
            | "switch_expression"
            | "switch_case"
            | "switch_label"
            | "catch_clause"
            | "break_statement"
            | "continue_statement"
    )
}

fn is_operator(kind: &str) -> bool {
    matches!(
        kind,
        "+" | "-"
            | "*"
            | "/"
            | "%"
            | "**"
            | "="
            | "+="
            | "-="
            | "*="
            | "/="
            | "%="
            | "=="
            | "!="
            | "==="
            | "!=="
            | "<"
            | ">"
            | "<="
            | ">="
            | "&&"
            | "||"
            | "??"
            | "!"
            | "~"
            | "&"
            | "|"
            | "^"
            | "<<"
            | ">>"
            | ">>>"
            | "++"
            | "--"
            | "?"
            | ":"
            | "=>"
            | "new"
            | "return"
            | "throw"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "switch"
            | "case"
            | "catch"
            | "break"
            | "continue"
            | "instanceof"
            | "in"
            | "typeof"
            | "void"
            | "delete"
            | "await"
            | "yield"
    )
}

fn is_operand(kind: &str) -> bool {
    kind.ends_with("identifier")
        || matches!(
            kind,
            "identifier"
                | "number"
                | "decimal_integer_literal"
                | "hex_integer_literal"
                | "decimal_floating_point_literal"
                | "string"
                | "string_fragment"
                | "character_literal"
                | "template_string"
                | "true"
                | "false"
                | "null"
                | "this"
                | "super"
        )
}

fn node_text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.byte_range()]).unwrap_or("")
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
