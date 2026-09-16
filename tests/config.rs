use leadline::config::{
    Config, DuplicationConfig, RegressionLimits, Severity, SqlConfig, Thresholds,
    VulnerabilityConfig, load_from, parse_str,
};

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const FULL: &str = r#"
[analysis]
exclude = ["generated/**", "vendor/**"]
[metrics]
cyclomatic_profile = "default"
cognitive_profile = "default"
[thresholds.function]
cognitive = 15
cyclomatic = 10
crap = 30.0
max_nesting = 4
"#;

#[test]
fn parses_full_config() {
    let config = parse_str(FULL).unwrap();
    assert_eq!(
        config,
        Config {
            analysis_excludes: vec!["generated/**".to_string(), "vendor/**".to_string(),],
            cyclomatic_profile: "default".to_string(),
            cognitive_profile: "default".to_string(),
            thresholds: Thresholds {
                cognitive: Some(15),
                cyclomatic: Some(10),
                crap: Some(30.0),
                max_nesting: Some(4),
            },
            regressions: RegressionLimits::default(),
            duplication: DuplicationConfig::default(),
            architecture_rules: Vec::new(),
            vulnerabilities: VulnerabilityConfig::default(),
            sql: SqlConfig::default(),
            index: None,
        }
    );
    config.validate().unwrap();
}

const DUPLICATION: &str = r#"
[duplication]
min_tokens = 80
min_lines = 6
exclude = ["generated/**"]
"#;

const ARCHITECTURE: &str = r#"
[[architecture.rules]]
name = "domain-no-ui"
source = "src/domain/**"
deny = ["src/ui/**", "src/widgets/**"]
severity = "error"
[[architecture.rules]]
name = "api-no-db"
source = "src/api/**"
deny = ["src/db/**"]
severity = "warning"
"#;

#[test]
fn duplication_defaults_and_parsing() {
    let default = DuplicationConfig::default();
    assert_eq!(default.min_tokens, 100);
    assert_eq!(default.min_lines, 10);
    assert!(default.excludes.is_empty());

    let parsed = parse_str(DUPLICATION).unwrap();
    assert_eq!(parsed.duplication.min_tokens, 80);
    assert_eq!(parsed.duplication.min_lines, 6);
    assert_eq!(parsed.duplication.excludes, ["generated/**"]);
}

#[test]
fn duplication_rejects_unknown_keys_and_non_positive_minimums() {
    for doc in [
        "[duplication]\nmin_tokens = 0\n",
        "[duplication]\nmin_lines = 0\n",
        "[duplication]\nmin_tokens = -1\n",
        "[duplication]\nbogus = 1\n",
        "[duplication]\nexclude = \"not-an-array\"\n",
        "[duplication]\nexclude = [1]\n",
    ] {
        assert!(parse_str(doc).is_err(), "accepted: {doc:?}");
    }
}

#[test]
fn architecture_rules_preserve_order_and_severity() {
    let parsed = parse_str(ARCHITECTURE).unwrap();
    assert_eq!(parsed.architecture_rules.len(), 2);
    let first = &parsed.architecture_rules[0];
    assert_eq!(first.name, "domain-no-ui");
    assert_eq!(first.source, "src/domain/**");
    assert_eq!(first.deny, ["src/ui/**", "src/widgets/**"]);
    assert_eq!(first.severity, Severity::Error);
    assert_eq!(parsed.architecture_rules[1].severity, Severity::Warning);
}

