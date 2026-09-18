use crate::Result;
use crate::core::{
    DecisionKind, Event, FileAnalysis, FunctionInput, FunctionKind, Language, LogicalOperator,
    ParseDiagnostic, Span, analyze_function,
};
use crate::source_snapshot::SourceEntry;
use rayon::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use tree_sitter::{Language as TsLanguage, Node, Parser, Tree};

mod java;
mod js;

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
        Ok(analyze_tree(path, language, source, tree.root_node()))
    }
}

/// Discovers functions and parse diagnostics in an already-parsed tree.
fn analyze_tree(path: &str, language: Language, source: &[u8], root: Node<'_>) -> FileAnalysis {
    let mut nodes = Vec::new();
    let mut parse_errors = Vec::new();
    discover_tree(root, language, &mut nodes, &mut parse_errors);
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
    FileAnalysis {
        path: path.to_owned(),
        language,
        functions,
        parse_errors,
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

/// Child count at or below which direct indexing is cheaper than a cursor
/// pass. `Node::child(i)` rescans from the first child, so the index loop is
/// quadratic on the wide nodes error recovery produces; ordinary nodes stay
/// on the indexed fast path.
const CURSOR_CHILD_THRESHOLD: u32 = 8;

/// Pushes `node`'s children onto `stack` so the next pop yields the first
/// child.
#[inline]
pub(super) fn push_children_reversed<'tree>(node: Node<'tree>, stack: &mut Vec<Node<'tree>>) {
    let count = node.child_count();
    if count <= CURSOR_CHILD_THRESHOLD {
        for index in (0..count).rev() {
            stack.push(node.child(index).expect("child index is in bounds"));
        }
        return;
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        let start = stack.len();
        stack.push(cursor.node());
        while cursor.goto_next_sibling() {
            stack.push(cursor.node());
        }
        stack[start..].reverse();
    }
}

/// Tuple-stack variant for walks that carry nesting state.
#[inline]
fn push_children_reversed_with<'tree>(
    node: Node<'tree>,
    nesting: u32,
    inside_logical: bool,
    stack: &mut Vec<(Node<'tree>, u32, bool)>,
) {
    let count = node.child_count();
    if count <= CURSOR_CHILD_THRESHOLD {
        for index in (0..count).rev() {
            stack.push((
                node.child(index).expect("child index is in bounds"),
                nesting,
                inside_logical,
            ));
        }
        return;
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        let start = stack.len();
        stack.push((cursor.node(), nesting, inside_logical));
        while cursor.goto_next_sibling() {
            stack.push((cursor.node(), nesting, inside_logical));
        }
        stack[start..].reverse();
    }
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
    Ok(tokenize_tree(language, source, tree.root_node()))
}

/// Walks an already-parsed tree for normalized leaf tokens.
fn tokenize_tree(language: Language, source: &[u8], root: Node<'_>) -> TokenizedSource {
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
            push_children_reversed(node, &mut stack);
        }
    }
    debug_assert!(
        tokens.windows(2).all(|pair| pair[0].line <= pair[1].line),
        "token walk must visit leaves in line order"
    );
    TokenizedSource {
        language,
        tokens,
        parse_errors,
        line_count: source.iter().filter(|byte| **byte == b'\n').count() as u32 + 1,
    }
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
        || kind == "hex_integer_literal"
        || kind == "octal_integer_literal"
        || kind == "binary_integer_literal"
        || kind == "decimal_floating_point_literal"
        || kind == "hex_floating_point_literal"
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
        Language::Java => java::extract_java_dependencies(root, source),
        Language::JavaScript | Language::TypeScript | Language::Tsx => {
            js::extract_javascript_dependencies(root, source)
        }
    })
}

/// Everything one source file yields for a full project pass.
pub(crate) struct ParsedBundle {
    pub(crate) analysis: FileAnalysis,
    pub(crate) dependencies: ParsedDependencies,
    pub(crate) tokens: TokenizedSource,
}

