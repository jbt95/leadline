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
//! [quality_gate]
//! name = "team-default"
//! maintainability_rating = "B"
//! security_rating = "A"
//! effort_minutes = 600
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
    pub vulnerabilities: VulnerabilityConfig,
    pub sql: SqlConfig,
    pub unused: UnusedConfig,
    pub index: Option<IndexConfig>,
    pub quality_gate: Option<QualityGateConfig>,
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
            min_tokens: 100,
            min_lines: 10,
            excludes: Vec::new(),
        }
    }
}

/// Vulnerability gate settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VulnerabilityConfig {
    pub minimum_severity: Option<crate::security::SecuritySeverity>,
}

/// Static PostgreSQL risk settings.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlConfig {
    pub large_offset: u64,
    pub migration_roots: Vec<String>,
}

impl Default for SqlConfig {
    fn default() -> Self {
        Self {
            large_offset: 1000,
            migration_roots: Vec::new(),
        }
    }
}

/// Unused-code detection settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnusedConfig {
    /// Entry-point patterns, gitignore-style and analysis-root-relative.
    pub entries: Vec<String>,
    /// Whether test files are analyzed as ordinary candidates.
    pub include_tests: bool,
}

/// Analysis index settings; `[index]` presence enables the warm path.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexConfig {
    pub path: String,
}

/// A–E quality rating, `A` best; derived order is the gate's compare order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Rating {
    A,
    B,
    C,
    D,
    E,
}

impl Rating {
    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_uppercase().as_str() {
            "A" => Some(Rating::A),
            "B" => Some(Rating::B),
            "C" => Some(Rating::C),
            "D" => Some(Rating::D),
            "E" => Some(Rating::E),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Rating::A => "A",
            Rating::B => "B",
            Rating::C => "C",
            Rating::D => "D",
            Rating::E => "E",
        }
    }
}

/// Condition names a `[quality_gate]` may declare, in report order.
pub const QUALITY_GATE_CONDITIONS: [&str; 3] = [
    "maintainability_rating",
    "security_rating",
    "effort_minutes",
];

/// A named quality gate: the conditions one `check` run must satisfy.
#[derive(Clone, Debug, PartialEq)]
pub struct QualityGateConfig {
    pub name: String,
    pub maintainability_rating: Option<Rating>,
    pub security_rating: Option<Rating>,
    pub effort_minutes: Option<u32>,
}

