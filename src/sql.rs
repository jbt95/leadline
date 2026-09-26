//! Bounded PostgreSQL-oriented SQL tokenizer and statement splitter.
//!
//! One lexical pass handles comments, single/double/dollar-quoted strings,
//! and parenthesis depth without a parser dependency. Tokens retain shapes
//! (keyword, identifier, string wildcard prefix, integer) but never literal
//! text, so findings cannot leak source contents.

use std::collections::BTreeSet;
use std::path::Path;

/// Maximum source bytes per analyzed file.
const MAX_SQL_BYTES: u64 = 64 << 20;
/// Maximum parenthesis nesting depth.
const MAX_PAREN_DEPTH: usize = 128;
/// Maximum tokens per analyzed file: a 64 MiB input of one-letter words must
/// not expand into tens of millions of owned token strings.
const MAX_SQL_TOKENS: usize = 1_000_000;

/// Case-sensitive `.sql` file predicate for [`crate::discovery::discover_matching`].
pub fn accepts_sql(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "sql")
}

/// Token shape without literal text.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// Lowercase keyword.
    Keyword(String),
    /// Identifier: lowercase unless double-quoted (case preserved).
    Ident { name: String, quoted: bool },
    /// String literal: only whether it starts with an unescaped `%`.
    Str { leading_wildcard: bool },
    /// Integer literal value.
    Int(u64),
    /// Non-integer numeric literal (no value retained).
    Float,
    /// `$n` query parameter.
    Param,
    /// Operator or punctuation text (`::`, `=`, `.`, ...).
    Op(String),
}

/// One lexical token with its 1-based line and parenthesis depth.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: u32,
    pub depth: usize,
}

/// Semicolon-delimited statement with its line span.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub tokens: Vec<Token>,
    pub start_line: u32,
    pub end_line: u32,
}

/// Split PostgreSQL-oriented source into statements.
///
/// Rejects invalid UTF-8, unclosed quotes/comments/dollar quotes,
/// parenthesis nesting above 128, and inputs above 64 MiB. Statements split
/// only on depth-zero semicolons outside comments and literals.
pub fn tokenize_statements(source: &[u8]) -> crate::Result<Vec<Statement>> {
    if source.len() as u64 > MAX_SQL_BYTES {
        return Err("SQL input exceeds the 64 MiB limit".into());
    }
    let text = std::str::from_utf8(source).map_err(|_| "SQL input is not valid UTF-8")?;
    let lexer = Lexer::new(text);
    lexer.run()
}

/// Lowercase keywords the rules distinguish; everything else lexical that
/// looks like a word is an identifier.
const KEYWORDS: &[&str] = &[
    "select",
    "insert",
    "update",
    "delete",
    "where",
    "from",
    "join",
    "into",
    "values",
    "set",
    "like",
    "ilike",
    "offset",
    "limit",
    "group",
    "order",
    "returning",
    "union",
    "intersect",
    "except",
    "create",
    "table",
    "if",
    "not",
    "exists",
    "with",
    "as",
    "on",
];

fn keyword(word: &str) -> Option<&'static str> {
    KEYWORDS
        .iter()
        .find(|candidate| **candidate == word)
        .copied()
}

struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: u32,
    depth: usize,
    tokens: Vec<Token>,
    statements: Vec<Statement>,
    pushed: usize,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            pos: 0,
            line: 1,
            depth: 0,
            tokens: Vec::new(),
            statements: Vec::new(),
            pushed: 0,
        }
    }

    fn run(mut self) -> crate::Result<Vec<Statement>> {
        while self.pos < self.bytes.len() {
            self.skip_trivia()?;
            if self.pushed > MAX_SQL_TOKENS {
                return Err(format!("SQL input exceeds the {MAX_SQL_TOKENS}-token limit").into());
            }
            if self.pos >= self.bytes.len() {
                break;
            }
            let byte = self.bytes[self.pos];
            if byte == b';' && self.depth == 0 {
                self.pos += 1;
                self.flush_statement();
            } else if byte == b'\'' || byte == b'"' || byte == b'$' || byte.is_ascii_digit() {
                self.lex_literal_or_param()?;
            } else if is_word_start(byte) {
                self.lex_word()?;
            } else if byte == b'(' {
                self.depth += 1;
                if self.depth > MAX_PAREN_DEPTH {
                    return Err("SQL parenthesis nesting exceeds 128 levels".into());
                }
                self.push(TokenKind::Op("(".to_owned()));
                self.pos += 1;
            } else if byte == b')' {
                self.depth = self.depth.saturating_sub(1);
                self.push(TokenKind::Op(")".to_owned()));
                self.pos += 1;
            } else if byte.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                self.lex_operator();
            }
        }
        self.flush_statement();
        if self.pushed > MAX_SQL_TOKENS {
            return Err(format!("SQL input exceeds the {MAX_SQL_TOKENS}-token limit").into());
        }
        Ok(self.statements)
    }

    fn push(&mut self, kind: TokenKind) {
        self.pushed += 1;
        self.tokens.push(Token {
            kind,
            line: self.line,
            depth: self.depth,
        });
    }

    fn flush_statement(&mut self) {
        if self.tokens.is_empty() {
            return;
        }
        let tokens = std::mem::take(&mut self.tokens);
        let start_line = tokens.first().map_or(self.line, |token| token.line);
        let end_line = tokens.last().map_or(self.line, |token| token.line);
        self.statements.push(Statement {
            tokens,
            start_line,
            end_line,
        });
    }

    /// Whitespace, `--` line comments, and (nested) `/* */` block comments.
    fn skip_trivia(&mut self) -> crate::Result<()> {
        loop {
            while self.pos < self.bytes.len()
                && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\r' | b'\n')
            {
                if self.bytes[self.pos] == b'\n' {
                    self.line += 1;
                }
                self.pos += 1;
            }
            if self.bytes[self.pos..].starts_with(b"--") {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.bytes[self.pos..].starts_with(b"/*") {
                // PostgreSQL block comments nest: `/* /* */ */` is one comment.
                let mut depth = 0usize;
                loop {
                    if self.pos >= self.bytes.len() {
                        return Err("SQL block comment is never closed".into());
                    }
                    if self.bytes[self.pos..].starts_with(b"/*") {
                        depth += 1;
                        self.pos += 2;
                    } else if self.bytes[self.pos..].starts_with(b"*/") {
                        depth -= 1;
                        self.pos += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if self.bytes[self.pos] == b'\n' {
                            self.line += 1;
                        }
                        self.pos += 1;
                    }
                }
                continue;
            }
            break;
        }
        Ok(())
    }

    fn lex_word(&mut self) -> crate::Result<()> {
        let start = self.pos;
        while self.pos < self.bytes.len() && is_word_continue(self.bytes[self.pos]) {
            self.pos += 1;
        }
        let word = &self.bytes[start..self.pos];
        // `E'...'` escape strings: the prefix merges into the literal.
        if word.len() == 1
            && (word[0] == b'E' || word[0] == b'e')
            && self.bytes.get(self.pos) == Some(&b'\'')
        {
            self.lex_quoted(true)?;
            return Ok(());
        }
        let lower = String::from_utf8_lossy(word).to_ascii_lowercase();
        if let Some(matched) = keyword(lower.as_str()) {
            self.push(TokenKind::Keyword(matched.to_owned()));
        } else {
            self.push(TokenKind::Ident {
                name: lower,
                quoted: false,
            });
        }
        Ok(())
    }

    /// String, quoted-identifier, dollar-quote, number, or `$n` parameter.
    fn lex_literal_or_param(&mut self) -> crate::Result<()> {
        let byte = self.bytes[self.pos];
        if byte == b'"' {
            return self.lex_quoted_ident();
        }
        if byte == b'\'' {
            self.lex_quoted(false)?;
            return Ok(());
        }
        if byte == b'$' {
            if let Some(tag) = self.dollar_tag() {
                return self.lex_dollar_quoted(&tag);
            }
            let mut end = self.pos + 1;
            while end < self.bytes.len() && self.bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > self.pos + 1 {
                self.push(TokenKind::Param);
                self.pos = end;
                return Ok(());
            }
            self.push(TokenKind::Op("$".to_owned()));
            self.pos += 1;
            return Ok(());
        }
        self.lex_number();
        Ok(())
    }

    /// `$tag` opener length including both dollars, or `None`.
    fn dollar_tag(&self) -> Option<String> {
        if self.bytes.get(self.pos) != Some(&b'$') {
            return None;
        }
        let mut end = self.pos + 1;
        // A bare `$` followed by digits is a parameter, not a quote.
        if end < self.bytes.len() && self.bytes[end].is_ascii_digit() {
            return None;
        }
        while end < self.bytes.len() && is_tag_continue(self.bytes[end]) {
            end += 1;
        }
        if end < self.bytes.len() && self.bytes[end] == b'$' {
            return Some(String::from_utf8_lossy(&self.bytes[self.pos..=end]).into_owned());
        }
        None
    }

    fn lex_dollar_quoted(&mut self, tag: &str) -> crate::Result<()> {
        let rest = &self.bytes[self.pos + tag.len()..];
        let Some(end) = rest
            .windows(tag.len())
            .position(|window| window == tag.as_bytes())
        else {
            return Err("SQL dollar-quoted string is never closed".into());
        };
        // Dollar-quoted bodies never escape: only a leading `%` counts.
        let leading_wildcard = rest.first() == Some(&b'%');
        self.line += rest[..end].iter().filter(|byte| **byte == b'\n').count() as u32;
        self.push(TokenKind::Str { leading_wildcard });
        self.pos += tag.len() + end + tag.len();
        Ok(())
    }

    /// `'...'` string; `escaped` selects backslash-escape (`E''`) rules.
    fn lex_quoted(&mut self, escaped: bool) -> crate::Result<()> {
        debug_assert_eq!(self.bytes[self.pos], b'\'');
        let mut index = self.pos + 1;
        let mut first = true;
        let mut leading_wildcard = false;
        while index < self.bytes.len() {
            let byte = self.bytes[index];
            if byte == b'\n' {
                self.line += 1;
            }
            if byte == b'\'' {
                if self.bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    first = false;
                    continue;
                }
                // Closing quote: record the literal shape only.
                self.push(TokenKind::Str { leading_wildcard });
                self.pos = index + 1;
                return Ok(());
            }
            if escaped && byte == b'\\' && self.bytes.get(index + 1).is_some() {
                // E-strings consume the backslash (including before `'`, so an
                // escaped quote never closes the literal). A leading `\%`
                // decodes to `%`, which LIKE then reads as a wildcard, while
                // `\\%` leaves a backslash first and stays escaped.
                if first && self.bytes.get(index + 1) == Some(&b'%') {
                    leading_wildcard = true;
                }
                index += 2;
                first = false;
                continue;
            }
            if first && byte == b'%' {
                leading_wildcard = true;
            }
            first = false;
            index += 1;
        }
        Err("SQL string literal is never closed".into())
    }

    fn lex_quoted_ident(&mut self) -> crate::Result<()> {
        debug_assert_eq!(self.bytes[self.pos], b'"');
        let mut index = self.pos + 1;
        let mut name = Vec::new();
        while index < self.bytes.len() {
            let byte = self.bytes[index];
            if byte == b'"' {
                if self.bytes.get(index + 1) == Some(&b'"') {
                    name.push(b'"');
                    index += 2;
                    continue;
                }
                let text = String::from_utf8_lossy(&name).into_owned();
                self.push(TokenKind::Ident {
                    name: text,
                    quoted: true,
                });
                self.pos = index + 1;
                return Ok(());
            }
            if byte == b'\n' {
                self.line += 1;
            }
            name.push(byte);
            index += 1;
        }
        Err("SQL quoted identifier is never closed".into())
    }

    fn lex_number(&mut self) {
        let start = self.pos;
        while self.pos < self.bytes.len()
            && (self.bytes[self.pos].is_ascii_digit() || self.bytes[self.pos] == b'_')
        {
            self.pos += 1;
        }
        let mut is_float = false;
        if self.bytes.get(self.pos) == Some(&b'.')
            && self
                .bytes
                .get(self.pos + 1)
                .is_some_and(|byte| byte.is_ascii_digit())
        {
            is_float = true;
            self.pos += 1;
            while self.pos < self.bytes.len()
                && (self.bytes[self.pos].is_ascii_digit() || self.bytes[self.pos] == b'_')
            {
                self.pos += 1;
            }
        }
        if matches!(self.bytes.get(self.pos), Some(b'e') | Some(b'E')) {
            let mut end = self.pos + 1;
            if matches!(self.bytes.get(end), Some(b'+') | Some(b'-')) {
                end += 1;
            }
            if self
                .bytes
                .get(end)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                is_float = true;
                end += 1;
                while end < self.bytes.len()
                    && (self.bytes[end].is_ascii_digit() || self.bytes[end] == b'_')
                {
                    end += 1;
                }
                self.pos = end;
            }
        }
        if is_float {
            self.push(TokenKind::Float);
            return;
        }
        let digits: Vec<u8> = self.bytes[start..self.pos]
            .iter()
            .filter(|byte| **byte != b'_')
            .copied()
            .collect();
        match String::from_utf8_lossy(&digits).parse::<u64>() {
            Ok(value) => self.push(TokenKind::Int(value)),
            Err(_) => self.push(TokenKind::Float),
        }
    }

    fn lex_operator(&mut self) {
        for candidate in ["::", "<>", "!=", "<=", ">=", "||"] {
            if self.bytes[self.pos..].starts_with(candidate.as_bytes()) {
                self.push(TokenKind::Op(candidate.to_owned()));
                self.pos += candidate.len();
                return;
            }
        }
        let byte = self.bytes[self.pos];
        self.push(TokenKind::Op((byte as char).to_string()));
        self.pos += 1;
    }
}

fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn is_word_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

fn is_tag_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// Fixed rule identity: each ID, severity, and remediation is defined once.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize)]
pub enum SqlRule {
    UpdateDeleteWithoutWhere,
    LeadingWildcard,
    NonsargablePredicate,
    LargeOffset,
    UnknownTable,
    DynamicConcatenation,
    QueryInLoop,
}

impl SqlRule {
    pub fn id(self) -> &'static str {
        match self {
            SqlRule::UpdateDeleteWithoutWhere => "sql/update-delete-without-where",
            SqlRule::LeadingWildcard => "sql/leading-wildcard",
            SqlRule::NonsargablePredicate => "sql/nonsargable-predicate",
            SqlRule::LargeOffset => "sql/large-offset",
            SqlRule::UnknownTable => "sql/unknown-table",
            SqlRule::DynamicConcatenation => "sql/dynamic-concatenation",
            SqlRule::QueryInLoop => "sql/query-in-loop",
        }
    }

    pub fn severity(self) -> crate::security::SecuritySeverity {
        match self {
            SqlRule::UpdateDeleteWithoutWhere | SqlRule::DynamicConcatenation => {
                crate::security::SecuritySeverity::High
            }
            SqlRule::LeadingWildcard
            | SqlRule::NonsargablePredicate
            | SqlRule::LargeOffset
            | SqlRule::UnknownTable
            | SqlRule::QueryInLoop => crate::security::SecuritySeverity::Medium,
        }
    }

    pub fn remediation(self) -> &'static str {
        match self {
            SqlRule::UpdateDeleteWithoutWhere => {
                "Add a WHERE clause to bound the rows this statement touches."
            }
            SqlRule::LeadingWildcard => {
                "Avoid a leading wildcard in LIKE patterns or back it with a trigram index."
            }
            SqlRule::NonsargablePredicate => {
                "Compare the bare column so indexes stay usable; move functions and casts to the literal side."
            }
            SqlRule::LargeOffset => "Replace large OFFSET pagination with keyset pagination.",
            SqlRule::UnknownTable => {
                "Declare the table in a migration under a configured root or fix the reference."
            }
            SqlRule::DynamicConcatenation => {
                "Pass query text as a literal with bound parameters instead of concatenation."
            }
            SqlRule::QueryInLoop => {
                "Move the query out of the loop or batch it into one round trip."
            }
        }
    }
}

/// Tunable thresholds; rule severities stay fixed.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlOptions {
    pub large_offset: u64,
}

impl Default for SqlOptions {
    fn default() -> Self {
        Self { large_offset: 1000 }
    }
}

/// One normalized risk finding. Shapes only: never SQL or source text.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct SqlFinding {
    pub rule_id: String,
    pub severity: crate::security::SecuritySeverity,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub function_id: Option<String>,
    pub remediation: String,
}

/// Normalized report for analyzed SQL text.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct SqlReport {
    pub findings: Vec<SqlFinding>,
    /// False until migration-backed schema evidence is attached.
    pub schema_evidence_available: bool,
}

/// Analyze SQL text with the four statement-local rules.
pub fn analyze_sql_bytes(
    path: &str,
    source: &[u8],
    options: &SqlOptions,
) -> crate::Result<SqlReport> {
    let statements = tokenize_statements(source)?;
    let mut findings = Vec::new();
    for statement in &statements {
        findings.extend(statement_findings(path, statement, options));
    }
    sort_by_line(&mut findings);
    Ok(SqlReport {
        findings,
        schema_evidence_available: false,
    })
}

/// Report order within one file: line, then rule ID.
fn sort_by_line(findings: &mut [SqlFinding]) {
    findings.sort_by(|left, right| {
        left.start_line
            .cmp(&right.start_line)
            .then(left.rule_id.cmp(&right.rule_id))
    });
}

/// Report order across files: path, line, then rule ID.
fn sort_by_path(findings: &mut [SqlFinding]) {
    findings.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.start_line.cmp(&right.start_line))
            .then(left.rule_id.cmp(&right.rule_id))
    });
}

fn finding(rule: SqlRule, path: &str, line: u32, end_line: u32) -> SqlFinding {
    SqlFinding {
        rule_id: rule.id().to_owned(),
        severity: rule.severity(),
        path: path.to_owned(),
        start_line: line,
        end_line,
        function_id: None,
        remediation: rule.remediation().to_owned(),
    }
}