/// Parses `source` once and derives metrics, dependencies, and normalized
/// tokens from the same tree. Callers that need only one product keep using
/// the narrower entry points.
pub(crate) fn parse_bundle(path: &str, source: &[u8]) -> Result<ParsedBundle> {
    let (language, tree) = parse_tree(path, source)?;
    let root = tree.root_node();
    Ok(ParsedBundle {
        analysis: analyze_tree(path, language, source, root),
        dependencies: match language {
            Language::Java => java::extract_java_dependencies(root, source),
            Language::JavaScript | Language::TypeScript | Language::Tsx => {
                js::extract_javascript_dependencies(root, source)
            }
        },
        tokens: tokenize_tree(language, source, root),
    })
}

/// Parses every entry once, in parallel, preserving input order. Errors are
/// returned for the first path in sorted order.
pub(crate) fn parse_bundles(entries: &[SourceEntry]) -> Result<Vec<(String, ParsedBundle)>> {
    let mut outcomes: Vec<(String, Result<ParsedBundle>)> = entries
        .par_iter()
        .map(|entry| (entry.path.clone(), parse_bundle(&entry.path, &entry.bytes)))
        .collect();
    outcomes.sort_by(|left, right| left.0.cmp(&right.0));
    outcomes
        .into_iter()
        .map(|(path, bundle)| Ok((path, bundle?)))
        .collect()
}

/// Parses one host-language file once for SQL call-site analysis and
/// function attribution.
pub(crate) fn parse_sql_host(
    path: &str,
    source: &[u8],
) -> Result<(FileAnalysis, Vec<HostSqlSite>)> {
    let (language, tree) = parse_tree(path, source)?;
    let root = tree.root_node();
    Ok((
        analyze_tree(path, language, source, root),
        host_sql_sites_from_tree(language, source, root),
    ))
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
        push_children_reversed(node, &mut stack);
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
    let name = function_name(node, source);
    let recursive = !name.starts_with('<') && calls_self(node, language, &name, source);
    analyze_function(
        FunctionInput {
            name,
            id: crate::core::function_id(display_path, kind, start_byte, end_byte),
            kind,
            language,
            recursive,
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
            if language == Language::Java && node.kind() == "lambda_expression" {
                events.push(Event::Decision {
                    kind: DecisionKind::Arrow,
                    nesting,
                    else_if: false,
                    line: node.start_position().row as u32 + 1,
                });
            }
            continue;
        }

        if is_logical_loc(node.kind()) {
            *logical_loc += 1;
        }

        let else_if = is_else_if(node);
        let decision = decision_kind(node, language, source);
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
            collect_logical(node, language, next_sequence, nesting, events);
            next_sequence += 1;
        }

        if node.child_count() == 0 {
            // NOTE(Task 3): no childless `->` leaf observed in this grammar
            // revision (arrow-switch arms surface without one); kept so a
            // future grammar bump that emits one activates Arrow automatically.
            // A lambda's own header `->` is excluded: Arrow counts in the
            // enclosing function (skip branch above), never in itself.
            let own_lambda_header = node.kind() == "->"
                && node.parent().is_some_and(|parent| {
                    parent.id() == root.id() && parent.kind() == "lambda_expression"
                });
            if node.kind() == "->" && language == Language::Java && !own_lambda_header {
                events.push(Event::Decision {
                    kind: DecisionKind::Arrow,
                    nesting,
                    else_if: false,
                    line: node.start_position().row as u32 + 1,
                });
                continue;
            }
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

        push_children_reversed_with(
            node,
            child_nesting,
            inside_logical || this_logical,
            &mut stack,
        );
    }
}

fn decision_kind(node: Node<'_>, language: Language, source: &[u8]) -> Option<DecisionKind> {
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
        "throw_statement" if language != Language::Java => Some(DecisionKind::Throw),
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
            _ => None,
        })
}