impl QualityGateConfig {
    /// The condition names this gate declares, in report order.
    pub fn conditions(&self) -> impl Iterator<Item = &'static str> + '_ {
        QUALITY_GATE_CONDITIONS
            .into_iter()
            .filter(move |name| match *name {
                "maintainability_rating" => self.maintainability_rating.is_some(),
                "security_rating" => self.security_rating.is_some(),
                "effort_minutes" => self.effort_minutes.is_some(),
                _ => false,
            })
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
            vulnerabilities: VulnerabilityConfig::default(),
            sql: SqlConfig::default(),
            unused: UnusedConfig::default(),
            index: None,
            quality_gate: None,
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
        for pattern in &self.unused.entries {
            validate_analysis_pattern("unused entry", pattern)?;
        }
        if let Some(gate) = &self.quality_gate {
            validate_quality_gate_config(gate)?;
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

/// A gate needs a stable name and at least one condition to evaluate.
fn validate_quality_gate_config(gate: &QualityGateConfig) -> Result<(), ConfigError> {
    if gate.name.trim().is_empty() {
        return Err(ConfigError::new(
            "quality gate name must not be empty".to_string(),
        ));
    }
    if gate.conditions().next().is_none() {
        return Err(ConfigError::new(
            "[quality_gate] needs at least one of maintainability_rating, security_rating, or effort_minutes"
                .to_string(),
        ));
    }
    Ok(())
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
    if has_drive_prefix(pattern) {
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

/// Fingerprint of the repository's configuration file bytes.
///
/// The analysis index reuses entries only when this value is unchanged.
/// Missing configuration is a stable `"none"`; unreadable or oversized
/// configuration also falls back to `"none"`, which over-invalidates the index
/// instead of serving metrics derived from different rules.
pub fn fingerprint(dir: &Path) -> String {
    let path = dir.join(CONFIG_FILE);
    let Ok(metadata) = std::fs::metadata(&path) else {
        return "none".to_owned();
    };
    if metadata.len() > CONFIG_BYTES_LIMIT {
        return "none".to_owned();
    }
    match std::fs::read(&path) {
        Ok(bytes) => blake3::hash(&bytes).to_hex().to_string(),
        Err(_) => "none".to_owned(),
    }
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
            "unused" => read_unused(&mut config, value)?,
            "sql" => read_sql(&mut config, value)?,
            "quality_gate" => config.quality_gate = Some(read_quality_gate(value)?),
            "index" => config.index = read_index(value)?,
            "vulnerabilities" => read_vulnerabilities(&mut config, value)?,
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

/// One `[section]` table; every key inside is validated by its reader.
fn section_table<'a>(section: &str, value: &'a Value) -> Result<&'a toml::Table, ConfigError> {
    value
        .as_table()
        .ok_or_else(|| ConfigError::new(format!("expected table for [{section}]")))
}

/// Rejects an unrecognized key inside `[section]`.
fn unknown_key(section: &str, key: &str) -> ConfigError {
    ConfigError::new(format!("unknown key `{key}` in [{section}]"))
}

/// One `[section]` array of plain strings.
fn string_array(key: &str, value: &Value) -> Result<Vec<String>, ConfigError> {
    let array = value.as_array().ok_or_else(|| {
        ConfigError::new(format!(
            "expected array for `{key}`, got {}",
            value.type_str()
        ))
    })?;
    array
        .iter()
        .map(|entry| {
            entry.as_str().map(str::to_owned).ok_or_else(|| {
                ConfigError::new(format!(
                    "expected string in `{key}` array, got {}",
                    entry.type_str()
                ))
            })
        })
        .collect()
}

/// One quoted string field, named `key` in the error message.
fn read_string(key: &str, value: &Value) -> Result<String, ConfigError> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        ConfigError::new(format!(
            "expected quoted string for `{key}`, got {}",
            value.type_str()
        ))
    })
}

/// A non-negative finite score. `label` names the setting, so the threshold
/// and regression messages stay distinct while sharing one parser.
fn read_score(label: &str, value: &Value) -> Result<f64, ConfigError> {
    let score = match value {
        Value::Integer(n) => *n as f64,
        Value::Float(n) => *n,
        _ => {
            return Err(ConfigError::new(format!(
                "{label} must be >= 0, got `{value}`"
            )));
        }
    };
    if score.is_finite() && score >= 0.0 {
        Ok(score)
    } else {
        Err(ConfigError::new(format!(
            "{label} must be >= 0, got `{value}`"
        )))
    }
}

/// A non-negative `u32` count, `label` naming the setting as [`read_score`] does.
fn read_limit(label: &str, value: &Value) -> Result<u32, ConfigError> {
    match value {
        Value::Integer(n) if (0..=i64::from(u32::MAX)).contains(n) => Ok(*n as u32),
        _ => Err(ConfigError::new(format!(
            "{label} must be >= 0, got `{value}`"
        ))),
    }
}

fn read_analysis(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = section_table("analysis", value)?;
    for (key, item) in table {
        match key.as_str() {
            "exclude" => config.analysis_excludes = string_array(key, item)?,
            _ => return Err(unknown_key("analysis", key)),
        }
    }
    Ok(())
}

fn read_metrics(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = section_table("metrics", value)?;
    for (key, item) in table {
        match key.as_str() {
            "cyclomatic_profile" | "cognitive_profile" => {
                let profile = read_string(key, item)?;
                if key == "cyclomatic_profile" {
                    config.cyclomatic_profile = profile;
                } else {
                    config.cognitive_profile = profile;
                }
            }
            _ => return Err(unknown_key("metrics", key)),
        }
    }
    Ok(())
}

fn read_vulnerabilities(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = section_table("vulnerabilities", value)?;
    for (key, item) in table {
        match key.as_str() {
            "minimum_severity" => {
                let raw = read_string(key, item)?;
                let severity = crate::security::parse_gate_severity(&raw).ok_or_else(|| {
                    ConfigError::new(format!(
                        "unknown minimum_severity '{raw}': expected 'low', 'medium', 'high', or 'critical'"
                    ))
                })?;
                config.vulnerabilities.minimum_severity = Some(severity);
            }
            _ => return Err(unknown_key("vulnerabilities", key)),
        }
    }
    Ok(())
}

fn read_sql(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = section_table("sql", value)?;
    for (key, item) in table {
        match key.as_str() {
            "large_offset" => match item {
                Value::Integer(n) if *n >= 1 => config.sql.large_offset = *n as u64,
                _ => {
                    return Err(ConfigError::new(format!(
                        "sql `large_offset` must be an integer >= 1, got `{item}`"
                    )));
                }
            },
            "migration_roots" => {
                let array = item.as_array().ok_or_else(|| {
                    ConfigError::new(format!(
                        "expected array for `{key}`, got {}",
                        item.type_str()
                    ))
                })?;
                let mut roots = Vec::with_capacity(array.len());
                let mut seen = std::collections::BTreeSet::new();
                for entry in array {
                    let root = normalize_migration_root(entry.as_str().ok_or_else(|| {
                        ConfigError::new(format!(
                            "expected string in `{key}` array, got {}",
                            entry.type_str()
                        ))
                    })?)?;
                    if !seen.insert(root.clone()) {
                        return Err(ConfigError::new(format!(
                            "duplicate migration root `{root}`"
                        )));
                    }
                    roots.push(root);
                }
                config.sql.migration_roots = roots;
            }
            _ => return Err(unknown_key("sql", key)),
        }
    }
    Ok(())
}

/// True for Windows drive prefixes (`C:`, `C:/x`): neither absolute nor
/// parent-relative, so they would escape the analysis root on Windows.
fn has_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

/// Normalize one migration root: relative, `/`-separated, no escapes.
///
/// Shared by `[sql]` parsing and the `sql` command's `--migration-root` flag.
pub fn normalize_migration_root(raw: &str) -> Result<String, ConfigError> {
    let reject =
        |reason: &str| ConfigError::new(format!("invalid migration root {raw:?}: {reason}"));
    if raw.is_empty() {
        return Err(reject("empty"));
    }
    if raw.chars().any(char::is_control) {
        return Err(reject("control character"));
    }
    if raw.starts_with('/') || raw.starts_with('\\') || raw.contains('\\') {
        return Err(reject("must be relative with `/` separators"));
    }
    if has_drive_prefix(raw) {
        return Err(reject("drive prefix"));
    }
    // `.` and empty components are dropped: `./migrations` and `db/./schema`
    // must match the normalized display paths sql analysis builds, and a
    // surviving leading `.` would never match any file.
    let mut parts = Vec::new();
    for part in raw.trim_end_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(reject("may not escape the analysis root")),
            name => parts.push(name),
        }
    }
    if parts.is_empty() {
        return Err(reject("empty"));
    }
    Ok(parts.join("/"))
}

