//! `leadline.toml` project configuration.
//!
//! Parsed with the `toml` crate, then extracted and validated. Accepted schema:
//!
//! ```toml
//! [analysis]
//! exclude = ["generated/**", "vendor/**"]
//! [metrics]
//! cyclomatic_profile = "default"
//! cognitive_profile = "default"
//! [thresholds.function]
//! cognitive = 15
//! cyclomatic = 10
//! crap = 30.0
//! max_nesting = 4
//! [regressions]
//! cognitive = 1
//! cyclomatic = 0
//! crap = 0.0
//! max_nesting = 0
//! ```
//! Every threshold is optional. Unknown sections or keys are errors, never
//! ignored, so no `include`/`exec` style key can ever slip through: config
//! never executes commands by construction.

use std::path::Path;

use toml::Value;

/// The only metric profile accepted for now.
pub const DEFAULT_PROFILE: &str = "default";

const CONFIG_FILE: &str = "leadline.toml";
/// `leadline.toml` size limit, enforced before parsing.
pub const CONFIG_BYTES_LIMIT: u64 = 1 << 20;
/// Maximum accepted TOML nesting depth.
const CONFIG_DEPTH_LIMIT: usize = 16;

/// Project configuration loaded from `leadline.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub analysis_excludes: Vec<String>,
    pub cyclomatic_profile: String,
    pub cognitive_profile: String,
    pub thresholds: Thresholds,
    pub regressions: RegressionLimits,
    pub duplication: DuplicationConfig,
    pub architecture_rules: Vec<ArchitectureRule>,
}

/// Duplication detection settings.
#[derive(Clone, Debug, PartialEq)]
pub struct DuplicationConfig {
    pub min_tokens: usize,
    pub min_lines: usize,
    pub excludes: Vec<String>,
}

impl Default for DuplicationConfig {
    fn default() -> Self {
        Self {
            min_tokens: 50,
            min_lines: 5,
            excludes: Vec::new(),
        }
    }
}

/// Severity of an architecture rule violation, in ascending order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "info" => Some(Severity::Info),
            "warning" => Some(Severity::Warning),
            "error" => Some(Severity::Error),
            _ => None,
        }
    }
}

/// One dependency architecture rule: `source` files must not import `deny`.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchitectureRule {
    pub name: String,
    pub source: String,
    pub deny: Vec<String>,
    pub severity: Severity,
}

/// Allowed positive metric deltas for regression-only gates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegressionLimits {
    pub cognitive: u32,
    pub cyclomatic: u32,
    pub crap: f64,
    pub max_nesting: u32,
}

/// Optional per-function thresholds; `None` means "no limit".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Thresholds {
    pub cognitive: Option<u32>,
    pub cyclomatic: Option<u32>,
    pub crap: Option<f64>,
    pub max_nesting: Option<u32>,
}

/// Configuration failure. Implements [`std::error::Error`], so the std
/// blanket `From` converts it into the crate's `crate::Error` box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Default for Config {
    fn default() -> Self {
        Self {
            analysis_excludes: Vec::new(),
            cyclomatic_profile: DEFAULT_PROFILE.to_string(),
            cognitive_profile: DEFAULT_PROFILE.to_string(),
            thresholds: Thresholds::default(),
            regressions: RegressionLimits::default(),
            duplication: DuplicationConfig::default(),
            architecture_rules: Vec::new(),
        }
    }
}

impl Config {
    /// Rejects unknown profiles and negative thresholds or deltas.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (name, profile) in [
            ("cyclomatic_profile", &self.cyclomatic_profile),
            ("cognitive_profile", &self.cognitive_profile),
        ] {
            if profile != DEFAULT_PROFILE {
                return Err(ConfigError::new(format!(
                    "unknown {name} {profile:?}: expected {DEFAULT_PROFILE:?}"
                )));
            }
        }
        if let Some(crap) = self.thresholds.crap
            && (!crap.is_finite() || crap < 0.0)
        {
            return Err(ConfigError::new(format!(
                "threshold crap must be >= 0, got {crap}"
            )));
        }
        if !self.regressions.crap.is_finite() || self.regressions.crap < 0.0 {
            return Err(ConfigError::new(format!(
                "regression crap delta must be >= 0, got {}",
                self.regressions.crap
            )));
        }
        if self.duplication.min_tokens == 0 {
            return Err(ConfigError::new(
                "duplication min_tokens must be >= 1".to_string(),
            ));
        }
        if self.duplication.min_lines == 0 {
            return Err(ConfigError::new(
                "duplication min_lines must be >= 1".to_string(),
            ));
        }
        for pattern in &self.duplication.excludes {
            validate_analysis_pattern("duplication exclude", pattern)?;
        }
        let mut names = std::collections::BTreeSet::new();
        for rule in &self.architecture_rules {
            if rule.name.trim().is_empty() {
                return Err(ConfigError::new(
                    "architecture rule name must not be empty".to_string(),
                ));
            }
            if !names.insert(rule.name.as_str()) {
                return Err(ConfigError::new(format!(
                    "duplicate architecture rule name {:?}",
                    rule.name
                )));
            }
            if rule.source.trim().is_empty() {
                return Err(ConfigError::new(format!(
                    "architecture rule {:?} needs a source glob",
                    rule.name
                )));
            }
            if rule.deny.is_empty() {
                return Err(ConfigError::new(format!(
                    "architecture rule {:?} needs at least one deny glob",
                    rule.name
                )));
            }
            validate_rule_pattern(&rule.name, &rule.source)?;
            for pattern in &rule.deny {
                validate_rule_pattern(&rule.name, pattern)?;
            }
        }
        Ok(())
    }
}

