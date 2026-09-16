use leadline::discovery::discover_matching;
use leadline::sql::{accepts_sql, tokenize_statements};
use std::path::Path;
mod common;
use common::temporary_directory;

#[test]
fn discovery_sql_files_respect_excludes_ignores_and_symlinks() {
    let root = temporary_directory();
    std::fs::write(root.join("q.sql"), "SELECT 1;").unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/inner.sql"), "SELECT 2;").unwrap();
    std::fs::write(root.join("notes.txt"), "ignored").unwrap();
    std::fs::create_dir_all(root.join("node_modules")).unwrap();
    std::fs::write(root.join("node_modules/skip.sql"), "SELECT 3;").unwrap();
    std::fs::create_dir_all(root.join("gen")).unwrap();
    std::fs::write(root.join("gen/out.sql"), "SELECT 4;").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("q.sql"), root.join("link.sql")).unwrap();
    let found = discover_matching(&root, &["gen/**".to_owned()], accepts_sql).unwrap();
    let names: Vec<String> = found
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/")
        })
        .collect();
    assert!(names.iter().any(|name| name == "q.sql"), "{names:?}");
    assert!(
        names.iter().any(|name| name == "sub/inner.sql"),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|name| name.contains("node_modules")),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|name| name.starts_with("gen")),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|name| name.contains("notes.txt")),
        "{names:?}"
    );
    #[cfg(unix)]
    assert!(
        !names.iter().any(|name| name == "link.sql"),
        "symlinks are skipped: {names:?}"
    );
    // Explicit files honor the predicate.
    assert!(discover_matching(&root.join("q.sql"), &[], accepts_sql).is_ok());
    assert!(discover_matching(&root.join("notes.txt"), &[], accepts_sql).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_sql_predicate_is_case_sensitive() {
    assert!(accepts_sql(Path::new("queries.sql")));
    assert!(accepts_sql(Path::new("dir/migration.sql")));
    assert!(!accepts_sql(Path::new("queries.SQL")));
    assert!(!accepts_sql(Path::new("queries.ts")));
    assert!(!accepts_sql(Path::new("sql")));
}

#[test]
fn tokenizes_postgresql_quotes_and_comments() {
    let statements = tokenize_statements(b"SELECT ';'; UPDATE users SET active = false;").unwrap();
    assert_eq!(statements.len(), 2);
    assert_eq!(statements[1].start_line, 1);
    let source = b"SELECT E'a\\nb', \"Weird\" FROM t; /* block; comment */ SELECT $tag$dollar;$tag$; -- trailing; comment\nSELECT 2;";
    let statements = tokenize_statements(source).unwrap();
    assert_eq!(statements.len(), 3);
    assert_eq!(statements[2].start_line, 2);
    // E-strings escape the next character, so `\'` never closes them.
    let statements = tokenize_statements(b"SELECT E'it\\'s';").unwrap();
    assert_eq!(statements.len(), 1);
    assert_eq!(statements[0].tokens.len(), 2);
    // Nested parentheses track depth without splitting.
    let statements = tokenize_statements(b"SELECT (1 + (2 * 3));").unwrap();
    assert_eq!(statements.len(), 1);
    assert!(statements[0].tokens.iter().any(|token| token.depth == 2));
}

#[test]
fn tokenize_keeps_no_literal_text() {
    let source = b"SELECT * FROM users WHERE email = 'LITERAL_SENTINEL' AND code = 90210;";
    let statements = tokenize_statements(source).unwrap();
    assert_eq!(statements.len(), 1);
    let debug = format!("{:?}", statements);
    assert!(!debug.contains("LITERAL_SENTINEL"));
    // Integer shapes are retained for threshold rules but never serialized
    // into findings; only the value-carrying shape exists, never the text.
    assert!(
        statements[0]
            .tokens
            .iter()
            .any(|token| token.kind == leadline::sql::TokenKind::Int(90210))
    );
}

#[test]
fn rejects_unclosed_constructs_and_limits() {
    for source in [
        &b"SELECT 'oops;"[..],
        b"SELECT 1; /* never closed",
        b"SELECT $tag$ oops;",
        b"SELECT \"oops;",
    ] {
        assert!(tokenize_statements(source).is_err());
    }
    // Parenthesis nesting above 128 is rejected.
    let mut deep = vec![b'('; 129];
    deep.extend_from_slice(b"SELECT 1");
    deep.extend(vec![b')'; 129]);
    assert!(tokenize_statements(&deep).is_err());
    // Invalid UTF-8 and oversized inputs are rejected.
    assert!(tokenize_statements(&[0xff, 0xfe]).is_err());
    assert!(tokenize_statements(&vec![b'x'; (64 << 20) + 1]).is_err());
}

#[test]
fn detects_update_delete_without_where() {
    use leadline::sql::{SqlOptions, analyze_sql_bytes};
    let options = SqlOptions::default();
    let risky = analyze_sql_bytes("q.sql", b"UPDATE users SET active = false;", &options).unwrap();
    assert_eq!(risky.findings.len(), 1);
    assert_eq!(risky.findings[0].rule_id, "sql/update-delete-without-where");
    let safe = analyze_sql_bytes(
        "q.sql",
        b"UPDATE users SET active = false WHERE id = 1;",
        &options,
    )
    .unwrap();
    assert!(safe.findings.is_empty());
    let deleted = analyze_sql_bytes("q.sql", b"DELETE FROM sessions;", &options).unwrap();
    assert_eq!(deleted.findings.len(), 1);
    let kept =
        analyze_sql_bytes("q.sql", b"DELETE FROM sessions WHERE expired;", &options).unwrap();
    assert!(kept.findings.is_empty());
    // Comments around keywords and CTE prefixes do not hide the statement.
    let cte = analyze_sql_bytes(
        "q.sql",
        b"WITH old AS (SELECT id FROM users) UPDATE users SET active = false;",
        &options,
    )
    .unwrap();
    assert_eq!(cte.findings.len(), 1);
    // A WHERE inside a subquery does not satisfy the top-level statement.
    let sub = analyze_sql_bytes(
        "q.sql",
        b"UPDATE users SET active = (SELECT max(x) FROM t WHERE y = 1);",
        &options,
    )
    .unwrap();
    assert_eq!(sub.findings.len(), 1);
}

#[test]
fn detects_leading_wildcard() {
    use leadline::sql::{SqlOptions, analyze_sql_bytes};
    let options = SqlOptions::default();
    let risky = analyze_sql_bytes(
        "q.sql",
        b"SELECT * FROM users WHERE email LIKE '%x';",
        &options,
    )
    .unwrap();
    assert_eq!(risky.findings.len(), 1);
    assert_eq!(risky.findings[0].rule_id, "sql/leading-wildcard");
    let ilike = analyze_sql_bytes(
        "q.sql",
        b"SELECT * FROM users WHERE email ILIKE '%x';",
        &options,
    )
    .unwrap();
    assert_eq!(ilike.findings.len(), 1);
    for safe in [
        &b"SELECT * FROM users WHERE email LIKE 'x%';"[..],
        b"SELECT * FROM users WHERE email LIKE '\\%x';",
        // `E'\\%x'` decodes to `\%x`, where LIKE's escape keeps `%` literal.
        b"SELECT * FROM users WHERE email LIKE E'\\\\%x';",
        b"SELECT * FROM users WHERE email LIKE $1;",
        b"SELECT * FROM users WHERE email LIKE name;",
    ] {
        let report = analyze_sql_bytes("q.sql", safe, &options).unwrap();
        assert!(report.findings.is_empty(), "{safe:?}");
    }
    // E-strings consume the backslash, so `E'\%x'` is the literal `%x` and
    // starts with a wildcard regardless of LIKE's escape character.
    for risky in [
        &b"SELECT * FROM users WHERE email LIKE E'%x';"[..],
        b"SELECT * FROM users WHERE email LIKE E'\\%x';",
    ] {
        let report = analyze_sql_bytes("q.sql", risky, &options).unwrap();
        assert_eq!(report.findings.len(), 1, "{risky:?}");
    }
}

#[test]
fn detects_nonsargable_predicates() {
    use leadline::sql::{SqlOptions, analyze_sql_bytes};
    let options = SqlOptions::default();
    let wrapped = analyze_sql_bytes(
        "q.sql",
        b"SELECT * FROM users WHERE lower(email) = 'x';",
        &options,
    )
    .unwrap();
    assert_eq!(wrapped.findings.len(), 1);
    assert_eq!(wrapped.findings[0].rule_id, "sql/nonsargable-predicate");
    let cast = analyze_sql_bytes(
        "q.sql",
        b"SELECT * FROM users WHERE id::text = '7';",
        &options,
    )
    .unwrap();
    assert_eq!(cast.findings.len(), 1);
    for safe in [
        &b"SELECT * FROM users WHERE email = 'x';"[..],
        b"SELECT * FROM users WHERE lower('X') = email;",
        b"SELECT * FROM users WHERE myfunc(email) = 'x';",
        b"SELECT * FROM users WHERE '2024-01-01'::date = created;",
    ] {
        let report = analyze_sql_bytes("q.sql", safe, &options).unwrap();
        assert!(report.findings.is_empty(), "{safe:?}");
    }
}

#[test]
fn detects_large_offset() {
    use leadline::sql::{SqlOptions, analyze_sql_bytes};
    let options = SqlOptions::default();
    let big = analyze_sql_bytes(
        "q.sql",
        b"SELECT * FROM users ORDER BY id LIMIT 10 OFFSET 1001;",
        &options,
    )
    .unwrap();
    assert_eq!(big.findings.len(), 1);
    assert_eq!(big.findings[0].rule_id, "sql/large-offset");
    for safe in [
        &b"SELECT * FROM users ORDER BY id LIMIT 10 OFFSET 1000;"[..],
        b"SELECT * FROM users ORDER BY id LIMIT 10 OFFSET $1;",
        b"SELECT * FROM users WHERE id IN (SELECT id FROM t ORDER BY id LIMIT 10 OFFSET 5000);",
    ] {
        let report = analyze_sql_bytes("q.sql", safe, &options).unwrap();
        assert!(report.findings.is_empty(), "{safe:?}");
    }
}

#[test]
fn statement_rule_sort_by_path_line_rule() {
    use leadline::sql::{SqlOptions, analyze_sql_bytes};
    let source = b"UPDATE users SET active = false;
SELECT * FROM u WHERE e LIKE '%x';
SELECT * FROM u WHERE lower(e) = 'x';
SELECT * FROM u ORDER BY id LIMIT 1 OFFSET 2000;
";
    let report = analyze_sql_bytes("q.sql", source, &SqlOptions::default()).unwrap();
    assert_eq!(
        report
            .findings
            .iter()
            .map(|finding| finding.rule_id.as_str())
            .collect::<Vec<_>>(),
        [
            "sql/update-delete-without-where",
            "sql/leading-wildcard",
            "sql/nonsargable-predicate",
            "sql/large-offset",
        ]
    );
}

fn sql_file(path: &str, source: &[u8]) -> leadline::sql::SqlFile {
    leadline::sql::SqlFile {
        path: path.to_owned(),
        statements: leadline::sql::tokenize_statements(source).unwrap(),
    }
}

#[test]
fn migration_collects_declared_tables() {
    use leadline::sql::collect_declared_tables;
    let files = vec![
        sql_file("migrations/001.sql", b"CREATE TABLE public.users (id integer);"),
        sql_file(
            "migrations/002.sql",
            b"CREATE TABLE IF NOT EXISTS sessions (id integer); CREATE TABLE public.users (id integer);",
        ),
        sql_file("migrations/003.sql", b"CREATE TABLE \"Audit\" (id integer);"),
        // Storage modifiers sit between CREATE and TABLE; a table actually
        // named `temp` is still a name.
        sql_file(
            "migrations/004.sql",
            b"CREATE TEMP TABLE scratch (id integer); CREATE UNLOGGED TABLE logs (id integer); CREATE GLOBAL TEMPORARY TABLE staged (id integer); CREATE TABLE temp (id integer);",
        ),
    ];
    let declared = collect_declared_tables(&files).unwrap();
    assert!(declared.contains("public.users"));
    assert!(declared.contains("public.sessions"));
    assert!(declared.contains("public.Audit"));
    assert!(declared.contains("public.scratch"));
    assert!(declared.contains("public.logs"));
    assert!(declared.contains("public.staged"));
    assert!(declared.contains("public.temp"));
    // Duplicates collapse; bare names default to the public schema.
    assert_eq!(declared.len(), 7);
}

#[test]
fn every_cte_name_in_a_statement_is_known() {
    use leadline::sql::{SqlOptions, add_unknown_table_findings, analyze_sql_bytes};
    let source = b"WITH a AS (SELECT 1), b AS (SELECT * FROM a) SELECT * FROM b;";
    let mut report = analyze_sql_bytes("q.sql", source, &SqlOptions::default()).unwrap();
    let files = vec![sql_file("q.sql", source)];
    add_unknown_table_findings(
        &mut report,
        &files,
        &std::collections::BTreeSet::new(),
        true,
    );
    assert!(
        report.findings.is_empty(),
        "both CTE aliases must stay known: {:?}",
        report.findings
    );
}

#[test]
fn tokenizer_nests_block_comments_and_bounds_tokens() {
    // PostgreSQL nests block comments; text inside must not become tokens.
    let statements =
        tokenize_statements(b"/* outer /* inner */ still-comment */ SELECT 1;").unwrap();
    assert_eq!(statements.len(), 1);
    assert!(
        statements[0]
            .tokens
            .iter()
            .all(|token| !format!("{:?}", token.kind).contains("still"))
    );
    // A 64 MiB allowance of one-letter words must fail fast, not allocate
    // millions of owned token strings.
    let dense = "a ".repeat(1_000_001);
    assert!(tokenize_statements(dense.as_bytes()).is_err());
}

#[test]
fn analyze_sql_path_accepts_an_explicit_sql_file() {
    let root = temporary_directory();
    let file = root.join("query.sql");
    std::fs::write(&file, "UPDATE users SET active = false;").unwrap();
    let report =
        leadline::sql::analyze_sql_path(&file, &leadline::config::SqlConfig::default(), &[])
            .expect("explicit .sql files must analyze");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.rule_id == "sql/update-delete-without-where"),
        "{:?}",
        report.findings
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reports_tables_absent_from_migrations() {
    use leadline::sql::{SqlOptions, add_unknown_table_findings, analyze_sql_bytes};
    let mut report = analyze_sql_bytes(
        "q.sql",
        b"SELECT * FROM users; SELECT * FROM ghost; WITH mine AS (SELECT 1) SELECT * FROM mine;",
        &SqlOptions::default(),
    )
    .unwrap();
    assert!(!report.schema_evidence_available);
    let files = vec![sql_file(
        "q.sql",
        b"SELECT * FROM users; SELECT * FROM ghost; WITH mine AS (SELECT 1) SELECT * FROM mine;",
    )];
    let mut declared = std::collections::BTreeSet::new();
    declared.insert("public.users".to_owned());
    add_unknown_table_findings(&mut report, &files, &declared, true);
    assert!(report.schema_evidence_available);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.rule_id == "sql/unknown-table"),
        "ghost table must be reported"
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| finding.rule_id == "sql/unknown-table")
            .count(),
        1,
        "known users and CTE alias mine stay silent"
    );
    // Without configured roots nothing emits and evidence stays unavailable.
    let mut report =
        analyze_sql_bytes("q.sql", b"SELECT * FROM ghost;", &SqlOptions::default()).unwrap();
    add_unknown_table_findings(&mut report, &files, &declared, false);
    assert!(!report.schema_evidence_available);
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.rule_id == "sql/unknown-table")
    );
}

