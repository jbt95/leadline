//! `leadline.toml` project configuration.
//!
//! Parsed with the `toml` crate, then extracted and validated. Accepted schema:
//!
//! ```toml
//! [analysis]
//! exclude = ["generated/**", "vendor/**"]
//! [metrics]
//! cyclomatic_profile = "default-v1"
//! cognitive_profile = "default-v1"
//! [thresholds.function]
//! cognitive = 15
//! cyclomatic = 10
//! crap = 30.0
//! max_nesting = 4
//! ```
//!
//! Every threshold is optional. Unknown sections or keys are errors, never
//! ignored, so no `include`/`exec` style key can ever slip through: config
//! never executes commands by construction.

use std::path::Path;

use toml::Value;

/// The only metric profile accepted for now.
pub const DEFAULT_PROFILE: &str = "default-v1";

const CONFIG_FILE: &str = "leadline.toml";

/// Project configuration loaded from `leadline.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub analysis_excludes: Vec<String>,
    pub cyclomatic_profile: String,
    pub cognitive_profile: String,
    pub thresholds: Thresholds,
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
        }
    }
}

impl Config {
    /// Rejects unknown profiles and negative thresholds.
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
        Ok(())
    }
}

/// Loads `leadline.toml` from `dir`. `Ok(None)` when the file is absent.
pub fn load_from(dir: &Path) -> Result<Option<Config>, ConfigError> {
    let path = dir.join(CONFIG_FILE);
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
    let mut config = Config::default();
    for (section, value) in &table {
        match section.as_str() {
            "analysis" => read_analysis(&mut config, value)?,
            "metrics" => read_metrics(&mut config, value)?,
            "thresholds" => read_thresholds(&mut config, value)?,
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
