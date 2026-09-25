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

pub(crate) mod c_family;
mod go;
mod java;
mod js;
pub(crate) mod python;
pub(crate) mod rust;

thread_local! {
    static THREAD_PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RawDependencyKind {
    GoImport,
    JavaScriptImport,
    JavaScriptCall,
    JavaScriptUndecodable,
    Java,
    JavaStatic,
    JavaWildcard,
    LocalInclude,
    PythonImport,
    PythonAbsoluteImport,
    RustModule,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawDependency {
    pub(crate) kind: RawDependencyKind,
    pub(crate) specifier: String,
    pub(crate) line: u32,
}

/// One name a file exports to its consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExportedSymbol {
    /// Exported name. A star re-export exports no name of its own, so it
    /// carries the module specifier instead.
    pub(crate) name: String,
    pub(crate) line: u32,
    pub(crate) kind: ExportKind,
}

/// How an exported name reaches consumers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExportKind {
    Named,
    Default,
    /// `export * from "..."`: every name of the target module is re-exported.
    Star,
}

/// One name a file imports or re-exports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportedSymbol {
    /// Name as the source module declares it: `import { a as b }` records
    /// `a`, so matching it against exports needs no alias table. A star form
    /// (`import * as ns`, `export * from "..."`) reads no single name and
    /// records none.
    pub(crate) name: String,
    pub(crate) line: u32,
    /// True for a namespace import or a star re-export: the names it reads
    /// are not statically known.
    pub(crate) star: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ParsedDependencies {
    pub(crate) references: Vec<RawDependency>,
    pub(crate) exports: Vec<ExportedSymbol>,
    pub(crate) imports: Vec<ImportedSymbol>,
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
        if language == Language::Zig
            && matches!(node.kind(), "builtin_type" | "character" | "multiline_string")
        {
            if let Some(text) = normalize_leaf(node, source) {
                tokens.push(NormalizedToken {
                    text,
                    line: node.start_position().row as u32 + 1,
                });
            }
            continue;
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
            | "package_identifier"
            | "blank_identifier"
            | "property_identifier"
            | "type_identifier"
            | "field_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern"
            | "statement_identifier"
            | "builtin_type"
    ) {
        return Some("<id>".to_owned());
    }
    if kind.contains("string")
        || matches!(
            kind,
            "template_chars" | "string_fragment" | "character" | "character_content"
        )
    {
        return Some("<str>".to_owned());
    }
    if kind.contains("number")
        || kind == "integer"
        || kind == "float"
        || kind == "int_literal"
        || kind == "integer_literal"
        || kind == "float_literal"
        || kind == "imaginary_literal"
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
        Language::C | Language::Cpp => c_family::extract_local_includes(root, source),
        Language::Go => go::extract_go_dependencies(root, source),
        Language::Java => java::extract_java_dependencies(root, source),
        Language::Rust => rust::extract_rust_dependencies(root, source),
        Language::Python => python::extract_relative_imports(root, source),
        Language::JavaScript | Language::TypeScript | Language::Tsx => {
            js::extract_javascript_dependencies(root, source)
        }
        Language::Zig => ParsedDependencies::default(),
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
            Language::C | Language::Cpp => c_family::extract_local_includes(root, source),
            Language::Go => go::extract_go_dependencies(root, source),
            Language::Java => java::extract_java_dependencies(root, source),
            Language::Rust => rust::extract_rust_dependencies(root, source),
            Language::Python => python::extract_relative_imports(root, source),
            Language::JavaScript | Language::TypeScript | Language::Tsx => {
                js::extract_javascript_dependencies(root, source)
            }
            Language::Zig => ParsedDependencies::default(),
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

/// True when a Python module carries an `if __name__ == "__main__":` guard.
///
/// Parses `source` with the Python grammar and delegates to the pure tree walk;
/// `path` must be a Python path, exactly as `SourceEntry` paths are.
pub(crate) fn python_has_main_guard(path: &str, source: &[u8]) -> Result<bool> {
    let (_, tree) = parse_tree(path, source)?;
    Ok(python::has_main_guard(tree.root_node(), source))
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
        "c" => Some(Language::C),
        "java" => Some(Language::Java),
        "go" => Some(Language::Go),
        "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some(Language::Cpp),
        "rs" => Some(Language::Rust),
        "zig" => Some(Language::Zig),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
        "py" => Some(Language::Python),
        "ts" | "mts" | "cts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        _ => None,
    }
}