fn host_functions(path: &str, source: &[u8]) -> Vec<leadline::core::FunctionAnalysis> {
    leadline::analyze_source(path, source).unwrap().functions
}

#[test]
fn unknown_table_ignores_scalar_function_from_and_reads_comma_lists() {
    use leadline::sql::{SqlOptions, add_unknown_table_findings, analyze_sql_bytes};
    let source = b"SELECT extract(epoch FROM ts) FROM known;
SELECT trim(both from name) FROM known;
SELECT * FROM known, ghost_a g, ghost_b;
SELECT * FROM known, (SELECT 1) x;
SELECT * FROM known FOR UPDATE OF a, b;
INSERT INTO known (SELECT id FROM ghost_c);
SELECT substring(name FROM 2 FOR 3) FROM known;
WITH mine AS (SELECT 1) SELECT * FROM mine, ghost_d;
SELECT * FROM known WHERE id = ANY (SELECT id FROM ghost_e);
DELETE FROM known USING ghost_f, ghost_g;
CLUSTER known USING known_pkey;
";
    let mut report = analyze_sql_bytes("q.sql", source, &SqlOptions::default()).unwrap();
    let files = vec![sql_file("q.sql", source)];
    let mut declared = std::collections::BTreeSet::new();
    declared.insert("public.known".to_owned());
    add_unknown_table_findings(&mut report, &files, &declared, true);
    let mut lines: Vec<u32> = report
        .findings
        .iter()
        .filter(|finding| finding.rule_id == "sql/unknown-table")
        .map(|finding| finding.start_line)
        .collect();
    lines.sort_unstable();
    // ghost_a/ghost_b share the line-3 comma list and ghost_f/ghost_g the
    // line-10 USING list; scalar-function `FROM`s, the derived table,
    // `FOR UPDATE OF`, CTE and declared names, and the CLUSTER index name
    // stay silent.
    assert_eq!(lines, [3, 3, 6, 8, 9, 10, 10], "{:?}", report.findings);
}