fn keyword_is(token: &Token, word: &str) -> bool {
    matches!(&token.kind, TokenKind::Keyword(found) if found == word)
}

fn keyword_in(token: &Token, words: &[&str]) -> bool {
    matches!(&token.kind, TokenKind::Keyword(found) if words.contains(&found.as_str()))
}

fn is_ident(token: &Token) -> bool {
    matches!(&token.kind, TokenKind::Ident { .. })
}

fn ident_name(token: &Token) -> Option<&str> {
    match &token.kind {
        TokenKind::Ident { name, .. } => Some(name),
        _ => None,
    }
}

/// The statement's verb when it is a real `SELECT`/`INSERT`/`UPDATE`/`DELETE`.
///
/// The verb must be the first depth-zero keyword: `ON DELETE` inside an
/// `ALTER TABLE`/`CREATE TABLE` foreign-key clause is a clause, not a DELETE
/// statement. A `WITH` keyword seen before any other depth-zero keyword (an
/// `EXPLAIN` prefix may precede it) switches to CTE scanning: CTE bodies are
/// parenthesized, so the first depth-zero verb after it is the main
/// statement's.
fn statement_kind(statement: &Statement) -> Option<&str> {
    let mut cte = false;
    for token in &statement.tokens {
        if token.depth != 0 {
            continue;
        }
        let TokenKind::Keyword(found) = &token.kind else {
            continue;
        };
        if let Some(verb) = dml_verb(found) {
            return Some(verb);
        }
        if !cte && keyword_is(token, "with") {
            cte = true;
            continue;
        }
        if !cte {
            return None;
        }
    }
    None
}

/// The four statement verbs, or `None` for any other keyword.
fn dml_verb(word: &str) -> Option<&str> {
    match word {
        "select" => Some("select"),
        "insert" => Some("insert"),
        "update" => Some("update"),
        "delete" => Some("delete"),
        _ => None,
    }
}

/// Top-level `UPDATE`/`DELETE` with no top-level `WHERE`.
fn update_delete_without_where(path: &str, statement: &Statement) -> Option<SqlFinding> {
    let keyword = statement_kind(statement)?;
    if keyword != "update" && keyword != "delete" {
        return None;
    }
    let keyword_line = statement
        .tokens
        .iter()
        .find_map(|token| (token.depth == 0 && keyword_is(token, keyword)).then_some(token.line))?;
    let guarded = statement
        .tokens
        .iter()
        .any(|token| token.depth == 0 && keyword_is(token, "where"));
    if guarded {
        return None;
    }
    Some(finding(
        SqlRule::UpdateDeleteWithoutWhere,
        path,
        keyword_line,
        statement.end_line,
    ))
}

/// `LIKE`/`ILIKE` with a literal leading-wildcard pattern at any depth.
fn leading_wildcard(path: &str, statement: &Statement) -> Option<SqlFinding> {
    for (index, token) in statement.tokens.iter().enumerate() {
        if !keyword_in(token, &["like", "ilike"]) {
            continue;
        }
        if let Some(next) = statement.tokens.get(index + 1)
            && matches!(
                &next.kind,
                TokenKind::Str {
                    leading_wildcard: true
                }
            )
        {
            return Some(finding(
                SqlRule::LeadingWildcard,
                path,
                next.line,
                statement.end_line,
            ));
        }
    }
    None
}

/// Functions that defeat index use when wrapping a filtered column.
const NONSARGABLE_BUILTINS: &[&str] = &["lower", "upper", "trim", "date", "date_trunc", "cast"];

/// Clause keywords ending a `WHERE` scan at the same depth.
const WHERE_ENDS: &[&str] = &[
    "group",
    "order",
    "limit",
    "offset",
    "returning",
    "union",
    "intersect",
    "except",
];

/// Comparison operators whose operands may wrap the filtered column.
fn is_comparison(token: &Token) -> bool {
    matches!(&token.kind, TokenKind::Op(found) if matches!(found.as_str(), "=" | "<>" | "!=" | "<" | "<=" | ">" | ">="))
}

/// A `WHERE` scan over its depth-zero clause: unknown and dynamic patterns
/// never trigger, and each statement yields at most one finding.
fn nonsargable_predicate(path: &str, statement: &Statement) -> Option<SqlFinding> {
    let tokens = &statement.tokens;
    for (start, token) in tokens.iter().enumerate() {
        if !keyword_is(token, "where") {
            continue;
        }
        let depth = token.depth;
        let mut index = start + 1;
        while index < tokens.len() {
            let current = &tokens[index];
            if current.depth < depth {
                break;
            }
            if current.depth == depth && keyword_in(current, WHERE_ENDS) {
                break;
            }
            if current.depth >= depth && is_comparison(current) && wraps_column(tokens, index) {
                return Some(finding(
                    SqlRule::NonsargablePredicate,
                    path,
                    current.line,
                    statement.end_line,
                ));
            }
            index += 1;
        }
    }
    None
}

/// True when either comparison side wraps a column identifier in a fixed
/// built-in call or an explicit cast. Literals never count as columns.
fn wraps_column(tokens: &[Token], op: usize) -> bool {
    // Left side: `ident :: type`.
    if op >= 3
        && let (Some(cast), Some(marks), Some(column)) =
            (tokens.get(op - 1), tokens.get(op - 2), tokens.get(op - 3))
        && ident_name(cast).is_some()
        && matches!(&marks.kind, TokenKind::Op(mark) if mark == "::")
        && is_ident(column)
    {
        return true;
    }
    // Left side: `builtin(ident ...`.
    if op >= 1
        && let Some(Token { kind, .. }) = tokens.get(op - 1)
        && matches!(kind, TokenKind::Op(mark) if mark == ")")
        && call_wraps_column(tokens, op - 1)
    {
        return true;
    }
    // Right side: mirror image.
    if let (Some(column), Some(marks), Some(cast)) =
        (tokens.get(op + 1), tokens.get(op + 2), tokens.get(op + 3))
        && is_ident(column)
        && matches!(&marks.kind, TokenKind::Op(mark) if mark == "::")
        && ident_name(cast).is_some()
    {
        return true;
    }
    if let Some(Token { kind, .. }) = tokens.get(op + 1)
        && matches!(kind, TokenKind::Ident { .. })
        && let Some(open) = tokens.get(op + 2)
        && matches!(&open.kind, TokenKind::Op(mark) if mark == "(")
        && call_wraps_column_at(tokens, op + 1)
    {
        return true;
    }
    false
}