/// One `[quality_gate]` table: a name plus at least one condition.
fn read_quality_gate(value: &Value) -> Result<QualityGateConfig, ConfigError> {
    let table = section_table("quality_gate", value)?;
    let mut name = None;
    let mut maintainability = None;
    let mut security = None;
    let mut effort = None;
    for (key, item) in table {
        match key.as_str() {
            "name" => name = Some(read_string(key, item)?),
            "maintainability_rating" | "security_rating" => {
                let raw = read_string(key, item)?;
                let rating = Rating::parse(&raw).ok_or_else(|| {
                    ConfigError::new(format!(
                        "quality gate `{key}` must be A, B, C, D, or E, got {raw:?}"
                    ))
                })?;
                if key == "maintainability_rating" {
                    maintainability = Some(rating);
                } else {
                    security = Some(rating);
                }
            }
            "effort_minutes" => {
                effort = Some(read_limit("quality gate `effort_minutes`", item)?);
            }
            _ => return Err(unknown_key("quality_gate", key)),
        }
    }
    Ok(QualityGateConfig {
        name: name.unwrap_or_else(|| "default".to_owned()),
        maintainability_rating: maintainability,
        security_rating: security,
        effort_minutes: effort,
    })
}

fn read_index(value: &Value) -> Result<Option<IndexConfig>, ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ConfigError::new("[index] must be a table"))?;
    let mut path = None;
    for (key, value) in table {
        match key.as_str() {
            "path" => {
                let raw = value
                    .as_str()
                    .ok_or_else(|| ConfigError::new("[index].path must be a string"))?;
                path = Some(normalize_index_path(raw)?);
            }
            other => return Err(ConfigError::new(format!("unknown [index] key '{other}'"))),
        }
    }
    Ok(Some(IndexConfig {
        path: path.unwrap_or_else(|| crate::index::DEFAULT_INDEX_DIR.to_owned()),
    }))
}