fn collect_logical(
    node: Node<'_>,
    language: Language,
    sequence: u32,
    nesting: u32,
    events: &mut Vec<Event>,
) {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        // Nested function bodies have their own walk: an operator inside an
        // arrow/lambda must not join the enclosing function's sequence.
        if current.id() != node.id() && is_function(current.kind(), language) {
            continue;
        }
        if current.child_count() == 0 {
            // Anonymous operator tokens carry their text as the node kind;
            // matching text would also match `||` inside strings or JSX text.
            let operator = match current.kind() {
                "&&" => Some(LogicalOperator::And),
                "||" => Some(LogicalOperator::Or),
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
        push_children_reversed(current, &mut stack);
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

/// True when the function body calls itself by name (direct recursion).
///
/// Nested function bodies are skipped, mirroring `walk_function`: a call
/// inside a nested function is never attributed to the outer one.
/// `super`-qualified calls are excluded (parent-class dispatch, not a cycle).
fn calls_self(root: Node<'_>, language: Language, name: &str, source: &[u8]) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.id() != root.id() && is_function(node.kind(), language) {
            continue;
        }
        let callee = match language {
            Language::Java => {
                if node.kind() == "method_invocation" {
                    let is_super = node
                        .child_by_field_name("object")
                        .is_some_and(|object| node_text(object, source) == "super");
                    if is_super {
                        None
                    } else {
                        node.child_by_field_name("name")
                    }
                } else {
                    None
                }
            }
            Language::JavaScript | Language::TypeScript | Language::Tsx => {
                if node.kind() == "call_expression" {
                    node.child_by_field_name("function").and_then(|function| {
                        match function.kind() {
                            "identifier" => Some(function),
                            "member_expression" => {
                                let is_super = function
                                    .child_by_field_name("object")
                                    .is_some_and(|object| node_text(object, source) == "super");
                                if is_super {
                                    None
                                } else {
                                    function.child_by_field_name("property")
                                }
                            }
                            _ => None,
                        }
                    })
                } else {
                    None
                }
            }
        };
        if callee.is_some_and(|callee| node_text(callee, source) == name) {
            return true;
        }
        push_children_reversed(node, &mut stack);
    }
    false
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
                | "octal_integer_literal"
                | "binary_integer_literal"
                | "decimal_floating_point_literal"
                | "hex_floating_point_literal"
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

pub(super) fn node_text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.byte_range()]).unwrap_or("")
}

/// One recognized query/execute call site in host-language source.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HostSqlSite {
    pub(crate) line: u32,
    pub(crate) end_line: u32,
    pub(crate) dynamic: bool,
    pub(crate) inside_loop: bool,
}

/// Terminal query/execute call names across supported host languages.
const HOST_SQL_CALLS: &[&str] = &["query", "execute", "executeQuery", "executeUpdate", "raw"];

fn is_loop_kind(kind: &str) -> bool {
    matches!(
        kind,
        "for_statement"
            | "for_in_statement"
            | "enhanced_for_statement"
            | "while_statement"
            | "do_statement"
    )
}

/// Recognized SQL call sites: terminal `query`/`execute`-family calls with
/// dynamic-argument and loop-ancestor flags.
///
/// Walks call and method-invocation nodes once. Dynamic means the first
/// argument subtree holds a `+` concatenation or a template substitution;
/// literal text is never inspected or retained. Loop state walks ancestors
/// until a function boundary and stops there, so calls in a nested function
/// are never attributed to an outer loop. Dedplicated by span and flags.
pub(crate) fn host_sql_sites(path: &str, source: &[u8]) -> crate::Result<Vec<HostSqlSite>> {
    let (language, tree) = parse_tree(path, source)?;
    Ok(host_sql_sites_from_tree(language, source, tree.root_node()))
}

/// Walks an already-parsed tree for recognized SQL call sites.
fn host_sql_sites_from_tree(language: Language, source: &[u8], root: Node<'_>) -> Vec<HostSqlSite> {
    let mut seen = std::collections::BTreeSet::new();
    let mut sites = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let Some((name, first_argument)) = sql_call_target(node, language, source)
            && HOST_SQL_CALLS.contains(&name.as_str())
        {
            let dynamic =
                first_argument.is_some_and(|argument| subtree_is_dynamic(argument, language));
            let inside_loop = has_loop_ancestor(node, language);
            if seen.insert((node.start_byte(), node.end_byte(), dynamic, inside_loop)) {
                sites.push(HostSqlSite {
                    line: node.start_position().row as u32 + 1,
                    end_line: node.end_position().row as u32 + 1,
                    dynamic,
                    inside_loop,
                });
            }
        }
        push_children_reversed(node, &mut stack);
    }
    sites.sort_by_key(|site| (site.line, site.end_line));
    sites
}