fn validate_toml_depth(table: &toml::Table, limit: usize) -> Result<(), ConfigError> {
    fn walk(value: &Value, limit: usize) -> Result<(), ConfigError> {
        if limit == 0 {
            return Err(ConfigError::new(format!(
                "leadline.toml nesting exceeds {CONFIG_DEPTH_LIMIT} levels"
            )));
        }
        match value {
            Value::Table(table) => {
                for value in table.values() {
                    walk(value, limit - 1)?;
                }
            }
            Value::Array(items) => {
                for value in items {
                    if value.is_table() || value.is_array() {
                        walk(value, limit - 1)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    for value in table.values() {
        walk(value, limit)?;
    }
    Ok(())
}

fn validate_analysis_pattern(kind: &str, pattern: &str) -> Result<(), ConfigError> {
    if pattern.trim().is_empty() {
        return Err(ConfigError::new(format!("{kind} must not be empty")));
    }
    if pattern.chars().any(char::is_control) {
        return Err(ConfigError::new(format!(
            "{kind} {pattern:?} contains control characters"
        )));
    }
    Ok(())
}

/// Architecture globs are analysis-root-anchored and never negated or
/// absolute; parent traversal and directory-only forms are rejected.
fn validate_rule_pattern(rule: &str, pattern: &str) -> Result<(), ConfigError> {
    validate_analysis_pattern("architecture glob", pattern)?;
    let reject = |reason: &str| {
        Err(ConfigError::new(format!(
            "architecture rule {rule:?} glob {pattern:?} {reason}"
        )))
    };
    if pattern.starts_with('!') {
        return reject("may not be negated");
    }
    if pattern.starts_with('/') || pattern.starts_with('\\') {
        return reject("must be relative");
    }
    if pattern.contains('\\') {
        return reject("must use `/` separators");
    }
    if pattern.ends_with('/') {
        return reject("may not be directory-only");
    }
    let bytes = pattern.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return reject("may not use a drive prefix");
    }
    let mut depth = 0i64;
    for part in pattern.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return reject("may not escape the analysis root");
                }
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

/// Loads `leadline.toml` from `dir`. `Ok(None)` when the file is absent.
pub fn load_from(dir: &Path) -> Result<Option<Config>, ConfigError> {
    let path = dir.join(CONFIG_FILE);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ConfigError::new(format!(
                "cannot read {}: {error}",
                path.display()
            )));
        }
    };
    if metadata.len() > CONFIG_BYTES_LIMIT {
        return Err(ConfigError::new(format!(
            "{} exceeds the {} byte limit",
            path.display(),
            CONFIG_BYTES_LIMIT
        )));
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ConfigError::new(format!(
                "cannot read {}: {error}",
                path.display()
            )));
        }
    };
    parse_str(&text).map(Some)
}

/// Parses and validates a `leadline.toml` document.
pub fn parse_str(text: &str) -> Result<Config, ConfigError> {
    let table: toml::Table = text
        .parse()
        .map_err(|error| ConfigError::new(format!("invalid TOML: {error}")))?;
    validate_toml_depth(&table, CONFIG_DEPTH_LIMIT)?;
    let mut config = Config::default();
    for (section, value) in &table {
        match section.as_str() {
            "analysis" => read_analysis(&mut config, value)?,
            "metrics" => read_metrics(&mut config, value)?,
            "thresholds" => read_thresholds(&mut config, value)?,
            "regressions" => read_regressions(&mut config, value)?,
            "duplication" => read_duplication(&mut config, value)?,
            "architecture" => read_architecture(&mut config, value)?,
            _ if !value.is_table() => {
                return Err(ConfigError::new(format!(
                    "key `{section}` outside any section"
                )));
            }
            _ => {
                return Err(ConfigError::new(format!("unknown section [{section}]")));
            }
        }
    }
    config.validate()?;
    Ok(config)
}