#[test]
fn architecture_rejects_invalid_rules() {
    for doc in [
        // Duplicate names.
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\ndeny = [\"x/**\"]\nseverity = \"info\"\n[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\ndeny = [\"y/**\"]\nseverity = \"info\"\n",
        // Empty name.
        "[[architecture.rules]]\nname = \"\"\nsource = \"src/**\"\ndeny = [\"x/**\"]\nseverity = \"info\"\n",
        // Missing fields.
        "[[architecture.rules]]\nname = \"a\"\ndeny = [\"x/**\"]\nseverity = \"info\"\n",
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\nseverity = \"info\"\n",
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\ndeny = [\"x/**\"]\n",
        // Invalid severity.
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\ndeny = [\"x/**\"]\nseverity = \"fatal\"\n",
        // Empty deny list or non-string entries.
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\ndeny = []\nseverity = \"info\"\n",
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\ndeny = [1]\nseverity = \"info\"\n",
        // Rejected glob forms.
        "[[architecture.rules]]\nname = \"a\"\nsource = \"!src/**\"\ndeny = [\"x/**\"]\nseverity = \"info\"\n",
        "[[architecture.rules]]\nname = \"a\"\nsource = \"/src/**\"\ndeny = [\"x/**\"]\nseverity = \"info\"\n",
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**/\"\ndeny = [\"x/**\"]\nseverity = \"info\"\n",
        "[[architecture.rules]]\nname = \"a\"\nsource = \"../src/**\"\ndeny = [\"x/**\"]\nseverity = \"info\"\n",
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\ndeny = [\"C:\\\\x/**\"]\nseverity = \"info\"\n",
        "[[architecture.rules]]\nname = \"a\"\nsource = \"src/**\"\ndeny = [\"x/**\"]\nextra = 1\nseverity = \"info\"\n",
    ] {
        assert!(parse_str(doc).is_err(), "accepted: {doc:?}");
    }
}

#[test]
fn architecture_rejects_unknown_keys_in_section() {
    assert!(parse_str("[architecture]\nbogus = []\n").is_err());
}

#[test]
fn rejects_config_size_and_depth_limits() {
    let root = temporary_directory();
    let oversized = format!("# {}\n", "x".repeat(1 << 20));
    std::fs::write(root.join("leadline.toml"), oversized).unwrap();
    assert!(load_from(&root).is_err());

    let mut nested = String::from("1");
    for _ in 0..20 {
        nested = format!("{{ inner = {nested} }}");
    }
    assert!(parse_str(&format!("value = {nested}\n")).is_err());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn parses_minimal_and_empty_configs() {
    let empty = parse_str("").unwrap();
    assert!(empty.analysis_excludes.is_empty());
    assert_eq!(empty.cyclomatic_profile, "default");
    assert_eq!(empty.cognitive_profile, "default");
    assert_eq!(empty.thresholds, Thresholds::default());

    let partial = parse_str("[thresholds.function]\ncognitive = 15\n").unwrap();
    assert_eq!(partial.thresholds.cognitive, Some(15));
    assert_eq!(partial.thresholds.cyclomatic, None);
    assert_eq!(partial.thresholds.crap, None);
    assert_eq!(partial.thresholds.max_nesting, None);
}

#[test]
fn rejects_unknown_keys_and_sections() {
    for doc in [
        "[analysis]\nexec = \"evil\"\n",
        "[analysis]\ninclude = [\"src/**\"]\n",
        "[metrics]\nunknown = \"x\"\n",
        "[thresholds.function]\nbogus = 1\n",
        "[coverage]\nfoo = 1\n",
        "cognitive = 15\n",
    ] {
        assert!(parse_str(doc).is_err(), "accepted: {doc:?}");
    }
}

#[test]
fn rejects_negative_thresholds() {
    for doc in [
        "[thresholds.function]\ncognitive = -1\n",
        "[thresholds.function]\ncyclomatic = -10\n",
        "[thresholds.function]\nmax_nesting = -4\n",
        "[thresholds.function]\ncrap = -0.5\n",
    ] {
        assert!(parse_str(doc).is_err(), "accepted: {doc:?}");
    }
}

#[test]
fn rejects_non_default_profiles() {
    let doc = "[metrics]\ncyclomatic_profile = \"custom\"\n";
    assert!(parse_str(doc).is_err());
}

#[test]
fn error_converts_into_crate_error() {
    let error = parse_str("[bogus]\n").unwrap_err();
    assert!(!error.to_string().is_empty());
    let boxed: leadline::Error = error.into();
    assert!(!boxed.to_string().is_empty());
}

#[test]
fn error_message_mentions_offending_key() {
    let error = parse_str("[analysis]\nexec = \"evil\"\n").unwrap_err();
    assert!(!error.message().is_empty());
    assert!(
        error.message().contains("exec"),
        "unexpected message: {}",
        error.message()
    );
}

#[test]
fn load_from_returns_none_without_file() {
    let root = temporary_directory();
    assert!(load_from(&root).unwrap().is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn load_from_reads_and_validates_temp_dir_file() {
    let root = temporary_directory();
    std::fs::write(root.join("leadline.toml"), FULL).unwrap();
    let loaded = load_from(&root).unwrap().expect("config present");
    assert_eq!(
        loaded.analysis_excludes,
        vec!["generated/**".to_string(), "vendor/**".to_string()]
    );
    assert_eq!(loaded.thresholds.crap, Some(30.0));

    std::fs::write(
        root.join("leadline.toml"),
        "[thresholds.function]\ncrap = -1.0\n",
    )
    .unwrap();
    assert!(load_from(&root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn parses_regression_limits_with_zero_defaults() {
    let config = parse_str("[regressions]\ncognitive = 2\ncrap = 1.5\n").unwrap();
    assert_eq!(config.regressions.cognitive, 2);
    assert_eq!(config.regressions.cyclomatic, 0);
    assert_eq!(config.regressions.crap, 1.5);
    assert_eq!(config.regressions.max_nesting, 0);
}

#[test]
fn rejects_negative_regression_limits() {
    for key in ["cognitive", "cyclomatic", "max_nesting"] {
        assert!(
            parse_str(&format!("[regressions]\n{key} = -1\n")).is_err(),
            "accepted negative {key}"
        );
    }
    assert!(parse_str("[regressions]\ncrap = -0.5\n").is_err());
}

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-config-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn vulnerabilities_minimum_severity_parses() {
    let config = parse_str(
        "[vulnerabilities]
minimum_severity = 'high'
",
    )
    .unwrap();
    assert_eq!(
        config.vulnerabilities,
        VulnerabilityConfig {
            minimum_severity: Some(leadline::security::SecuritySeverity::High),
        }
    );
    assert!(
        parse_str(
            "[vulnerabilities]
minimum_severity = 'unknown'
"
        )
        .is_err()
    );
    assert!(
        parse_str(
            "[vulnerabilities]
minimum_severity = 'bogus'
"
        )
        .is_err()
    );
    assert!(
        parse_str(
            "[vulnerabilities]
minimum_severity = 3
"
        )
        .is_err()
    );
    assert!(
        parse_str(
            "[vulnerabilities]
bananas = true
"
        )
        .is_err()
    );
}

#[test]
fn sql_section_parses_threshold_and_roots() {
    let config =
        parse_str("[sql]\nlarge_offset = 500\nmigration_roots = [\"migrations\", \"db/schema\"]\n")
            .unwrap();
    assert_eq!(config.sql.large_offset, 500);
    assert_eq!(config.sql.migration_roots, vec!["migrations", "db/schema"]);
    // Dotted and empty components normalize away so roots match display paths.
    let dotted =
        parse_str("[sql]\nmigration_roots = [\"./migrations\", \"db/./schema\", \"a//b\"]\n")
            .unwrap();
    assert_eq!(
        dotted.sql.migration_roots,
        vec!["migrations", "db/schema", "a/b"]
    );
    let defaults = parse_str("").unwrap();
    assert_eq!(defaults.sql.large_offset, 1000);
    assert!(defaults.sql.migration_roots.is_empty());
}

#[test]
fn sql_section_rejects_bad_values() {
    for body in [
        "[sql]\nlarge_offset = 0\n",
        "[sql]\nlarge_offset = -5\n",
        "[sql]\nlarge_offset = 'far'\n",
        "[sql]\nbananas = true\n",
        "[sql]\nmigration_roots = ['/abs']\n",
        "[sql]\nmigration_roots = ['../escape']\n",
        "[sql]\nmigration_roots = ['.']\n",
        "[sql]\nmigration_roots = ['./']\n",
        // Drive prefixes become absolute on Windows.
        "[sql]\nmigration_roots = ['C:/migrations']\n",
        "[sql]\nmigration_roots = ['a', 'a/']\n",
        "[sql]\nmigration_roots = 'migrations'\n",
        "[sql]\nmigration_roots = [\"a\u{1}b\"]\n",
    ] {
        assert!(parse_str(body).is_err(), "{body:?}");
    }
}

#[test]
fn config_fingerprint_tracks_file_bytes() {
    let dir = std::env::temp_dir().join(format!(
        "leadline-config-fingerprint-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    assert_eq!(leadline::config::fingerprint(&dir), "none");

    let config = dir.join("leadline.toml");
    std::fs::write(&config, "[thresholds.function]\ncognitive = 15\n").unwrap();
    let first = leadline::config::fingerprint(&dir);
    assert_ne!(first, "none");

    std::fs::write(&config, "[thresholds.function]\ncognitive = 20\n").unwrap();
    assert_ne!(first, leadline::config::fingerprint(&dir));

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn index_section_defaults_to_dot_leadline() {
    let config = leadline::config::parse_str("[index]\n").unwrap();
    assert_eq!(config.index.unwrap().path, ".leadline");
}

#[test]
fn index_path_must_stay_inside_the_repository() {
    for text in ["[index]\npath = \"/tmp/x\"\n", "[index]\npath = \"../x\"\n"] {
        assert!(
            leadline::config::parse_str(text).is_err(),
            "{text} must be rejected"
        );
    }
}

#[test]
fn unknown_index_key_is_rejected() {
    assert!(leadline::config::parse_str("[index]\ncache = true\n").is_err());
}