#[test]
fn host_sql_matrix_across_languages() {
    use leadline::sql::analyze_host_sql;
    let ts = "export function run(db: any, ids: number[]) {\n  for (const id of ids) {\n    db.execute('SELECT ' + id);\n  }\n}\n";
    let js = "function run(db, ids) {\n  let i = 0;\n  while (i < ids.length) {\n    db.query('SELECT ' + ids[i]);\n    i++;\n  }\n}\n";
    let tsx = "export function run(db: any, x: string) {\n  return db.raw(`SELECT ${x}`);\n}\n";
    let java = r#"class Dao {
  void find(Db db, String name) {
    db.executeUpdate("UPDATE users SET n = '" + name + "'");
  }
}
"#;
    let cases = [
        ("src/a.ts", ts),
        ("src/b.js", js),
        ("src/c.tsx", tsx),
        ("src/Dao.java", java),
    ];
    for (path, source) in cases {
        let functions = host_functions(path, source.as_bytes());
        let findings = analyze_host_sql(path, source.as_bytes(), &functions).unwrap();
        let rules: Vec<&str> = findings
            .iter()
            .map(|finding| finding.rule_id.as_str())
            .collect();
        assert!(
            rules.contains(&"sql/dynamic-concatenation"),
            "{path}: {rules:?}"
        );
        assert!(
            findings.iter().all(|finding| finding.function_id.is_some()),
            "{path}: sites attribute to a function"
        );
        assert!(
            !serde_json::to_string(&findings).unwrap().contains("SELECT"),
            "{path}: no SQL text leaks"
        );
    }
    let functions = host_functions("src/a.ts", ts.as_bytes());
    let findings = analyze_host_sql("src/a.ts", ts.as_bytes(), &functions).unwrap();
    assert!(
        findings
            .iter()
            .any(|finding| finding.rule_id == "sql/query-in-loop")
    );
}

