// Tests for the unwired config module: include the file directly so this
// suite passes before the later wiring task adds `pub mod config` to lib.rs.
#[path = "../src/config.rs"]
mod config;

use config::{Config, load_from, parse_str};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const FULL: &str = r#"
[analysis]
exclude = ["generated/**", "vendor/**"]
[metrics]
cyclomatic_profile = "default-v1"
cognitive_profile = "default-v1"
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
            cyclomatic_profile: "default-v1".to_string(),
            cognitive_profile: "default-v1".to_string(),
            thresholds: config::Thresholds {
                cognitive: Some(15),
                cyclomatic: Some(10),
                crap: Some(30.0),
                max_nesting: Some(4),
            },
            regressions: config::RegressionLimits::default(),
        }
    );
    config.validate().unwrap();
}

#[test]
fn parses_minimal_and_empty_configs() {
    let empty = parse_str("").unwrap();
    assert!(empty.analysis_excludes.is_empty());
    assert_eq!(empty.cyclomatic_profile, "default-v1");
    assert_eq!(empty.cognitive_profile, "default-v1");
    assert_eq!(empty.thresholds, config::Thresholds::default());

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
