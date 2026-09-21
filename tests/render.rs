//! CI format rendering over one check document.

use leadline::render::{
    CiFormat, Finding, GateLimits, GateSummary, findings_from_check_json, parse_format, render,
    summary_from_check_json,
};
use serde_json::{Value, json};

/// Gate limits of the fixture: cognitive 15, cyclomatic 10, nesting 4, CRAP 30.
fn limits() -> GateLimits {
    GateLimits {
        cognitive: Some(15.0),
        cyclomatic: Some(10.0),
        max_nesting: Some(4.0),
        crap: Some(30.0),
    }
}

/// One saved check document: a function over the cognitive limit, a clean
/// function, one parse error, and one high SQL violation.
fn document() -> Value {
    json!({
        "schema_version": 1,
        "metric_profile": "default",
        "analyzer_version": "0.15.0",
        "files": [
            {
                "path": "src/app.ts",
                "language": "typescript",
                "functions": [
                    {
                        "name": "handle",
                        "id": "function:handle:0:10",
                        "kind": "function",
                        "start_line": 12,
                        "end_line": 40,
                        "start_byte": 0,
                        "end_byte": 10,
                        "metrics": {
                            "loc": 29,
                            "logical_loc": 20,
                            "function_length": 29,
                            "parameters": 3,
                            "max_nesting": 2,
                            "cyclomatic": 4,
                            "cognitive": 21,
                            "coverage": null,
                            "crap": 12.0
                        },
                        "contributions": []
                    },
                    {
                        "name": "clean",
                        "id": "function:clean:10:20",
                        "kind": "function",
                        "start_line": 42,
                        "end_line": 44,
                        "start_byte": 10,
                        "end_byte": 20,
                        "metrics": {
                            "loc": 3,
                            "logical_loc": 1,
                            "function_length": 3,
                            "parameters": 0,
                            "max_nesting": 1,
                            "cyclomatic": 2,
                            "cognitive": 3,
                            "coverage": 1.0,
                            "crap": 2.0
                        },
                        "contributions": []
                    }
                ],
                "parse_errors": [
                    {
                        "kind": "ERROR",
                        "start_line": 7,
                        "start_column": 1,
                        "end_line": 7,
                        "end_column": 4
                    }
                ]
            }
        ],
        "sql_violations": [
            {
                "rule_id": "sql/select-star",
                "severity": "high",
                "path": "db/queries.sql",
                "start_line": 3,
                "end_line": 3,
                "function_id": null,
                "remediation": "Select only the columns you need."
            }
        ]
    })
}

/// A check document whose only function stays inside every limit.
fn clean_document() -> Value {
    json!({
        "schema_version": 1,
        "metric_profile": "default",
        "analyzer_version": "0.15.0",
        "files": [
            {
                "path": "src/app.ts",
                "language": "typescript",
                "functions": [
                    {
                        "name": "handle",
                        "start_line": 12,
                        "end_line": 40,
                        "metrics": {
                            "max_nesting": 1.0,
                            "cyclomatic": 2.0,
                            "cognitive": 3.0,
                            "crap": 2.0
                        }
                    }
                ],
                "parse_errors": []
            }
        ]
    })
}

fn fixture_findings() -> Vec<Finding> {
    findings_from_check_json(&document(), &limits()).unwrap()
}

fn fixture_summary() -> GateSummary {
    summary_from_check_json(&document(), &limits()).unwrap()
}

fn codeclimate() -> Value {
    serde_json::from_str(&render(
        &fixture_findings(),
        &fixture_summary(),
        CiFormat::CodeClimate,
    ))
    .unwrap()
}

#[test]
fn compact_lists_one_line_per_finding() {
    assert_eq!(
        render(&fixture_findings(), &fixture_summary(), CiFormat::Compact),
        "\
db/queries.sql:3 sql high Select only the columns you need.
src/app.ts:7 parse_error low ERROR
src/app.ts:12 function medium exceeds cognitive
"
    );
}

#[test]
fn annotations_use_the_workflow_command_shape_and_severity_levels() {
    assert_eq!(
        render(
            &fixture_findings(),
            &fixture_summary(),
            CiFormat::GithubAnnotations
        ),
        "\
::error file=db/queries.sql,line=3,title=sql::Select only the columns you need.
::warning file=src/app.ts,line=7,title=parse_error::ERROR
::warning file=src/app.ts,line=12,title=function::exceeds cognitive
"
    );
}