fn grammar(language: Language) -> TsLanguage {
    match language {
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::C => tree_sitter_c::LANGUAGE.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Zig => tree_sitter_zig::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

fn has_named_child(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| child.kind() == kind)
}

fn discover_tree<'tree>(
    root: Node<'tree>,
    language: Language,
    functions: &mut Vec<Node<'tree>>,
    parse_errors: &mut Vec<ParseDiagnostic>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if is_function(node, language) {
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
/// True when `node` is a function definition for `language`.
///
/// Python's `lambda` keyword token carries the same kind string as the
/// `lambda` rule, so only named nodes are functions; every other supported
/// grammar spells its function rules as named nodes too, so the gate is
/// uniform.
fn is_function(node: Node<'_>, language: Language) -> bool {
    if !node.is_named() {
        return false;
    }
    let kind = node.kind();
    match language {
        Language::Go => matches!(
            kind,
            "function_declaration" | "method_declaration" | "func_literal"
        ),
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
        Language::C | Language::Cpp => matches!(kind, "function_definition" | "lambda_expression"),
        Language::Rust => matches!(kind, "function_item" | "closure_expression"),
        Language::Zig => match kind {
            "function_declaration" => node.child_by_field_name("body").is_some(),
            "test_declaration" => has_named_child(node, "block"),
            _ => false,
        },
        Language::Python => matches!(kind, "function_definition" | "lambda"),
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
    let kind = function_kind(node, language);
    let start_byte = node.start_byte() as u64;
    let end_byte = node.end_byte() as u64;
    let name = function_name(node, language, source);
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
            parameters: parameter_count(node, language, source),
            logical_loc,
            events,
            source_fingerprint: fingerprint(&source[node.byte_range()]),
        },
        source,
    )
}

fn function_kind(node: Node<'_>, language: Language) -> FunctionKind {
    if language == Language::Go && node.kind() == "method_declaration" {
        return FunctionKind::Method;
    }
    if language == Language::Rust && node.kind() == "function_item" && rust_is_method(node) {
        return FunctionKind::Method;
    }
    if language == Language::Cpp && node.kind() == "function_definition" && cpp_is_method(node) {
        return FunctionKind::Method;
    }
    if language == Language::Zig && node.kind() == "test_declaration" {
        return FunctionKind::Function;
    }
    // Python has no method node: a `def` inside a class body is still a
    // `function_definition`, so the method verdict walks the enclosing chain.
    if language == Language::Python {
        if node.kind() == "lambda" {
            return FunctionKind::Lambda;
        }
        if node.kind() == "function_definition" && python_is_method(node) {
            return FunctionKind::Method;
        }
    }
    match node.kind() {
        "method_definition" => FunctionKind::Method,
        "closure_expression" => FunctionKind::Lambda,
        "constructor_declaration" | "compact_constructor_declaration" => FunctionKind::Constructor,
        "lambda_expression" => FunctionKind::Lambda,
        "arrow_function" => FunctionKind::Arrow,
        "function_expression" | "func_literal" if node.child_by_field_name("name").is_none() => {
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
        if node.id() != root.id() && is_function(node, language) {
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

        if is_logical_loc(node, language) {
            *logical_loc += 1;
        }

        let else_if = is_else_if(node, language);
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
            // Python repeats the `alternative` field once per `elif_clause`
            // before the trailing `else_clause`, so the first alternative of an
            // if/elif chain is an `elif_clause`: it is scored as an else-if, not
            // as an else, and the final `else_clause` still fires below.
            if let Some(alternative) = node.child_by_field_name("alternative")
                && !matches!(
                    alternative.kind(),
                    "if_statement" | "else_clause" | "elif_clause"
                )
            {
                events.push(Event::Else {
                    line: node.start_position().row as u32 + 1,
                    nesting,
                });
            }
        } else if (language == Language::Zig
            && node.kind() == "if_expression"
            && zig_if_expression_has_non_if_else(node))
            || (node.kind() == "else_clause" && !has_if_child(node))
        {
            events.push(Event::Else {
                line: node.start_position().row as u32 + 1,
                nesting,
            });
        }
        if is_labeled_jump(node, language) {
            events.push(Event::LabeledJump {
                line: node.start_position().row as u32 + 1,
                nesting,
            });
        }

        let this_logical = logical_operator(node, language, source).is_some();
        if this_logical && !inside_logical {
            collect_logical(node, language, next_sequence, nesting, events);
            next_sequence += 1;
        }

        if language == Language::Zig
            && matches!(
                node.kind(),
                "boolean" | "builtin_type" | "string" | "character" | "multiline_string"
            )
        {
            events.push(Event::Operand(Span {
                start: node.start_byte(),
                end: node.end_byte(),
            }));
            continue;
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
            if is_operator_leaf(node.kind(), language) {
                events.push(Event::Operator(span));
            } else if is_operand_leaf(node, language) {
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
        "if_statement" | "if_expression" => Some(DecisionKind::If),
        "for_statement"
        | "for_in_statement"
        | "enhanced_for_statement"
        | "while_statement"
        | "while_expression"
        | "loop_expression"
        | "for_expression"
        | "do_statement"
        | "for_range_loop" => Some(DecisionKind::Loop),
        "catch_clause" | "catch_expression" => Some(DecisionKind::Catch),
        "switch_statement"
        | "switch_expression"
        | "expression_switch_statement"
        | "type_switch_statement"
        | "select_statement"
        | "match_expression" => Some(DecisionKind::Switch),
        "switch_case" | "expression_case" | "type_case" | "communication_case" | "default_case"
        | "match_arm" | "case_statement" => Some(DecisionKind::Case),
        "switch_label" if node_text(node, source).trim_start().starts_with("case") => {
            Some(DecisionKind::Case)
        }
        "binary_expression"
            if language == Language::Zig
                && node
                    .child_by_field_name("operator")
                    .is_some_and(|operator| node_text(operator, source) == "orelse") =>
        {
            Some(DecisionKind::Ternary)
        }
        "ternary_expression" | "conditional_expression" => Some(DecisionKind::Ternary),
        "try_expression" => Some(DecisionKind::Try),
        "throw_statement" if !matches!(language, Language::Java | Language::Cpp) => {
            Some(DecisionKind::Throw)
        }
        _ if language == Language::Python => python_decision_kind(node.kind()),
        _ => None,
    }
}

/// Python-only decision kinds, keyed on tree-sitter-python node kinds.
///
/// `elif_clause` is not an `if_statement`: it maps to `If` and is scored as an
/// else-if, so a chain adds one point per branch and never raises nesting.
/// `for_in_clause`/`if_clause` are comprehension clauses, scored so a
/// comprehension reads like the equivalent explicit loop.
fn python_decision_kind(kind: &str) -> Option<DecisionKind> {
    match kind {
        "elif_clause" | "if_clause" => Some(DecisionKind::If),
        "for_in_clause" => Some(DecisionKind::Loop),
        "except_clause" => Some(DecisionKind::Catch),
        "match_statement" => Some(DecisionKind::Switch),
        "case_clause" => Some(DecisionKind::Case),
        "raise_statement" => Some(DecisionKind::Throw),
        _ => None,
    }
}

/// True when a `break`/`continue` carries a label.
fn is_labeled_jump(node: Node<'_>, language: Language) -> bool {
    match node.kind() {
        // Go and the C-family grammars expose the label as the only named
        // child. Rust exposes a `label` node, so `break value` stays a value
        // jump rather than a labeled one.
        "break_statement" | "continue_statement" => node.named_child_count() > 0,
        "break_expression" | "continue_expression" => {
            has_named_child(node, "label") || has_named_child(node, "break_label")
        }
        "labeled_statement"
            if language == Language::Zig && has_named_child(node, "block_label") =>
        {
            true
        }
        "goto_statement" | "labeled_statement"
            if matches!(language, Language::C | Language::Cpp) =>
        {
            true
        }
        _ => false,
    }
}

fn zig_if_expression_has_non_if_else(node: Node<'_>) -> bool {
    let mut saw_else = false;
    let mut alternative = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "else" {
            saw_else = true;
        } else if saw_else && child.is_named() {
            alternative = Some(child);
        }
    }
    alternative.is_some_and(|child| child.kind() != "if_expression")
}

fn is_else_if(node: Node<'_>, language: Language) -> bool {
    if language == Language::Zig && node.kind() == "if_expression" {
        return node
            .prev_sibling()
            .is_some_and(|previous| previous.kind() == "else");
    }
    if node.kind() == "elif_clause" {
        return true;
    }
    if !matches!(node.kind(), "if_statement" | "if_expression") {
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
        .any(|child| matches!(child.kind(), "if_statement" | "if_expression"))
}

fn logical_operator(node: Node<'_>, language: Language, source: &[u8]) -> Option<LogicalOperator> {
    // `let_chain` is Rust's `if let A = x && let B = y`: its `&&` separators
    // are direct tokens of the chain, never `binary_expression` operators.
    // Python spells its logical operators as the keyword tokens inside a
    // dedicated `boolean_operator` node.
    let python_boolean = language == Language::Python && node.kind() == "boolean_operator";
    if !matches!(
        node.kind(),
        "binary_expression" | "binary_expression2" | "let_chain"
    ) && !python_boolean
    {
        return None;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| match node_text(child, source) {
            "&&" => Some(LogicalOperator::And),
            "||" => Some(LogicalOperator::Or),
            "and" if matches!(language, Language::Python | Language::Zig) => {
                Some(LogicalOperator::And)
            }
            "or" if matches!(language, Language::Python | Language::Zig) => {
                Some(LogicalOperator::Or)
            }
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
        if current.id() != node.id() && is_function(current, language) {
            continue;
        }
        if current.child_count() == 0 {
            // Anonymous operator tokens carry their text as the node kind;
            // matching text would also match `||` inside strings or JSX text.
            let operator = match current.kind() {
                "&&" => Some(LogicalOperator::And),
                "||" => Some(LogicalOperator::Or),
                "and" if matches!(language, Language::Python | Language::Zig) => {
                    Some(LogicalOperator::And)
                }
                "or" if matches!(language, Language::Python | Language::Zig) => {
                    Some(LogicalOperator::Or)
                }
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

fn cpp_is_method(node: Node<'_>) -> bool {
    if c_family_declarator_is_qualified(node) {
        return true;
    }
    let mut saw_field_list = false;
    let mut ancestor = node.parent();
    while let Some(current) = ancestor {
        if current.kind() == "field_declaration_list" {
            saw_field_list = true;
        } else if matches!(current.kind(), "class_specifier" | "struct_specifier") {
            return saw_field_list;
        }
        ancestor = current.parent();
    }
    false
}

fn c_family_declarator_is_qualified(node: Node<'_>) -> bool {
    let Some(mut declarator) = node.child_by_field_name("declarator") else {
        return false;
    };
    loop {
        if declarator.kind() == "qualified_identifier" {
            return true;
        }
        let Some(inner) = declarator.child_by_field_name("declarator") else {
            return false;
        };
        declarator = inner;
    }
}
fn c_family_function_declarator_name<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    let mut declarator = node.child_by_field_name("declarator")?;
    loop {
        match declarator.kind() {
            "function_declarator"
            | "pointer_declarator"
            | "reference_declarator"
            | "parenthesized_declarator" => {
                declarator = declarator.child_by_field_name("declarator")?;
            }
            "qualified_identifier" => return declarator.child_by_field_name("name"),
            "identifier" | "field_identifier" | "destructor_name" | "operator_name" => {
                return Some(declarator);
            }
            _ => return None,
        }
    }
}

fn c_family_function_parameters<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    let mut declarator = node.child_by_field_name("declarator")?;
    loop {
        if matches!(
            declarator.kind(),
            "function_declarator" | "abstract_function_declarator"
        ) {
            return declarator.child_by_field_name("parameters");
        }
        declarator = declarator.child_by_field_name("declarator")?;
    }
}

fn function_name(node: Node<'_>, language: Language, source: &[u8]) -> String {
    // Both the C family and Python spell a `def`-like declaration
    // `function_definition`; only the C family hides the name under a
    // `declarator`, so the branch stays language-gated and Python falls
    // through to the `name` field below.
    if node.kind() == "function_definition" && matches!(language, Language::C | Language::Cpp) {
        return c_family_function_declarator_name(node)
            .map(|name| node_text(name, source).to_owned())
            .unwrap_or_else(|| {
                let point = node.start_position();
                format!("<anonymous@{}:{}>", point.row + 1, point.column + 1)
            });
    }
    if node.kind() == "lambda_expression"
        && let Some(parent) = node.parent()
    {
        let binding = match parent.kind() {
            "init_declarator" => parent.child_by_field_name("declarator"),
            "assignment_expression" => parent.child_by_field_name("left"),
            _ => None,
        };
        if let Some(binding) = binding {
            return node_text(binding, source).to_owned();
        }
    }
    if node.kind() == "func_literal" {
        // The literal sits in the right-hand `expression_list`; the
        // declaration is the grandparent.
        let mut ancestor = node.parent();
        if ancestor.is_some_and(|parent| parent.kind() == "expression_list") {
            ancestor = ancestor.and_then(|parent| parent.parent());
        }
        if let Some(parent) = ancestor {
            if parent.kind() == "short_var_declaration"
                && let Some(left) = parent.child_by_field_name("left")
            {
                let mut cursor = left.walk();
                if let Some(id) = left.named_children(&mut cursor).next() {
                    return node_text(id, source).to_owned();
                }
            }
            if parent.kind() == "var_spec"
                && let Some(name) = parent.child_by_field_name("name")
            {
                return node_text(name, source).to_owned();
            }
        }
    }
    // Rust closures take their name from the `let` binding when one exists.
    if node.kind() == "closure_expression"
        && let Some(pattern) = node
            .parent()
            .filter(|parent| parent.kind() == "let_declaration")
            .and_then(|parent| parent.child_by_field_name("pattern"))
        && pattern.kind() == "identifier"
    {
        return node_text(pattern, source).to_owned();
    }
    // Python binds a lambda to an assignment target: `f = lambda x: ...` names
    // the lambda `f`, the way a Rust closure takes its `let` binding.
    if node.kind() == "lambda"
        && language == Language::Python
        && let Some(binding) = node
            .parent()
            .filter(|parent| parent.kind() == "assignment")
            .and_then(|parent| parent.child_by_field_name("left"))
        && binding.kind() == "identifier"
    {
        return node_text(binding, source).to_owned();
    }
    if language == Language::Zig && node.kind() == "test_declaration" {
        let mut cursor = node.walk();
        if let Some(name) = node
            .named_children(&mut cursor)
            .find(|child| matches!(child.kind(), "string" | "identifier"))
        {
            let name = node_text(name, source);
            return if name.starts_with('"') && name.ends_with('"') {
                name.trim_matches('"').to_owned()
            } else {
                name.to_owned()
            };
        }
    }
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
        if node.id() != root.id() && is_function(node, language) {
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
            Language::Go => {
                if node.kind() == "call_expression" {
                    node.child_by_field_name("function").and_then(|function| {
                        match function.kind() {
                            "identifier" => Some(function),
                            "selector_expression" => function.child_by_field_name("field"),
                            _ => None,
                        }
                    })
                } else {
                    None
                }
            }
            // C and C++ accept a bare call and a scoped `Type::method` path;
            // only a `this->method()` receiver is the current type's own
            // dispatch, so a call on another object is not a cycle.
            Language::C | Language::Cpp => {
                if node.kind() == "call_expression" {
                    node.child_by_field_name("function").and_then(|function| {
                        match function.kind() {
                            "identifier" => Some(function),
                            "qualified_identifier" => function.child_by_field_name("name"),
                            "field_expression" => {
                                let is_self = function
                                    .child_by_field_name("argument")
                                    .is_some_and(|argument| node_text(argument, source) == "this");
                                if is_self {
                                    function.child_by_field_name("field")
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    })
                } else {
                    None
                }
            }
            Language::Zig => {
                if node.kind() == "call_expression" {
                    node.child_by_field_name("function")
                        .filter(|function| function.kind() == "identifier")
                } else {
                    None
                }
            }
            // Rust accepts a bare call, a scoped `Type::method` path, and a
            // receiver call, but only `self.method()` is the current type's
            // own dispatch; `other.method()` is not a cycle. A turbofish call
            // (`count::<T>()`) reports the callee as a `generic_function`.
            Language::Rust => {
                if node.kind() == "call_expression" {
                    node.child_by_field_name("function").and_then(|function| {
                        let function = if function.kind() == "generic_function" {
                            function.child_by_field_name("function")?
                        } else {
                            function
                        };
                        match function.kind() {
                            "identifier" => Some(function),
                            "scoped_identifier" => function.child_by_field_name("name"),
                            "field_expression" => {
                                let is_self = function
                                    .child_by_field_name("value")
                                    .is_some_and(|value| value.kind() == "self");
                                if is_self {
                                    function.child_by_field_name("field")
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    })
                } else {
                    None
                }
            }
            // Python accepts a bare call and a `self.method()` receiver call;
            // an absolute-import-style module call (`os.path.join`) is another
            // module's function, never a cycle.
            Language::Python => {
                if node.kind() == "call" {
                    node.child_by_field_name("function").and_then(|function| {
                        match function.kind() {
                            "identifier" => Some(function),
                            "attribute" => {
                                let is_self =
                                    function
                                        .child_by_field_name("object")
                                        .is_some_and(|object| {
                                            object.kind() == "identifier"
                                                && node_text(object, source) == "self"
                                        });
                                if is_self {
                                    function.child_by_field_name("attribute")
                                } else {
                                    None
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

fn parameter_count(node: Node<'_>, language: Language, source: &[u8]) -> u32 {
    // Python's `parameters` node is also the kind name Rust uses, but Python
    // adds named `keyword_separator` (`*`) and `positional_separator` (`/`)
    // children and keeps comments inside the list, so the count is an
    // allow-list of the six parameter carriers. `self` is a plain
    // `identifier`, so the receiver counts like any other parameter.
    if language == Language::Python {
        return node
            .child_by_field_name("parameters")
            .map_or(0, |parameters| {
                let mut cursor = parameters.walk();
                parameters
                    .named_children(&mut cursor)
                    .filter(|child| {
                        matches!(
                            child.kind(),
                            "identifier"
                                | "typed_parameter"
                                | "default_parameter"
                                | "typed_default_parameter"
                                | "list_splat_pattern"
                                | "dictionary_splat_pattern"
                        )
                    })
                    .count() as u32
            });
    }
    if matches!(language, Language::C | Language::Cpp)
        && let Some(parameters) = c_family_function_parameters(node)
    {
        let mut count = 0;
        let mut lone_void = false;
        let mut cursor = parameters.walk();
        for declaration in parameters
            .named_children(&mut cursor)
            .filter(|declaration| {
                matches!(
                    declaration.kind(),
                    "parameter_declaration" | "optional_parameter_declaration"
                )
            })
        {
            count += 1;
            lone_void = count == 1
                && declaration.child_by_field_name("declarator").is_none()
                && declaration
                    .child_by_field_name("type")
                    .is_some_and(|kind| node_text(kind, source) == "void");
        }
        return if lone_void { 0 } else { count };
    }
    if language == Language::Zig
        && let Some(parameters) = node
            .named_children(&mut node.walk())
            .find(|child| child.kind() == "parameters")
    {
        let mut cursor = parameters.walk();
        return parameters
            .named_children(&mut cursor)
            .filter(|child| child.kind() == "parameter")
            .count() as u32;
    }
    if let Some(parameters) = node.child_by_field_name("parameters") {
        // Go groups names (`func f(a, b, c bool)` is one
        // parameter_declaration with three `name` fields); only
        // tree-sitter-go emits `parameter_list`, so this arm is Go-only.
        // The receiver (`receiver` field) is never counted.
        if language == Language::Go && parameters.kind() == "parameter_list" {
            let mut cursor = parameters.walk();
            return parameters
                .named_children(&mut cursor)
                .map(|declaration| {
                    let mut inner = declaration.walk();
                    (declaration
                        .children_by_field_name("name", &mut inner)
                        .count() as u32)
                        .max(1)
                })
                .sum();
        }
        // Rust receivers (`self`, `&self`, `&mut self`) are dispatch syntax,
        // not inputs; Go receivers are excluded the same way. Only Rust
        // emits a `parameters` node, so this arm cannot reach another
        // language's parameter list.
        if parameters.kind() == "parameters" {
            let mut cursor = parameters.walk();
            return parameters
                .named_children(&mut cursor)
                .filter(|child| child.kind() != "self_parameter")
                .count() as u32;
        }
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

/// True when a Rust `function_item` sits in an `impl` or `trait` body:
/// `fn add(&self)` inside `impl Point` is a method, a free `fn add()` is not.
fn rust_is_method(node: Node<'_>) -> bool {
    node.parent()
        .filter(|parent| parent.kind() == "declaration_list")
        .and_then(|parent| parent.parent())
        .is_some_and(|owner| matches!(owner.kind(), "impl_item" | "trait_item"))
}

/// True when a Python `function_definition` sits inside a `class_definition`
/// body: `def run(self)` in `class Worker` is a method, a module-level `def` is
/// not. The walk crosses nested blocks and decorators; a `def` nested inside a
/// function that is itself inside a class still reports a method, because the
/// enclosing chain reaches the class.
fn python_is_method(node: Node<'_>) -> bool {
    let mut ancestor = node.parent();
    while let Some(current) = ancestor {
        if current.kind() == "class_definition" {
            return true;
        }
        ancestor = current.parent();
    }
    false
}

fn is_logical_loc(node: Node<'_>, language: Language) -> bool {
    let kind = node.kind();
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
            | "short_var_declaration"
            | "var_spec"
            | "const_spec"
            | "assignment_statement"
            | "inc_statement"
            | "dec_statement"
            | "send_statement"
            | "go_statement"
            | "defer_statement"
            | "expression_case"
            | "type_case"
            | "communication_case"
            | "default_case"
            | "select_statement"
            | "expression_switch_statement"
            | "type_switch_statement"
            | "let_declaration"
            | "return_expression"
            | "if_expression"
            | "while_expression"
            | "loop_expression"
            | "for_expression"
            | "match_expression"
            | "match_arm"
            | "break_expression"
            | "continue_expression"
    ) || (language == Language::Zig
        && (matches!(
            kind,
            "errdefer_statement"
                | "suspend_statement"
                | "nosuspend_statement"
                | "comptime_statement"
        ) || (kind == "labeled_statement" && has_named_child(node, "block_label"))))
        || (matches!(language, Language::C | Language::Cpp)
            && matches!(
                kind,
                "declaration"
                    | "goto_statement"
                    | "labeled_statement"
                    | "case_statement"
                    | "for_range_loop"
                    | "preproc_if"
                    | "preproc_ifdef"
                    | "preproc_elif"
                    | "preproc_else"
            ))
        || (language == Language::Python
            && matches!(
                kind,
                "raise_statement"
                    | "with_statement"
                    | "assert_statement"
                    | "match_statement"
                    | "case_clause"
                    | "global_statement"
                    | "nonlocal_statement"
                    | "pass_statement"
                    | "delete_statement"
                    | "for_in_clause"
                    | "if_clause"
            ))
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
            | ":="
            | "<-"
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
            | "loop"
            | "match"
            | "instanceof"
            | "in"
            | "typeof"
            | "void"
            | "delete"
            | "await"
            | "yield"
            | "and"
            | "or"
            | "orelse"
            | "try"
            | "defer"
            | "errdefer"
            | "suspend"
            | "resume"
            | "nosuspend"
            | "comptime"
            | "inline"
            | "noinline"
            | "test"
            | "fn"
            | "pub"
            | "usingnamespace"
            | "unreachable"
            | "undefined"
            | "align"
            | "addrspace"
            | "linksection"
            | "callconv"
            | "noalias"
            | "allowzero"
            | "threadlocal"
            | "packed"
            | "opaque"
    )
}

/// True when a childless node is an operator.
///
/// Shared operator kinds are keyed on punctuation. Python spells many
/// operators as keywords, and Java's grammar emits anonymous tokens whose text
/// is `import`, while other grammars emit `class`, `case`, `from`, `as`, or
/// `del`; adding those strings to `is_operator` would move another language's
/// Halstead numbers, so the keyword spellings stay behind the Python gate.
/// `match` is absent: the shared table already counts it as an operator for
/// every language, so a Python entry would be inert.
fn is_operator_leaf(kind: &str, language: Language) -> bool {
    if language == Language::Zig && matches!(kind, "undefined" | "unreachable") {
        return false;
    }
    is_operator(kind)
        || (language == Language::Python
            && matches!(
                kind,
                "and"
                    | "or"
                    | "not"
                    | "is"
                    | "lambda"
                    | "with"
                    | "as"
                    | "assert"
                    | "raise"
                    | "try"
                    | "except"
                    | "finally"
                    | "elif"
                    | "def"
                    | "import"
                    | "from"
                    | "del"
                    | "pass"
                    | "global"
                    | "nonlocal"
                    | "async"
            ))
}

/// True when a childless node is an operand.
///
/// Rust reports `true`/`false` as anonymous tokens inside expressions and as
/// named `boolean_literal` patterns, so both spellings count; every other
/// language spells its literals as named nodes, which the `is_named` gate
/// already covers.
fn is_operand_leaf(node: Node<'_>, language: Language) -> bool {
    if matches!(language, Language::C | Language::Cpp)
        && matches!(node.kind(), "number_literal" | "string_content")
    {
        return node.is_named();
    }
    // Python wraps a literal in a `string` node whose leaf is
    // `string_content`, so the shared `string` entry never reaches this
    // check: the leaf is the operand.
    if language == Language::Python && node.kind() == "string_content" {
        return node.is_named();
    }
    if !is_operand(node.kind()) {
        return false;
    }
    node.is_named()
        || (language == Language::Rust && matches!(node.kind(), "true" | "false"))
        || (language == Language::Zig
            && matches!(
                node.kind(),
                "undefined"
                    | "unreachable"
                    | "anyframe"
                    | "noreturn"
                    | "comptime_int"
                    | "comptime_float"
            ))
}

fn is_operand(kind: &str) -> bool {
    kind.ends_with("identifier")
        || matches!(
            kind,
            "identifier"
                | "number"
                | "int_literal"
                | "float_literal"
                | "imaginary_literal"
                | "rune_literal"
                | "nil"
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
                | "integer_literal"
                | "boolean_literal"
                | "char_literal"
                | "self"
                | "integer"
                | "float"
                | "none"
                | "boolean"
                | "builtin_type"
                | "builtin_identifier"
                | "character"
                | "multiline_string"
                | "undefined"
                | "unreachable"
                | "anyframe"
                | "noreturn"
                | "comptime_int"
                | "comptime_float"
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
const HOST_SQL_CALLS: &[&str] = &[
    "query",
    "execute",
    "executeQuery",
    "executeUpdate",
    "raw",
    "Query",
    "QueryRow",
    "QueryContext",
    "QueryRowContext",
    "Exec",
    "ExecContext",
    "query_as",
    "query_scalar",
    "executemany",
];

fn is_loop_kind(kind: &str) -> bool {
    matches!(
        kind,
        "for_statement"
            | "for_in_statement"
            | "enhanced_for_statement"
            | "while_statement"
            | "while_expression"
            | "loop_expression"
            | "for_expression"
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
            let dynamic = first_argument.is_some_and(|argument| subtree_is_dynamic(argument, language))
                    // `db.Query(fmt.Sprintf(...))` nests the Sprintf call in the
                    // argument, so the top-level callee name never carries the
                    // signal; the nested call builds the query text dynamically
                    // by construction.
                    || (language == Language::Go
                        && first_argument.is_some_and(|argument| {
                            subtree_contains_sprintf(argument, source)
                        }))
                    // Rust builds query text with `format!`/`concat!` rather
                    // than `+` concatenation, so the same argument never
                    // carries a `binary_expression` signal.
                    || (language == Language::Rust
                        && first_argument.is_some_and(|argument| {
                            subtree_contains_format_macro(argument, source)
                        }))
                    // Python builds query text with f-strings, `%` formatting,
                    // `.format(...)`, or `+` concatenation, none of which the
                    // shared `template_substitution`/`binary_expression` probes
                    // can see.
                    || (language == Language::Python
                        && first_argument.is_some_and(|argument| {
                            subtree_is_dynamic_python(argument, source)
                        }));
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
        Language::C | Language::Zig => return None,
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
        Language::Go => {
            if node.kind() != "call_expression" {
                return None;
            }
            let function = node.child_by_field_name("function")?;
            let name_node = match function.kind() {
                "identifier" => function,
                "selector_expression" => function.child_by_field_name("field")?,
                _ => return None,
            };
            (name_node, node.child_by_field_name("arguments")?)
        }
        Language::Cpp => return None,
        Language::Python => {
            if node.kind() != "call" {
                return None;
            }
            let function = node.child_by_field_name("function")?;
            let name_node = match function.kind() {
                "identifier" => function,
                "attribute" => function.child_by_field_name("attribute")?,
                _ => return None,
            };
            (name_node, node.child_by_field_name("arguments")?)
        }
        Language::Rust => {
            if node.kind() != "call_expression" {
                return None;
            }
            let function = node.child_by_field_name("function")?;
            // `sqlx::query_as::<_, User>(..)` reports the callee as a
            // `generic_function`; the name sits under its own `function` field.
            let function = if function.kind() == "generic_function" {
                function.child_by_field_name("function")?
            } else {
                function
            };
            let name_node = match function.kind() {
                "identifier" => function,
                "scoped_identifier" => function.child_by_field_name("name")?,
                "field_expression" => function.child_by_field_name("field")?,
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
        if node.id() != root.id() && is_function(node, language) {
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

/// True when a Python argument subtree builds query text dynamically.
///
/// Python has no template substitution and its concatenation is a
/// `binary_operator`, not a `binary_expression`: an f-string carries
/// `interpolation` children, and `%` formatting, `.format(...)`, and `+`
/// concatenation all hide the text from a literal reader. Nested function
/// bodies are not entered, mirroring `subtree_is_dynamic`.
fn subtree_is_dynamic_python(root: Node<'_>, source: &[u8]) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.id() != root.id() && is_function(node, Language::Python) {
            continue;
        }
        match node.kind() {
            "interpolation" => return true,
            "binary_operator"
                if matches!(
                    node.child_by_field_name("operator")
                        .map(|operator| node_text(operator, source)),
                    Some("+" | "%")
                ) =>
            {
                return true;
            }
            "call" if python_call_is_format(node, source) => return true,
            _ => {}
        }
        push_children_reversed(node, &mut stack);
    }
    false
}

/// True when a Python call is a `str.format(...)` method call.
fn python_call_is_format(node: Node<'_>, source: &[u8]) -> bool {
    node.child_by_field_name("function")
        .filter(|function| function.kind() == "attribute")
        .and_then(|function| function.child_by_field_name("attribute"))
        .is_some_and(|name| node_text(name, source) == "format")
}

/// True when the subtree holds a `Sprintf`-family call (Go only):
/// `fmt.Sprintf("...%v...", x)` builds the query text dynamically by
/// construction. Nested function bodies are not entered, mirroring
/// `subtree_is_dynamic`.
fn subtree_contains_sprintf(root: Node<'_>, source: &[u8]) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.id() != root.id() && is_function(node, Language::Go) {
            continue;
        }
        if node.kind() == "call_expression"
            && let Some(function) = node.child_by_field_name("function")
        {
            let callee = match function.kind() {
                "identifier" => Some(function),
                "selector_expression" => function.child_by_field_name("field"),
                _ => None,
            };
            if callee.is_some_and(|name| node_text(name, source).contains("Sprintf")) {
                return true;
            }
        }
        push_children_reversed(node, &mut stack);
    }
    false
}

/// True when the subtree holds a `format!`/`concat!` macro invocation
/// (Rust only): `format!("SELECT ... {x}")` builds the query text
/// dynamically by construction. Nested function bodies are not entered,
/// mirroring `subtree_is_dynamic`.
fn subtree_contains_format_macro(root: Node<'_>, source: &[u8]) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.id() != root.id() && is_function(node, Language::Rust) {
            continue;
        }
        if node.kind() == "macro_invocation"
            && let Some(macro_name) = node.child_by_field_name("macro")
        {
            let text = node_text(macro_name, source);
            let name = text.rsplit("::").next().unwrap_or(text);
            if matches!(name, "format" | "concat") {
                return true;
            }
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
        if is_function(parent, language) {
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
    fn host_sql_sites_detect_go_query_and_sprintf() {
        let source = b"package dao\n\nimport \"database/sql\"\n\nfunc find(db *sql.DB, name string) {\n\tdb.Query(\"SELECT * FROM users WHERE name = '\" + name + \"'\")\n}\n\nfunc all(db *sql.DB) {\n\tdb.Query(fmt.Sprintf(\"SELECT * FROM users WHERE active = %v\", true))\n}\n";
        let sites = host_sql_sites("dao.go", source).unwrap();
        assert_eq!(sites.len(), 2, "{sites:?}");
        assert!(sites[0].dynamic, "{sites:?}");
        assert!(!sites[0].inside_loop, "{sites:?}");
        assert!(sites[1].dynamic, "{sites:?}");
    }

    #[test]
    fn host_sql_sites_ignore_go_parameterized_query() {
        let source = b"package dao\n\nfunc get(db *sql.DB, id int) {\n\tdb.QueryRow(\"SELECT * FROM users WHERE id = $1\", id)\n}\n";
        let sites = host_sql_sites("dao.go", source).unwrap();
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert!(!sites[0].dynamic, "{sites:?}");
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
    fn zig_literals_normalize_once_at_named_wrappers() {
        let tokens = normalized_tokens(
            "literal.zig",
            b"pub fn f(value: []const u8) void { const ch = 'x'; _ = value; }\n",
        )
        .unwrap();
        let texts = tokens
            .tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(texts.iter().filter(|text| **text == "<str>").count(), 1);
        assert!(!texts.contains(&"void"), "{texts:?}");
        assert!(!texts.contains(&"u8"), "{texts:?}");
        assert!(!texts.contains(&"'"), "{texts:?}");
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
    #[test]
    fn go_dependencies_preserve_grouped_import_order_and_paths() {
        let source = br#"package dao

import (
	"fmt"
	`github.com/example/mod/pkg`
	"database/sql"
)

func outer(db *sql.DB) {
	nested := func() {
		_ = fmt.Sprint(sql.ErrNoRows)
	}
	_ = nested
	_ = wrap(one(two(three(db))))
}
"#;
        let parsed = extract_dependencies("dao.go", source).unwrap();
        let references: Vec<_> = parsed
            .references
            .iter()
            .map(|reference| (reference.kind, reference.specifier.as_str(), reference.line))
            .collect();
        assert_eq!(
            references,
            vec![
                (RawDependencyKind::GoImport, "fmt", 4),
                (RawDependencyKind::GoImport, "github.com/example/mod/pkg", 5,),
                (RawDependencyKind::GoImport, "database/sql", 6),
            ]
        );
    }

    #[test]
    fn javascript_symbols_cover_every_import_and_export_form() {
        // Every form the extractor recognizes, one per line. Line 7 exports a
        // destructuring pattern, which introduces no single name.
        let source = br#"import defaultExport from "./a";
import * as namespace from "./b";
import { named, aliased as local } from "./c";
import "./side-effect";
export function declared() {}
export class Klass {}
export const [first, second] = pair;
export let counter = 0;
export var legacy = 1;
export { named, local as exposed };
export { named as reexported } from "./c";
export default function () {}
export * from "./d";
export * as grouped from "./e";
const internal = 1;
"#;
        let parsed = extract_dependencies("src/forms.ts", source).unwrap();
        let exports: Vec<(&str, u32, ExportKind)> = parsed
            .exports
            .iter()
            .map(|export| (export.name.as_str(), export.line, export.kind))
            .collect();
        assert_eq!(
            exports,
            vec![
                ("declared", 5, ExportKind::Named),
                ("Klass", 6, ExportKind::Named),
                ("counter", 8, ExportKind::Named),
                ("legacy", 9, ExportKind::Named),
                ("named", 10, ExportKind::Named),
                ("exposed", 10, ExportKind::Named),
                ("reexported", 11, ExportKind::Named),
                ("default", 12, ExportKind::Default),
                ("./d", 13, ExportKind::Star),
                ("./e", 14, ExportKind::Star),
            ]
        );
        let imports: Vec<(&str, u32, bool)> = parsed
            .imports
            .iter()
            .map(|import| (import.name.as_str(), import.line, import.star))
            .collect();
        assert_eq!(
            imports,
            vec![
                ("default", 1, false),
                ("", 2, true),
                ("named", 3, false),
                ("aliased", 3, false),
                ("named", 11, false),
                ("", 13, true),
                ("", 14, true),
            ]
        );
    }
}