/// True when the `)` at `close` closes a fixed built-in call whose first
/// argument is a column identifier.
fn call_wraps_column(tokens: &[Token], close: usize) -> bool {
    let mut depth = 0usize;
    let mut index = close;
    loop {
        match &tokens[index].kind {
            TokenKind::Op(mark) if mark == ")" => depth += 1,
            TokenKind::Op(mark) if mark == "(" => {
                depth -= 1;
                if depth == 0 {
                    return call_wraps_column_at(tokens, index.saturating_sub(1))
                        && first_argument_is_column(tokens, index);
                }
            }
            _ => {}
        }
        if index == 0 {
            return false;
        }
        index -= 1;
    }
}

/// True when `tokens[name]` is a fixed built-in function name.
fn call_wraps_column_at(tokens: &[Token], name: usize) -> bool {
    tokens.get(name).is_some_and(|token| {
        ident_name(token).is_some_and(|found| {
            NONSARGABLE_BUILTINS.contains(&found.to_ascii_lowercase().as_str())
        })
    })
}

/// True when the first argument after `(` is a column identifier.
fn first_argument_is_column(tokens: &[Token], open: usize) -> bool {
    tokens.get(open + 1).is_some_and(is_ident)
}

/// Numeric top-level `OFFSET` above the configured threshold.
fn large_offset(path: &str, statement: &Statement, options: &SqlOptions) -> Option<SqlFinding> {
    for (index, token) in statement.tokens.iter().enumerate() {
        if token.depth != 0 || !keyword_is(token, "offset") {
            continue;
        }
        if let Some(next) = statement.tokens.get(index + 1)
            && let TokenKind::Int(value) = next.kind
            && value > options.large_offset
        {
            return Some(finding(
                SqlRule::LargeOffset,
                path,
                next.line,
                statement.end_line,
            ));
        }
    }
    None
}

/// Map host-language query call sites to fixed findings.
///
/// Each dynamic site emits `sql/dynamic-concatenation`, each in-loop site
/// emits `sql/query-in-loop`. Sites carry the innermost containing function
/// with the security-enrichment span rule. Argument text and call receivers
/// never cross over.
pub fn analyze_host_sql(
    path: &str,
    source: &[u8],
    functions: &[crate::core::FunctionAnalysis],
) -> crate::Result<Vec<SqlFinding>> {
    let sites = crate::parser::host_sql_sites(path, source)?;
    Ok(host_sql_findings(path, &sites, functions))
}

/// Maps already-parsed host call sites to fixed findings.
fn host_sql_findings(
    path: &str,
    sites: &[crate::parser::HostSqlSite],
    functions: &[crate::core::FunctionAnalysis],
) -> Vec<SqlFinding> {
    let mut findings = Vec::new();
    for site in sites {
        let function_id = crate::core::innermost_containing(
            functions,
            site.line,
            |function| (function.start_line, function.end_line),
            |function| function.id.as_str(),
        )
        .map(|function| function.id.clone());
        if site.dynamic {
            findings.push(SqlFinding {
                rule_id: SqlRule::DynamicConcatenation.id().to_owned(),
                severity: SqlRule::DynamicConcatenation.severity(),
                path: path.to_owned(),
                start_line: site.line,
                end_line: site.end_line,
                function_id: function_id.clone(),
                remediation: SqlRule::DynamicConcatenation.remediation().to_owned(),
            });
        }
        if site.inside_loop {
            findings.push(SqlFinding {
                rule_id: SqlRule::QueryInLoop.id().to_owned(),
                severity: SqlRule::QueryInLoop.severity(),
                path: path.to_owned(),
                start_line: site.line,
                end_line: site.end_line,
                function_id,
                remediation: SqlRule::QueryInLoop.remediation().to_owned(),
            });
        }
    }
    sort_by_line(&mut findings);
    findings
}

/// Gate violations: findings at or above `minimum`, in report order.
pub fn sql_gate_violations(
    report: &SqlReport,
    minimum: crate::security::SecuritySeverity,
) -> Vec<&SqlFinding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.severity >= minimum)
        .collect()
}

/// Bounded agent projection: first `top` findings plus a `truncated` flag.
pub fn agent_json(report: &SqlReport, top: usize) -> serde_json::Value {
    crate::security::agent_page(&report.findings, top)
}

/// Human-readable terminal rendering, deterministic in report order.
pub fn terminal_text(report: &SqlReport) -> String {
    if report.findings.is_empty() {
        return "No SQL risks found.\n".to_owned();
    }
    let mut out = String::new();
    for finding in &report.findings {
        let function = finding
            .function_id
            .as_deref()
            .map(|id| format!(" in {id}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "{}:{} [{}] {}{}\n",
            finding.path,
            finding.start_line,
            finding.severity.as_str(),
            finding.rule_id,
            function,
        ));
    }
    out
}

/// Assembled report plus owned gate violations.
pub struct SqlOutcome {
    pub report: SqlReport,
    pub violations: Vec<SqlFinding>,
}

/// Split a report into projections and gate violations.
pub fn outcome(
    report: SqlReport,
    minimum: Option<crate::security::SecuritySeverity>,
) -> SqlOutcome {
    let violations = minimum
        .map(|level| sql_gate_violations(&report, level))
        .unwrap_or_default()
        .into_iter()
        .cloned()
        .collect();
    SqlOutcome { report, violations }
}

/// Four statement-local rule findings for one statement.
fn statement_findings(path: &str, statement: &Statement, options: &SqlOptions) -> Vec<SqlFinding> {
    let mut findings = Vec::new();
    if let Some(finding) = update_delete_without_where(path, statement) {
        findings.push(finding);
    }
    if let Some(finding) = leading_wildcard(path, statement) {
        findings.push(finding);
    }
    if let Some(finding) = nonsargable_predicate(path, statement) {
        findings.push(finding);
    }
    if let Some(finding) = large_offset(path, statement, options) {
        findings.push(finding);
    }
    findings
}