#[test]
fn annotations_escape_properties_but_not_message_data() {
    let finding = Finding {
        path: "src/a,b:c.ts".to_owned(),
        line: 4,
        end_line: None,
        kind: "function",
        severity: "critical",
        message: "100% of,\nlines".to_owned(),
        fingerprint: String::new(),
    };
    let summary = GateSummary {
        passed: false,
        files: 1,
        violations: 1,
    };
    assert_eq!(
        render(
            std::slice::from_ref(&finding),
            &summary,
            CiFormat::GithubAnnotations
        ),
        "::error file=src/a%2Cb%3Ac.ts,line=4,title=function::100%25 of,%0Alines\n"
    );
    assert_eq!(
        render(&[finding], &summary, CiFormat::Compact),
        "src/a,b:c.ts:4 function critical 100% of, lines\n"
    );
}

#[test]
fn codeclimate_issues_carry_locations_the_severity_map_and_fingerprints() {
    let first = codeclimate();
    assert_eq!(first, codeclimate());
    let issues = first.as_array().unwrap();
    assert_eq!(issues.len(), 3);
    assert_eq!(
        issues
            .iter()
            .map(|issue| issue["location"]["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["db/queries.sql", "src/app.ts", "src/app.ts"]
    );
    assert_eq!(
        issues
            .iter()
            .map(|issue| issue["severity"].as_str().unwrap())
            .collect::<Vec<_>>(),
        // high -> critical, low -> minor, medium -> major
        vec!["critical", "minor", "major"]
    );
    let issue = &issues[0];
    let mut keys = issue
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "categories",
            "check_name",
            "description",
            "fingerprint",
            "location",
            "severity",
            "type"
        ]
    );
    assert_eq!(issue["type"], "issue");
    assert_eq!(issue["check_name"], "sql");
    assert_eq!(issue["description"], "Select only the columns you need.");
    assert_eq!(issue["categories"], json!(["Bug Risk"]));
    assert_eq!(issue["location"]["path"], "db/queries.sql");
    assert_eq!(issue["location"]["lines"]["begin"], 3);
    assert_eq!(issue["location"]["lines"]["end"], 3);
    assert_eq!(issues[1]["location"]["lines"]["begin"], 7);
    assert_eq!(issues[2]["location"]["lines"]["begin"], 12);
    assert_eq!(issues[2]["location"]["lines"]["end"], 40);
    let mut fingerprints = issues
        .iter()
        .map(|issue| issue["fingerprint"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    for fingerprint in &fingerprints {
        assert_eq!(fingerprint.len(), 16);
        assert!(
            fingerprint
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
    }
    fingerprints.sort_unstable();
    fingerprints.dedup();
    assert_eq!(fingerprints.len(), 3);
}

#[test]
fn markdown_and_github_summary_render_the_same_document() {
    let markdown = render(&fixture_findings(), &fixture_summary(), CiFormat::Markdown);
    assert_eq!(
        markdown,
        render(
            &fixture_findings(),
            &fixture_summary(),
            CiFormat::GithubSummary
        )
    );
    assert!(markdown.starts_with("## leadline check\n\n**failing** - 3 violations in 1 file\n"));
    assert!(markdown.contains("| Kind | Count |\n| --- | --- |\n| function | 1 |"));
    assert!(markdown.contains("| parse_error | 1 |"));
    assert!(markdown.contains("| sql | 1 |"));
    assert!(
        markdown
            .contains("| db/queries.sql | 3 | sql | high | Select only the columns you need. |")
    );
    assert!(markdown.contains("| src/app.ts | 7 | parse_error | low | ERROR |"));
    assert!(markdown.contains("| src/app.ts | 12 | function | medium | exceeds cognitive |"));
}

#[test]
fn badge_is_stable_and_carries_only_the_verdict_and_count() {
    assert!(!fixture_summary().passed);
    assert_eq!(fixture_summary().violations, 3);
    assert_eq!(fixture_summary().files, 1);
    let badge = render(&fixture_findings(), &fixture_summary(), CiFormat::Badge);
    assert_eq!(
        badge,
        render(&fixture_findings(), &fixture_summary(), CiFormat::Badge)
    );
    assert!(badge.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(badge.contains("height=\"20\""));
    assert!(badge.contains("<title>leadline: failing 3</title>"));
    assert!(badge.contains("#e05d44"));

    let passing = render(
        &[],
        &GateSummary {
            passed: true,
            files: 1,
            violations: 0,
        },
        CiFormat::Badge,
    );
    assert!(passing.contains("<title>leadline: passing 0</title>"));
    assert!(passing.contains("#4c1"));
}

#[test]
fn every_format_renders_identical_bytes_twice() {
    let first = (fixture_findings(), fixture_summary());
    let second = (fixture_findings(), fixture_summary());
    for format in [
        CiFormat::CodeClimate,
        CiFormat::GithubAnnotations,
        CiFormat::GithubSummary,
        CiFormat::Markdown,
        CiFormat::Badge,
        CiFormat::Compact,
    ] {
        assert_eq!(
            render(&first.0, &first.1, format),
            render(&second.0, &second.1, format),
            "{format:?} must be deterministic"
        );
    }
}

#[test]
fn a_clean_document_passes_without_rendering_rows() {
    let document = clean_document();
    let findings = findings_from_check_json(&document, &limits()).unwrap();
    let summary = summary_from_check_json(&document, &limits()).unwrap();
    assert!(findings.is_empty());
    assert_eq!(
        summary,
        GateSummary {
            passed: true,
            files: 1,
            violations: 0
        }
    );
    assert_eq!(render(&findings, &summary, CiFormat::Compact), "");
    assert_eq!(render(&findings, &summary, CiFormat::GithubAnnotations), "");
    assert_eq!(render(&findings, &summary, CiFormat::CodeClimate), "[]\n");
    assert!(
        render(&findings, &summary, CiFormat::GithubSummary)
            .contains("**passing** - 0 violations in 1 file")
    );
    assert!(render(&findings, &summary, CiFormat::Markdown).contains("No findings."));
}

#[test]
fn function_findings_follow_the_gate_order() {
    let document = json!({
        "files": [{
            "path": "src/gate.ts",
            "functions": [{
                "name": "wide",
                "start_line": 4,
                "end_line": 9,
                "metrics": {
                    "cognitive": 16,
                    "cyclomatic": 11,
                    "max_nesting": 5,
                    "crap": null
                }
            }],
            "parse_errors": []
        }]
    });
    let findings = findings_from_check_json(&document, &limits()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].path, "src/gate.ts");
    assert_eq!(findings[0].line, 4);
    assert_eq!(findings[0].end_line, Some(9));
    assert_eq!(findings[0].kind, "function");
    assert_eq!(findings[0].severity, "medium");
    assert_eq!(
        findings[0].message,
        "exceeds cognitive, cyclomatic, max_nesting, crap_unavailable"
    );

    let without_crap = GateLimits {
        crap: None,
        ..limits()
    };
    assert_eq!(
        findings_from_check_json(&document, &without_crap).unwrap()[0].message,
        "exceeds cognitive, cyclomatic, max_nesting"
    );
}

#[test]
fn a_scanner_only_document_reports_its_own_paths() {
    let document = json!({
        "sql_violations": [{
            "rule_id": "sql/no-where",
            "severity": "critical",
            "path": "db/q.sql",
            "start_line": 2,
            "end_line": 2,
            "remediation": "Add a WHERE clause."
        }]
    });
    let findings = findings_from_check_json(&document, &limits()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, "sql");
    assert_eq!(findings[0].severity, "critical");
    assert_eq!(findings[0].message, "Add a WHERE clause.");
    assert_eq!(
        summary_from_check_json(&document, &limits()).unwrap(),
        GateSummary {
            passed: false,
            files: 1,
            violations: 1
        }
    );
}

#[test]
fn a_document_that_is_not_a_check_report_is_rejected() {
    for document in [
        json!({}),
        json!({ "tool": "check", "violations": [] }),
        json!({ "schema_version": 1, "analyzer_version": "0.15.0" }),
        json!([1, 2]),
    ] {
        assert!(
            findings_from_check_json(&document, &limits()).is_err(),
            "{document} is not a check report"
        );
        assert!(summary_from_check_json(&document, &limits()).is_err());
    }
    assert!(findings_from_check_json(&json!({ "files": "nope" }), &limits()).is_err());
}

#[test]
fn parse_format_accepts_every_name_and_the_gitlab_alias() {
    assert_eq!(parse_format("codeclimate"), Some(CiFormat::CodeClimate));
    assert_eq!(
        parse_format("gitlab-codequality"),
        Some(CiFormat::CodeClimate)
    );
    assert_eq!(
        parse_format("github-annotations"),
        Some(CiFormat::GithubAnnotations)
    );
    assert_eq!(
        parse_format("github-summary"),
        Some(CiFormat::GithubSummary)
    );
    assert_eq!(parse_format("markdown"), Some(CiFormat::Markdown));
    assert_eq!(parse_format("badge"), Some(CiFormat::Badge));
    assert_eq!(parse_format("compact"), Some(CiFormat::Compact));
    assert_eq!(parse_format("sarif"), None);
    assert_eq!(parse_format("unknown"), None);
}