/// Callee name plus first-argument node for query-shaped calls, else `None`.
///
/// JavaScript/TypeScript match bare and member calls; Java matches method
/// invocations. Similarly named declarations and bare references are not
/// calls and never match.
fn sql_call_target<'a>(
    node: Node<'a>,
    language: Language,
    source: &'a [u8],
) -> Option<(String, Option<Node<'a>>)> {
    let (name_node, arguments) = match language {
        Language::Java => {
            if node.kind() != "method_invocation" {
                return None;
            }
            (
                node.child_by_field_name("name")?,
                node.child_by_field_name("arguments")?,
            )
        }
        Language::JavaScript | Language::TypeScript | Language::Tsx => {
            if node.kind() != "call_expression" {
                return None;
            }
            let function = node.child_by_field_name("function")?;
            let name_node = match function.kind() {
                "identifier" => function,
                "member_expression" => function.child_by_field_name("property")?,
                _ => return None,
            };
            (name_node, node.child_by_field_name("arguments")?)
        }
    };
    let name = node_text(name_node, source).to_owned();
    Some((name, first_named_child(arguments)))
}

fn first_named_child<'a>(node: Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

/// True when the subtree holds a `+` concatenation or template substitution.
///
/// Nested functions are not entered: a callback argument computing `a + b`
/// is not the call's SQL text.
fn subtree_is_dynamic(root: Node<'_>, language: Language) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.id() != root.id() && is_function(node.kind(), language) {
            continue;
        }
        if node.kind() == "template_substitution" {
            return true;
        }
        if node.kind() == "binary_expression" && has_plus_operator(node) {
            return true;
        }
        push_children_reversed(node, &mut stack);
    }
    false
}

fn has_plus_operator(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| !child.is_named() && child.kind() == "+")
}