/// Analyze `.sql` files and host-language call sites under `root`.
///
/// Every file is read once: `.sql` text runs the tokenizer and statement
/// rules, supported sources run host-site analysis over the same bytes.
/// Migration declarations come from the configured roots first; without
/// roots no unknown-table findings emit. `excludes` carries
/// `[analysis].exclude` for both discovery walks. Findings sort by path,
/// line, rule.
pub fn analyze_sql_path(
    root: &std::path::Path,
    config: &crate::config::SqlConfig,
    excludes: &[String],
) -> crate::Result<SqlReport> {
    let options = SqlOptions {
        large_offset: config.large_offset,
    };
    let base: std::path::PathBuf = if root.is_dir() {
        root.to_path_buf()
    } else {
        root.parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf()
    };
    if !config.migration_roots.is_empty() && !root.is_dir() {
        return Err("sql migration roots require a directory analysis path".into());
    }
    for migration_root in &config.migration_roots {
        if !base.join(migration_root).is_dir() {
            return Err(format!("sql migration root not found: {migration_root}").into());
        }
    }
    let sql_paths = match crate::discovery::discover_matching(root, excludes, accepts_sql) {
        Ok(paths) => paths,
        Err(_) if root.is_file() => Vec::new(),
        Err(error) => return Err(error),
    };
    let mut report = SqlReport {
        findings: Vec::new(),
        schema_evidence_available: false,
    };
    let mut sql_files = Vec::with_capacity(sql_paths.len());
    for path in &sql_paths {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("cannot read SQL file {}: {error}", path.display()))?;
        let display = crate::normalized_relative_path(path, &base);
        let statements = tokenize_statements(&bytes)
            .map_err(|error| format!("invalid SQL {display}: {error}"))?;
        let file = SqlFile {
            path: display,
            statements,
        };
        for statement in &file.statements {
            report
                .findings
                .extend(statement_findings(&file.path, statement, &options));
        }
        sql_files.push(file);
    }
    // An explicit SQL file was already handled above; source discovery
    // rejects `.sql`, so only directory roots and host-language files run it.
    let host_paths = if root.is_file() && accepts_sql(root) {
        Vec::new()
    } else {
        crate::discovery::discover_with_excludes(root, excludes)?
    };
    for path in &host_paths {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("cannot read source file {}: {error}", path.display()))?;
        let display = crate::normalized_relative_path(path, &base);
        let (analysis, sites) = crate::parser::parse_sql_host(&display, &bytes)?;
        report
            .findings
            .extend(host_sql_findings(&display, &sites, &analysis.functions));
    }
    let migration_files: Vec<SqlFile> = sql_files
        .iter()
        .filter(|file| {
            config.migration_roots.iter().any(|migration_root| {
                file.path == *migration_root || file.path.starts_with(&format!("{migration_root}/"))
            })
        })
        .cloned()
        .collect();
    let declared = collect_declared_tables(&migration_files)?;
    add_unknown_table_findings(
        &mut report,
        &sql_files,
        &declared,
        !config.migration_roots.is_empty(),
    );
    sort_by_path(&mut report.findings);
    Ok(report)
}

/// One tokenized SQL file with its display path.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlFile {
    pub path: String,
    pub statements: Vec<Statement>,
}

/// Normalize a table reference to `schema.name`.
///
/// Unquoted identifiers fold to lowercase, quoted identifiers keep their
/// case, and bare names default to the `public` schema.
fn normalize_table(parts: &[(String, bool)]) -> Option<String> {
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let normalize = |(name, quoted): &(String, bool)| {
        if *quoted {
            name.clone()
        } else {
            name.to_ascii_lowercase()
        }
    };
    if parts.len() == 2 {
        Some(format!("{}.{}", normalize(&parts[0]), normalize(&parts[1])))
    } else {
        Some(format!("public.{}", normalize(&parts[0])))
    }
}

/// Read one possibly-qualified table name starting at `index`: an identifier
/// optionally joined by `.` parts. Returns the normalized name and the index
/// past the name.
fn read_table_name(tokens: &[Token], index: usize) -> Option<(String, usize)> {
    let mut parts = Vec::new();
    let mut at = index;
    while let Some(TokenKind::Ident { name, quoted }) = tokens.get(at).map(|token| &token.kind) {
        parts.push((name.clone(), *quoted));
        let dot_then_ident = matches!(
            tokens.get(at + 1).map(|token| &token.kind),
            Some(TokenKind::Op(mark)) if mark == "."
        ) && matches!(
            tokens.get(at + 2).map(|token| &token.kind),
            Some(TokenKind::Ident { .. })
        );
        if dot_then_ident {
            at += 2;
            continue;
        }
        at += 1;
        break;
    }
    normalize_table(&parts).map(|name| (name, at))
}

/// Modifiers between `TABLE` and the declared name. Storage modifiers
/// (`TEMP`, `UNLOGGED`, ...) only appear before `TABLE`, so a table named
/// `temp` still reads as a name here.
fn is_table_modifier(token: &Token) -> bool {
    plain_word(token, &["if", "not", "exists"])
}

/// `true` when the token is the given lowercase keyword or an unquoted
/// identifier spelling it. Keywords cover grammar words; identifier matches
/// cover `CREATE TEMP TABLE` and friends, which the keyword table omits.
fn plain_word(token: &Token, words: &[&str]) -> bool {
    match &token.kind {
        TokenKind::Keyword(found) => words.contains(&found.as_str()),
        TokenKind::Ident {
            name,
            quoted: false,
        } => words.contains(&name.as_str()),
        _ => false,
    }
}