fn read_analysis(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ConfigError::new("expected table for [analysis]".to_string()))?;
    for (key, item) in table {
        match key.as_str() {
            "exclude" => {
                let array = item.as_array().ok_or_else(|| {
                    ConfigError::new(format!(
                        "expected array for `exclude`, got {}",
                        item.type_str()
                    ))
                })?;
                let mut excludes = Vec::with_capacity(array.len());
                for entry in array {
                    match entry.as_str() {
                        Some(pattern) => excludes.push(pattern.to_string()),
                        None => {
                            return Err(ConfigError::new(format!(
                                "expected string in `exclude` array, got {}",
                                entry.type_str()
                            )));
                        }
                    }
                }
                config.analysis_excludes = excludes;
            }
            _ => {
                return Err(ConfigError::new(format!(
                    "unknown key `{key}` in [analysis]"
                )));
            }
        }
    }
    Ok(())
}

fn read_metrics(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ConfigError::new("expected table for [metrics]".to_string()))?;
    for (key, item) in table {
        match key.as_str() {
            "cyclomatic_profile" | "cognitive_profile" => {
                let profile = item.as_str().ok_or_else(|| {
                    ConfigError::new(format!(
                        "expected quoted string for `{key}`, got {}",
                        item.type_str()
                    ))
                })?;
                if key == "cyclomatic_profile" {
                    config.cyclomatic_profile = profile.to_string();
                } else {
                    config.cognitive_profile = profile.to_string();
                }
            }
            _ => {
                return Err(ConfigError::new(format!(
                    "unknown key `{key}` in [metrics]"
                )));
            }
        }
    }
    Ok(())
}
fn read_regressions(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ConfigError::new("expected table for [regressions]".to_string()))?;
    for (key, item) in table {
        match key.as_str() {
            "cognitive" => config.regressions.cognitive = read_regression_limit(key, item)?,
            "cyclomatic" => config.regressions.cyclomatic = read_regression_limit(key, item)?,
            "max_nesting" => config.regressions.max_nesting = read_regression_limit(key, item)?,
            "crap" => config.regressions.crap = read_regression_score(item)?,
            _ => {
                return Err(ConfigError::new(format!(
                    "unknown key `{key}` in [regressions]"
                )));
            }
        }
    }
    Ok(())
}

fn read_regression_limit(key: &str, value: &Value) -> Result<u32, ConfigError> {
    match value {
        Value::Integer(n) if (0..=i64::from(u32::MAX)).contains(n) => Ok(*n as u32),
        _ => Err(ConfigError::new(format!(
            "regression `{key}` delta must be >= 0, got `{value}`"
        ))),
    }
}

fn read_regression_score(value: &Value) -> Result<f64, ConfigError> {
    let score = match value {
        Value::Integer(n) => *n as f64,
        Value::Float(n) => *n,
        _ => {
            return Err(ConfigError::new(format!(
                "regression `crap` delta must be >= 0, got `{value}`"
            )));
        }
    };
    if score.is_finite() && score >= 0.0 {
        Ok(score)
    } else {
        Err(ConfigError::new(format!(
            "regression `crap` delta must be >= 0, got `{value}`"
        )))
    }
}

fn read_thresholds(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ConfigError::new("expected table for [thresholds]".to_string()))?;
    for (key, item) in table {
        match key.as_str() {
            "function" => {
                let inner = item.as_table().ok_or_else(|| {
                    ConfigError::new("expected table for [thresholds.function]".to_string())
                })?;
                for (name, limit) in inner {
                    match name.as_str() {
                        "cognitive" => {
                            config.thresholds.cognitive = Some(read_limit(name, limit)?);
                        }
                        "cyclomatic" => {
                            config.thresholds.cyclomatic = Some(read_limit(name, limit)?);
                        }
                        "max_nesting" => {
                            config.thresholds.max_nesting = Some(read_limit(name, limit)?);
                        }
                        "crap" => {
                            config.thresholds.crap = Some(read_score(limit)?);
                        }
                        _ => {
                            return Err(ConfigError::new(format!(
                                "unknown key `{name}` in [thresholds.function]"
                            )));
                        }
                    }
                }
            }
            _ => {
                return Err(ConfigError::new(format!(
                    "unknown key `{key}` in [thresholds]"
                )));
            }
        }
    }
    Ok(())
}