#[test]
fn host_sql_parameterized_calls_are_quiet() {
    use leadline::sql::analyze_host_sql;
    let source = "export async function get(pool: any, id: number) {\n  return pool.query('SELECT * FROM users WHERE id = $1', [id]);\n}\n";
    let functions = host_functions("src/db.ts", source.as_bytes());
    let findings = analyze_host_sql("src/db.ts", source.as_bytes(), &functions).unwrap();
    assert!(findings.is_empty());
}

#[test]
fn host_sql_nested_function_stops_loop_attribution() {
    use leadline::sql::analyze_host_sql;
    let source = "export function outer(db: any, ids: number[]) {\n  for (const id of ids) {\n    const run = () => db.query('SELECT ' + id);\n    run();\n  }\n}\n";
    let functions = host_functions("src/db.ts", source.as_bytes());
    let findings = analyze_host_sql("src/db.ts", source.as_bytes(), &functions).unwrap();
    assert!(
        findings
            .iter()
            .any(|finding| finding.rule_id == "sql/dynamic-concatenation")
    );
    assert!(
        findings
            .iter()
            .all(|finding| finding.rule_id != "sql/query-in-loop"),
        "nested function boundary stops the outer loop"
    );
}

#[test]
fn host_sql_non_calls_are_ignored() {
    use leadline::sql::analyze_host_sql;
    let source = "const query = 'SELECT 1';\nexport function get() {\n  return query;\n}\n";
    let functions = host_functions("src/db.ts", source.as_bytes());
    let findings = analyze_host_sql("src/db.ts", source.as_bytes(), &functions).unwrap();
    assert!(findings.is_empty());
}