/// Collect `CREATE TABLE` declarations and `ALTER TABLE ... RENAME TO`
/// targets across migration files.
///
/// Duplicate declarations collapse; the result holds normalized
/// `schema.name` entries. Rename targets stay declared so references after a
/// rename resolve instead of reading as unknown tables.
pub fn collect_declared_tables(files: &[SqlFile]) -> crate::Result<BTreeSet<String>> {
    let mut declared = BTreeSet::new();
    for file in files {
        for statement in &file.statements {
            let tokens = &statement.tokens;
            let mut index = 0;
            while index < tokens.len() {
                if keyword_is(&tokens[index], "create") {
                    // `CREATE [OR REPLACE] [GLOBAL|LOCAL] [TEMP|TEMPORARY|
                    // UNLOGGED] TABLE ...`; every modifier may be absent.
                    let mut table = index + 1;
                    while table < tokens.len()
                        && plain_word(
                            &tokens[table],
                            &[
                                "or",
                                "replace",
                                "global",
                                "local",
                                "temp",
                                "temporary",
                                "unlogged",
                            ],
                        )
                    {
                        table += 1;
                    }
                    if tokens
                        .get(table)
                        .is_some_and(|next| keyword_is(next, "table"))
                    {
                        table += 1;
                        while table < tokens.len() && is_table_modifier(&tokens[table]) {
                            table += 1;
                        }
                        if let Some((name, next)) = read_table_name(tokens, table) {
                            declared.insert(name);
                            index = next;
                            continue;
                        }
                    }
                } else if let Some((target, next)) = read_rename_target(tokens, index) {
                    declared.insert(target);
                    index = next;
                    continue;
                }
                index += 1;
            }
        }
    }
    Ok(declared)
}

/// `ALTER TABLE [IF EXISTS] [ONLY] old RENAME TO new` → `new` and the index
/// past it. Column and constraint renames (`RENAME COLUMN|CONSTRAINT`) do not
/// match, and neither do non-table `ALTER ... RENAME TO` forms.
fn read_rename_target(tokens: &[Token], index: usize) -> Option<(String, usize)> {
    if !plain_word(tokens.get(index)?, &["alter"]) {
        return None;
    }
    let mut cursor = index + 1;
    if !tokens
        .get(cursor)
        .is_some_and(|token| plain_word(token, &["table"]))
    {
        return None;
    }
    cursor += 1;
    while tokens
        .get(cursor)
        .is_some_and(|token| plain_word(token, &["if", "exists", "only"]))
    {
        cursor += 1;
    }
    let (_, mut cursor) = read_table_name(tokens, cursor)?;
    if !tokens
        .get(cursor)
        .is_some_and(|token| plain_word(token, &["rename"]))
    {
        return None;
    }
    cursor += 1;
    if !tokens
        .get(cursor)
        .is_some_and(|token| plain_word(token, &["to"]))
    {
        return None;
    }
    cursor += 1;
    let (target, next) = read_table_name(tokens, cursor)?;
    Some((target, next))
}

/// CTE names defined by one statement: depth-zero identifiers after `WITH`
/// (or a depth-zero comma) that are immediately followed by `AS`.
fn cte_names(statement: &Statement) -> BTreeSet<String> {
    fn normalized(name: &str, quoted: bool) -> String {
        if quoted {
            name.to_owned()
        } else {
            name.to_ascii_lowercase()
        }
    }
    let mut names = BTreeSet::new();
    let tokens = &statement.tokens;
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].depth == 0 && keyword_is(&tokens[index], "with") {
            index += 1;
            while index < tokens.len() {
                let token = &tokens[index];
                // CTE bodies sit one level deeper: skip them so later CTE
                // names (`WITH a AS (...), b AS (...)`) are still collected.
                if token.depth != 0 {
                    index += 1;
                    continue;
                }
                if keyword_in(token, &["select", "insert", "update", "delete"]) {
                    break;
                }
                if let TokenKind::Ident { name, quoted } = &token.kind
                    && tokens
                        .get(index + 1)
                        .is_some_and(|next| keyword_is(next, "as"))
                {
                    names.insert(normalized(name, *quoted));
                }
                index += 1;
            }
            break;
        }
        index += 1;
    }
    names
}

/// Table reference with its source line.
struct TableRef {
    name: String,
    line: u32,
    end_line: u32,
}

/// Clause words that end a `FROM` item list at their depth.
const FROM_LIST_ENDS: &[&str] = &[
    "where",
    "group",
    "having",
    "order",
    "window",
    "limit",
    "offset",
    "fetch",
    "for",
    "returning",
    "union",
    "intersect",
    "except",
];

/// Words that precede a parenthesized subquery rather than an argument list,
/// even though they lex as identifiers.
const SUBQUERY_INTRODUCERS: &[&str] = &["any", "all", "some", "in", "lateral"];

/// True when the `(` at `index` opens a call's argument list. Column lists
/// after `INSERT INTO` are not calls even though an identifier precedes them.
fn is_call_paren(tokens: &[Token], index: usize) -> bool {
    if index == 0 || !is_ident(&tokens[index - 1]) {
        return false;
    }
    if matches!(
        tokens.get(index.wrapping_sub(2)).map(|token| &token.kind),
        Some(TokenKind::Keyword(word)) if word == "into"
    ) {
        return false;
    }
    ident_name(&tokens[index - 1]).is_some_and(|name| !SUBQUERY_INTRODUCERS.contains(&name))
}

/// One table reference at `at`, skipping table functions and CTE aliases.
fn table_ref_at(
    tokens: &[Token],
    at: usize,
    ctes: &BTreeSet<String>,
    end_line: u32,
) -> Option<TableRef> {
    let (name, after) = read_table_name(tokens, at)?;
    let is_call = tokens
        .get(after)
        .is_some_and(|token| matches!(&token.kind, TokenKind::Op(mark) if mark == "("));
    let qualified = matches!(
        tokens.get(at + 1).map(|token| &token.kind),
        Some(TokenKind::Op(mark)) if mark == "."
    );
    // CTEs are always bare names: compare the first segment with the same
    // case rule; qualified references consult declarations only.
    let first = match &tokens[at].kind {
        TokenKind::Ident { name, quoted } if *quoted => name.clone(),
        TokenKind::Ident { name, .. } => name.to_ascii_lowercase(),
        _ => String::new(),
    };
    let is_cte = !qualified && ctes.contains(&first);
    if is_call || is_cte {
        return None;
    }
    Some(TableRef {
        name,
        line: tokens[at].line,
        end_line,
    })
}