fn read_duplication(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ConfigError::new("expected table for [duplication]".to_string()))?;
    for (key, item) in table {
        match key.as_str() {
            "min_tokens" => {
                config.duplication.min_tokens = read_positive_usize(key, item)?;
            }
            "min_lines" => {
                config.duplication.min_lines = read_positive_usize(key, item)?;
            }
            "exclude" => {
                let array = item.as_array().ok_or_else(|| {
                    ConfigError::new(format!(
                        "expected array for `exclude`, got {}",
                        item.type_str()
                    ))
                })?;
                let mut excludes = Vec::with_capacity(array.len());
                for entry in array {
                    match entry.as_str() {
                        Some(pattern) => excludes.push(pattern.to_string()),
                        None => {
                            return Err(ConfigError::new(format!(
                                "expected string in `exclude` array, got {}",
                                entry.type_str()
                            )));
                        }
                    }
                }
                config.duplication.excludes = excludes;
            }
            _ => {
                return Err(ConfigError::new(format!(
                    "unknown key `{key}` in [duplication]"
                )));
            }
        }
    }
    Ok(())
}

fn read_positive_usize(key: &str, value: &Value) -> Result<usize, ConfigError> {
    match value {
        Value::Integer(n) if (1..=i64::from(u32::MAX)).contains(n) => Ok(*n as usize),
        _ => Err(ConfigError::new(format!(
            "duplication `{key}` must be >= 1, got `{value}`"
        ))),
    }
}

fn read_architecture(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ConfigError::new("expected table for [architecture]".to_string()))?;
    for (key, item) in table {
        match key.as_str() {
            "rules" => {
                let array = item.as_array().ok_or_else(|| {
                    ConfigError::new(format!(
                        "expected array for `rules`, got {}",
                        item.type_str()
                    ))
                })?;
                let mut rules = Vec::with_capacity(array.len());
                for entry in array {
                    rules.push(read_architecture_rule(entry)?);
                }
                config.architecture_rules = rules;
            }
            _ => {
                return Err(ConfigError::new(format!(
                    "unknown key `{key}` in [architecture]"
                )));
            }
        }
    }
    Ok(())
}

fn read_architecture_rule(value: &Value) -> Result<ArchitectureRule, ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ConfigError::new("expected table in [[architecture.rules]]".to_string()))?;
    let mut name = None;
    let mut source = None;
    let mut deny = None;
    let mut severity = None;
    for (key, item) in table {
        match key.as_str() {
            "name" => name = Some(read_rule_string(key, item)?),
            "source" => source = Some(read_rule_string(key, item)?),
            "deny" => {
                let array = item.as_array().ok_or_else(|| {
                    ConfigError::new(format!(
                        "expected array for `deny`, got {}",
                        item.type_str()
                    ))
                })?;
                let mut patterns = Vec::with_capacity(array.len());
                for entry in array {
                    match entry.as_str() {
                        Some(pattern) => patterns.push(pattern.to_string()),
                        None => {
                            return Err(ConfigError::new(format!(
                                "expected string in `deny` array, got {}",
                                entry.type_str()
                            )));
                        }
                    }
                }
                deny = Some(patterns);
            }
            "severity" => {
                let text = read_rule_string(key, item)?;
                severity = Some(Severity::parse(&text).ok_or_else(|| {
                    ConfigError::new(format!(
                        "architecture rule severity must be info, warning, or error, got {text:?}"
                    ))
                })?);
            }
            _ => {
                return Err(ConfigError::new(format!(
                    "unknown key `{key}` in [[architecture.rules]]"
                )));
            }
        }
    }
    Ok(ArchitectureRule {
        name: name.ok_or_else(|| ConfigError::new("architecture rule needs `name`".to_string()))?,
        source: source
            .ok_or_else(|| ConfigError::new("architecture rule needs `source`".to_string()))?,
        deny: deny.ok_or_else(|| ConfigError::new("architecture rule needs `deny`".to_string()))?,
        severity: severity
            .ok_or_else(|| ConfigError::new("architecture rule needs `severity`".to_string()))?,
    })
}

fn read_rule_string(key: &str, value: &Value) -> Result<String, ConfigError> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        ConfigError::new(format!(
            "expected quoted string for `{key}`, got {}",
            value.type_str()
        ))
    })
}

fn read_limit(key: &str, value: &Value) -> Result<u32, ConfigError> {
    match value {
        Value::Integer(n) if (0..=i64::from(u32::MAX)).contains(n) => Ok(*n as u32),
        _ => Err(ConfigError::new(format!(
            "threshold `{key}` must be >= 0, got `{value}`"
        ))),
    }
}

fn read_score(value: &Value) -> Result<f64, ConfigError> {
    let score = match value {
        Value::Integer(n) => *n as f64,
        Value::Float(n) => *n,
        _ => {
            return Err(ConfigError::new(format!(
                "threshold `crap` must be >= 0, got `{value}`"
            )));
        }
    };
    if score.is_finite() && score >= 0.0 {
        Ok(score)
    } else {
        Err(ConfigError::new(format!(
            "threshold `crap` must be >= 0, got `{value}`"
        )))
    }
}