/// True when a loop ancestor precedes any function boundary.
fn has_loop_ancestor(node: Node<'_>, language: Language) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        let kind = parent.kind();
        if is_loop_kind(kind) {
            return true;
        }
        if is_function(kind, language) {
            return false;
        }
        current = parent.parent();
    }
    false
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_sql_sites_detect_dynamic_query_in_loop() {
        let source = b"import { Pool } from 'pg';
const pool = new Pool();
export async function allUsers(names: string[]) {
  for (const name of names) {
    await pool.query(`SELECT * FROM users WHERE name = ${name}`);
  }
}
";
        let sites = host_sql_sites("src/db.ts", source).unwrap();
        assert_eq!(sites.len(), 1);
        assert!(sites[0].dynamic);
        assert!(sites[0].inside_loop);
        assert_eq!(sites[0].line, 5);
    }

    #[test]
    fn host_sql_sites_ignore_parameterized_calls_and_declarations() {
        let source = b"function query(sql: string) { return sql; }
const q = query;
export async function get(pool: any, id: number) {
  return pool.query('SELECT * FROM users WHERE id = $1', [id]);
}
";
        let sites = host_sql_sites("src/db.ts", source).unwrap();
        assert_eq!(sites.len(), 1);
        assert!(!sites[0].dynamic);
        assert!(!sites[0].inside_loop);
        assert_eq!(sites[0].line, 4);
    }

    #[test]
    fn host_sql_sites_detect_java_concatenation() {
        let source = r#"import java.sql.*;
class Dao {
  void find(Statement st, String name) throws Exception {
    st.executeQuery("SELECT * FROM users WHERE name = '" + name + "'");
  }
}
"#
        .as_bytes();
        let sites = host_sql_sites("src/Dao.java", source).unwrap();
        assert_eq!(sites.len(), 1);
        assert!(sites[0].dynamic);
        assert!(!sites[0].inside_loop);
        assert_eq!(sites[0].line, 4);
    }

    #[test]
    fn logical_operators_stay_out_of_nested_functions_and_literals() {
        let nested = crate::analyze_source(
            "outer.js",
            b"function outer(a, b, c) { return a && (() => b || c)(); }\n",
        )
        .unwrap();
        let outer = nested
            .functions
            .iter()
            .find(|function| function.name == "outer")
            .unwrap();
        assert_eq!(outer.metrics.cyclomatic, 2, "nested arrow operator");
        assert_eq!(outer.metrics.cognitive, 1, "nested arrow operator");

        let jsx =
            crate::analyze_source("f.tsx", b"function f(c) { return c && <span>||</span>; }\n")
                .unwrap();
        let f = jsx
            .functions
            .iter()
            .find(|function| function.name == "f")
            .unwrap();
        assert_eq!(f.metrics.cyclomatic, 2, "JSX text must not count");
        assert_eq!(f.metrics.cognitive, 1, "JSX text must not count");

        let template =
            crate::analyze_source("g.js", b"function g(a) { return a && `||`; }\n").unwrap();
        let g = template
            .functions
            .iter()
            .find(|function| function.name == "g")
            .unwrap();
        assert_eq!(g.metrics.cyclomatic, 2, "template text must not count");
        assert_eq!(g.metrics.cognitive, 1, "template text must not count");
    }

    #[test]
    fn java_numeric_literals_normalize_for_duplication() {
        let tokens = normalized_tokens(
            "A.java",
            b"class A { double d = 3.14; long h = 0xFF; int o = 017; int b = 0b1010; }\n",
        )
        .unwrap();
        let texts: Vec<&str> = tokens
            .tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect();
        assert!(texts.contains(&"<num>"), "{texts:?}");
        assert!(
            !texts
                .iter()
                .any(|text| text.contains("3.14") || text.contains("0xFF")),
            "{texts:?}"
        );
    }

    #[test]
    fn host_sql_dynamic_stops_at_nested_functions() {
        let source = b"await db.query(rows.map((r) => r.a + r.b));
await db.query('SELECT 1 ' + tail);
";
        let sites = host_sql_sites("src/db.js", source).unwrap();
        assert_eq!(sites.len(), 2, "{sites:?}");
        // A callback's arithmetic is not the call's SQL text.
        assert!(!sites[0].dynamic, "{sites:?}");
        assert!(sites[1].dynamic, "{sites:?}");
    }

    #[test]
    fn java_octal_binary_and_hex_float_literals_count_as_operands() {
        let operands = |name: &str, source: &[u8]| {
            crate::analyze_source(name, source).unwrap().functions[0]
                .metrics
                .halstead_n2
        };
        // The same two-literal shape in every syntax must count identically;
        // decimal literals are the established baseline.
        let decimal = operands(
            "Dec.java",
            b"class Dec { void run() { int a = 31; int b = 47; } }\n",
        );
        for (name, source) in [
            (
                "Hex.java",
                &b"class Hex { void run() { int a = 0x1F; int b = 0x2F; } }\n"[..],
            ),
            (
                "Octal.java",
                b"class Octal { void run() { int a = 017; int b = 027; } }\n",
            ),
            (
                "Binary.java",
                b"class Binary { void run() { int a = 0b1010; int b = 0b1101; } }\n",
            ),
            (
                "HexFloat.java",
                b"class HexFloat { void run() { double a = 0x1.8p3; double b = 0x2.8p3; } }\n",
            ),
        ] {
            assert_eq!(operands(name, source), decimal, "{name}");
        }
    }

    #[test]
    fn bundle_matches_narrow_entry_points() {
        let source = b"import { x } from './x';\nexport function f(a: number) { if (a > 1) { return a + 1; } return a; }\n";
        let bundle = parse_bundle("src/f.ts", source).unwrap();
        let analysis = crate::analyze_source("src/f.ts", source).unwrap();
        assert_eq!(bundle.analysis.functions, analysis.functions);
        assert_eq!(
            bundle.tokens,
            normalized_tokens("src/f.ts", source).unwrap()
        );
        assert_eq!(
            bundle.dependencies,
            extract_dependencies("src/f.ts", source).unwrap()
        );
    }

    #[test]
    fn parse_bundles_sorts_paths_and_selects_errors_deterministically() {
        let entries = vec![
            SourceEntry {
                path: "b.ts".to_owned(),
                bytes: b"function b() {}".to_vec(),
            },
            SourceEntry {
                path: "a.ts".to_owned(),
                bytes: b"function a() {}".to_vec(),
            },
        ];
        let bundles = parse_bundles(&entries).unwrap();
        assert_eq!(bundles[0].0, "a.ts");
        assert_eq!(bundles[1].0, "b.ts");

        let invalid = vec![
            SourceEntry {
                path: "b.unknown".to_owned(),
                bytes: b"x".to_vec(),
            },
            SourceEntry {
                path: "a.unknown".to_owned(),
                bytes: b"x".to_vec(),
            },
        ];
        assert!(parse_bundles(&invalid).is_err());
    }
}