/// Table references in one statement: `FROM`/`JOIN` targets (skipping table
/// functions, CTE aliases, and the scalar-function `FROM` of `extract` and
/// friends), comma-separated `FROM` items, `UPDATE` targets, and `INSERT INTO`
/// targets, at any depth so subqueries are covered.
fn table_refs(statement: &Statement) -> Vec<TableRef> {
    let tokens = &statement.tokens;
    let ctes = cte_names(statement);
    let delete_statement = statement_kind(statement) == Some("delete");
    let mut refs = Vec::new();
    // One flag per open parenthesis: argument-list parens hide their
    // interiors because scalar functions take `extract(epoch FROM ts)`.
    let mut calls = Vec::new();
    let mut call_depth = 0usize;
    // Depths of `FROM` item lists still expecting `,`-separated items.
    let mut from_lists: Vec<usize> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        match &token.kind {
            TokenKind::Op(mark) if mark == "(" => {
                let call = is_call_paren(tokens, index);
                calls.push(call);
                call_depth += usize::from(call);
            }
            TokenKind::Op(mark) if mark == ")" => {
                if let Some(call) = calls.pop()
                    && call
                {
                    call_depth -= 1;
                }
                from_lists.retain(|depth| *depth <= token.depth);
            }
            TokenKind::Op(mark) if mark == "," => {
                if from_lists.contains(&token.depth) {
                    let at = skip_only(tokens, index + 1);
                    if let Some(reference) = table_ref_at(tokens, at, &ctes, statement.end_line) {
                        refs.push(reference);
                    }
                }
            }
            _ if call_depth == 0 => {
                // `INTO` counts only as `INSERT INTO`; `UPDATE` after `FOR`
                // is lock syntax, not a table target.
                let takes_name = if keyword_is(token, "into") {
                    tokens
                        .get(index.wrapping_sub(1))
                        .is_some_and(|prev| keyword_is(prev, "insert"))
                } else if keyword_is(token, "update") {
                    !tokens
                        .get(index.wrapping_sub(1))
                        .is_some_and(|prev| plain_word(prev, &["for"]))
                } else if plain_word(token, &["using"]) {
                    // `DELETE ... USING` lists sources; `JOIN ... USING
                    // (col)`, `CREATE INDEX ... USING gin`, and `CLUSTER ...
                    // USING idx` name a column list or method, never a table.
                    delete_statement
                } else {
                    keyword_in(token, &["from", "join"])
                };
                if takes_name {
                    let at = skip_only(tokens, index + 1);
                    if let Some(reference) = table_ref_at(tokens, at, &ctes, statement.end_line) {
                        refs.push(reference);
                    }
                    if keyword_is(token, "from") && !from_lists.contains(&token.depth) {
                        from_lists.push(token.depth);
                    }
                } else if from_lists.contains(&token.depth)
                    && FROM_LIST_ENDS.iter().any(|word| plain_word(token, &[word]))
                {
                    let depth = token.depth;
                    from_lists.retain(|candidate| *candidate < depth);
                }
            }
            _ => {}
        }
        index += 1;
    }
    refs
}

/// `FROM ONLY t`: skip the qualifier when present.
fn skip_only(tokens: &[Token], at: usize) -> usize {
    if tokens
        .get(at)
        .is_some_and(|token| matches!(&token.kind, TokenKind::Ident { name, .. } if name == "only"))
    {
        at + 1
    } else {
        at
    }
}

/// Append `sql/unknown-table` findings for references missing from migration
/// declarations.
///
/// Without configured migration roots nothing emits and
/// `schema_evidence_available` stays false; configured-but-empty roots mean
/// available evidence with zero declarations. Findings re-sort the report by
/// path, line, and rule ID.
///
/// Note: `files` extends the plan signature because references need token
/// context no finding row retains (reports never carry SQL text).
pub fn add_unknown_table_findings(
    report: &mut SqlReport,
    files: &[SqlFile],
    declared: &BTreeSet<String>,
    migrations_configured: bool,
) {
    if !migrations_configured {
        report.schema_evidence_available = false;
        return;
    }
    report.schema_evidence_available = true;
    for file in files {
        for statement in &file.statements {
            for reference in table_refs(statement) {
                if !declared.contains(&reference.name) {
                    report.findings.push(finding(
                        SqlRule::UnknownTable,
                        &file.path,
                        reference.line,
                        reference.end_line,
                    ));
                }
            }
        }
    }
    sort_by_path(&mut report.findings);
}

#[cfg(test)]
mod tests {
    use super::{Lexer, TokenKind};

    /// Drive `lex_number` over `text` and report the token it produced and how
    /// far it consumed. The consumed length matters as much as the kind: a
    /// wrong `pos` desynchronizes every later token in the statement.
    fn number(text: &str) -> (TokenKind, usize) {
        let mut lexer = Lexer::new(text);
        lexer.lex_number();
        let kind = lexer
            .tokens
            .first()
            .expect("lex_number pushes exactly one token")
            .kind
            .clone();
        (kind, lexer.pos)
    }

    #[test]
    fn lexes_integer_float_and_exponent_forms() {
        use TokenKind::{Float, Int};
        for (text, expected, consumed) in [
            ("42", Int(42), 2),
            ("1_000", Int(1000), 5),
            ("1.5", Float, 3),
            ("1_0.2_5", Float, 7),
            ("1e3", Float, 3),
            ("1E3", Float, 3),
            ("1e+3", Float, 4),
            ("1e-3", Float, 4),
            ("1.5e-3", Float, 6),
            ("99999999999999999999999", Float, 23),
        ] {
            assert_eq!(number(text), (expected, consumed), "{text:?}");
        }
    }

    /// A dot or exponent with no digit after it is not part of the number, so
    /// the number ends before it and the next token starts there. Anything
    /// else would swallow the following operator as numeric text.
    #[test]
    fn stops_before_an_incomplete_fraction_or_exponent() {
        use TokenKind::Int;
        for (text, consumed) in [("1.", 1), ("1e", 1), ("1e+", 1), ("1e-x", 1)] {
            assert_eq!(number(text), (Int(1), consumed), "{text:?}");
        }
    }
}