/// Index paths are analysis-root-relative: no absolute paths or drive
/// prefixes, no parent escapes, no empty value.
fn normalize_index_path(raw: &str) -> Result<String, ConfigError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::new("[index].path must not be empty"));
    }
    // Rooted and backslash forms are rejected with string checks, exactly as
    // `normalize_migration_root` does: on Windows `Path::is_absolute` is
    // false for `/x`, which still escapes the analysis root.
    if trimmed.starts_with('/') || trimmed.contains('\\') {
        return Err(ConfigError::new(format!(
            "[index].path must be a relative path: '{raw}'"
        )));
    }
    // Windows drive-relative forms ("C:", "C:x") are neither absolute nor
    // parent-relative, so they would escape the analysis root on Windows.
    if has_drive_prefix(trimmed) {
        return Err(ConfigError::new(format!(
            "[index].path must not start with a drive prefix: '{raw}'"
        )));
    }
    if std::path::Path::new(trimmed)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ConfigError::new(format!(
            "[index].path must not contain '..': '{raw}'"
        )));
    }
    Ok(trimmed.to_owned())
}

fn read_regressions(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = section_table("regressions", value)?;
    for (key, item) in table {
        let label = format!("regression `{key}` delta");
        match key.as_str() {
            "cognitive" => config.regressions.cognitive = read_limit(&label, item)?,
            "cyclomatic" => config.regressions.cyclomatic = read_limit(&label, item)?,
            "max_nesting" => config.regressions.max_nesting = read_limit(&label, item)?,
            "crap" => config.regressions.crap = read_score(&label, item)?,
            _ => return Err(unknown_key("regressions", key)),
        }
    }
    Ok(())
}

fn read_thresholds(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = section_table("thresholds", value)?;
    for (key, item) in table {
        match key.as_str() {
            "function" => {
                let inner = section_table("thresholds.function", item)?;
                for (name, limit) in inner {
                    match name.as_str() {
                        "cognitive" => {
                            config.thresholds.cognitive =
                                Some(read_limit(&format!("threshold `{name}`"), limit)?);
                        }
                        "cyclomatic" => {
                            config.thresholds.cyclomatic =
                                Some(read_limit(&format!("threshold `{name}`"), limit)?);
                        }
                        "max_nesting" => {
                            config.thresholds.max_nesting =
                                Some(read_limit(&format!("threshold `{name}`"), limit)?);
                        }
                        "crap" => {
                            config.thresholds.crap = Some(read_score("threshold `crap`", limit)?);
                        }
                        _ => {
                            return Err(unknown_key("thresholds.function", name));
                        }
                    }
                }
            }
            _ => return Err(unknown_key("thresholds", key)),
        }
    }
    Ok(())
}

fn read_duplication(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = section_table("duplication", value)?;
    for (key, item) in table {
        match key.as_str() {
            "min_tokens" => {
                config.duplication.min_tokens = read_positive_usize(key, item)?;
            }
            "min_lines" => {
                config.duplication.min_lines = read_positive_usize(key, item)?;
            }
            "exclude" => config.duplication.excludes = string_array(key, item)?,
            _ => return Err(unknown_key("duplication", key)),
        }
    }
    Ok(())
}

fn read_unused(config: &mut Config, value: &Value) -> Result<(), ConfigError> {
    let table = section_table("unused", value)?;
    for (key, item) in table {
        match key.as_str() {
            "entries" => config.unused.entries = string_array(key, item)?,
            "include_tests" => {
                config.unused.include_tests = item.as_bool().ok_or_else(|| {
                    ConfigError::new(format!(
                        "expected bool for `{key}`, got {}",
                        item.type_str()
                    ))
                })?;
            }
            _ => return Err(unknown_key("unused", key)),
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
    let table = section_table("architecture", value)?;
    for (key, item) in table {
        match key.as_str() {
            "rules" => {
                let array = item.as_array().ok_or_else(|| {
                    ConfigError::new(format!(
                        "expected array for `{key}`, got {}",
                        item.type_str()
                    ))
                })?;
                config.architecture_rules = array
                    .iter()
                    .map(read_architecture_rule)
                    .collect::<Result<_, _>>()?;
            }
            _ => return Err(unknown_key("architecture", key)),
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
            "name" => name = Some(read_string(key, item)?),
            "source" => source = Some(read_string(key, item)?),
            "deny" => deny = Some(string_array(key, item)?),
            "severity" => {
                let text = read_string(key, item)?;
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
