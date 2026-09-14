use crate::core::{
    FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE, MetricContribution,
    OUTPUT_SCHEMA_VERSION,
};
use std::collections::BTreeMap;
use std::path::Path;

/// Parser backend versions baked into the cache key.
/// Bump when tree-sitter or any grammar crate changes.
pub const PARSER_VERSIONS: &str = "tree-sitter-0.27:java-0.23.5:javascript-0.25:typescript-0.23.2";

const CACHE_FILE_NAME: &str = "file-cache.json";

#[derive(Clone, Debug)]
pub struct CachedFile {
    pub key: String,
    pub functions: Vec<FunctionAnalysis>,
}

pub struct FileCache {
    entries: BTreeMap<String, CachedFile>,
    dirty: bool,
}

impl FileCache {
    pub fn open(dir: &Path) -> FileCache {
        let stored: Option<BTreeMap<String, StoredFile>> = std::fs::read(dir.join(CACHE_FILE_NAME))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let dirty = stored.is_none();
        let entries = stored
            .unwrap_or_default()
            .into_iter()
            .map(|(path, file)| {
                let functions = file
                    .functions
                    .into_iter()
                    .map(FunctionAnalysis::from)
                    .collect();
                (
                    path,
                    CachedFile {
                        key: file.key,
                        functions,
                    },
                )
            })
            .collect();
        FileCache { entries, dirty }
    }

    pub fn get(&self, path: &str, content: &[u8]) -> Option<Vec<FunctionAnalysis>> {
        let entry = self.entries.get(path)?;
        if entry.key == content_key(content) {
            Some(entry.functions.clone())
        } else {
            None
        }
    }

    pub fn put(&mut self, path: &str, content: &[u8], functions: Vec<FunctionAnalysis>) {
        self.dirty = true;
        self.entries.insert(
            path.to_owned(),
            CachedFile {
                key: content_key(content),
                functions,
            },
        );
    }

    pub fn save(&mut self, dir: &Path) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        std::fs::create_dir_all(dir)?;
        // BTreeMap iterates in key order, so the JSON is deterministic.
        let stored: BTreeMap<String, StoredFile> = self
            .entries
            .iter()
            .map(|(path, file)| {
                let functions = file.functions.iter().map(StoredFunction::from).collect();
                (
                    path.clone(),
                    StoredFile {
                        key: file.key.clone(),
                        functions,
                    },
                )
            })
            .collect();
        let json = serde_json::to_string(&stored).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(CACHE_FILE_NAME), json)?;
        self.dirty = false;
        Ok(())
    }
}

fn content_key(content: &[u8]) -> String {
    content_key_for(env!("CARGO_PKG_VERSION"), content)
}

fn content_key_for(version: &str, content: &[u8]) -> String {
    let schema_version = OUTPUT_SCHEMA_VERSION.to_le_bytes();
    let mut hasher = blake3::Hasher::new();
    for part in [
        version.as_bytes(),
        METRIC_PROFILE.as_bytes(),
        schema_version.as_slice(),
        PARSER_VERSIONS.as_bytes(),
        content,
    ] {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().to_hex().to_string()
}

// Core analysis types only derive Serialize (source_fingerprint is skipped),
// so the cache keeps its own serializable mirror and converts on load/save.

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredFile {
    key: String,
    functions: Vec<StoredFunction>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredFunction {
    name: String,
    id: String,
    kind: StoredKind,
    start_line: u32,
    end_line: u32,
    start_byte: u64,
    end_byte: u64,
    metrics: StoredMetrics,
    contributions: Vec<StoredContribution>,
    source_fingerprint: u64,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum StoredKind {
    Function,
    Method,
    Constructor,
    Lambda,
    Arrow,
    Anonymous,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredContribution {
    rule: String,
    line: u32,
    nesting: u32,
    cognitive: u32,
    cyclomatic: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredMetrics {
    loc: u32,
    logical_loc: u32,
    function_length: u32,
    parameters: u32,
    max_nesting: u32,
    cyclomatic: u32,
    cognitive: u32,
    halstead_n1: u32,
    halstead_n2: u32,
    #[serde(rename = "halstead_N1")]
    halstead_total_operators: u32,
    #[serde(rename = "halstead_N2")]
    halstead_total_operands: u32,
    halstead_vocabulary: u32,
    halstead_length: u32,
    halstead_volume: f64,
    halstead_difficulty: f64,
    halstead_effort: f64,
    maintainability_index: f64,
    coverage: Option<f64>,
    crap: Option<f64>,
}

impl From<FunctionKind> for StoredKind {
    fn from(kind: FunctionKind) -> StoredKind {
        match kind {
            FunctionKind::Function => StoredKind::Function,
            FunctionKind::Method => StoredKind::Method,
            FunctionKind::Constructor => StoredKind::Constructor,
            FunctionKind::Lambda => StoredKind::Lambda,
            FunctionKind::Arrow => StoredKind::Arrow,
            FunctionKind::Anonymous => StoredKind::Anonymous,
        }
    }
}

impl From<StoredKind> for FunctionKind {
    fn from(kind: StoredKind) -> FunctionKind {
        match kind {
            StoredKind::Function => FunctionKind::Function,
            StoredKind::Method => FunctionKind::Method,
            StoredKind::Constructor => FunctionKind::Constructor,
            StoredKind::Lambda => FunctionKind::Lambda,
            StoredKind::Arrow => FunctionKind::Arrow,
            StoredKind::Anonymous => FunctionKind::Anonymous,
        }
    }
}

impl From<&MetricContribution> for StoredContribution {
    fn from(contribution: &MetricContribution) -> StoredContribution {
        StoredContribution {
            rule: contribution.rule.clone(),
            line: contribution.line,
            nesting: contribution.nesting,
            cognitive: contribution.cognitive,
            cyclomatic: contribution.cyclomatic,
        }
    }
}

impl From<StoredContribution> for MetricContribution {
    fn from(contribution: StoredContribution) -> MetricContribution {
        MetricContribution {
            rule: contribution.rule,
            line: contribution.line,
            nesting: contribution.nesting,
            cognitive: contribution.cognitive,
            cyclomatic: contribution.cyclomatic,
        }
    }
}

impl From<&FunctionMetrics> for StoredMetrics {
    fn from(metrics: &FunctionMetrics) -> StoredMetrics {
        StoredMetrics {
            loc: metrics.loc,
            logical_loc: metrics.logical_loc,
            function_length: metrics.function_length,
            parameters: metrics.parameters,
            max_nesting: metrics.max_nesting,
            cyclomatic: metrics.cyclomatic,
            cognitive: metrics.cognitive,
            halstead_n1: metrics.halstead_n1,
            halstead_n2: metrics.halstead_n2,
            halstead_total_operators: metrics.halstead_total_operators,
            halstead_total_operands: metrics.halstead_total_operands,
            halstead_vocabulary: metrics.halstead_vocabulary,
            halstead_length: metrics.halstead_length,
            halstead_volume: metrics.halstead_volume,
            halstead_difficulty: metrics.halstead_difficulty,
            halstead_effort: metrics.halstead_effort,
            maintainability_index: metrics.maintainability_index,
            coverage: metrics.coverage,
            crap: metrics.crap,
        }
    }
}

impl From<StoredMetrics> for FunctionMetrics {
    fn from(metrics: StoredMetrics) -> FunctionMetrics {
        FunctionMetrics {
            loc: metrics.loc,
            logical_loc: metrics.logical_loc,
            function_length: metrics.function_length,
            parameters: metrics.parameters,
            max_nesting: metrics.max_nesting,
            cyclomatic: metrics.cyclomatic,
            cognitive: metrics.cognitive,
            halstead_n1: metrics.halstead_n1,
            halstead_n2: metrics.halstead_n2,
            halstead_total_operators: metrics.halstead_total_operators,
            halstead_total_operands: metrics.halstead_total_operands,
            halstead_vocabulary: metrics.halstead_vocabulary,
            halstead_length: metrics.halstead_length,
            halstead_volume: metrics.halstead_volume,
            halstead_difficulty: metrics.halstead_difficulty,
            halstead_effort: metrics.halstead_effort,
            maintainability_index: metrics.maintainability_index,
            coverage: metrics.coverage,
            crap: metrics.crap,
        }
    }
}

impl From<&FunctionAnalysis> for StoredFunction {
    fn from(function: &FunctionAnalysis) -> StoredFunction {
        StoredFunction {
            name: function.name.clone(),
            id: function.id.clone(),
            kind: StoredKind::from(function.kind),
            start_line: function.start_line,
            end_line: function.end_line,
            start_byte: function.start_byte,
            end_byte: function.end_byte,
            metrics: StoredMetrics::from(&function.metrics),
            contributions: function
                .contributions
                .iter()
                .map(StoredContribution::from)
                .collect(),
            source_fingerprint: function.source_fingerprint,
        }
    }
}

impl From<StoredFunction> for FunctionAnalysis {
    fn from(function: StoredFunction) -> FunctionAnalysis {
        FunctionAnalysis {
            name: function.name,
            id: function.id,
            kind: FunctionKind::from(function.kind),
            start_line: function.start_line,
            end_line: function.end_line,
            start_byte: function.start_byte,
            end_byte: function.end_byte,
            metrics: FunctionMetrics::from(function.metrics),
            contributions: function
                .contributions
                .into_iter()
                .map(MetricContribution::from)
                .collect(),
            source_fingerprint: function.source_fingerprint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn sample_function(name: &str) -> FunctionAnalysis {
        FunctionAnalysis {
            name: name.to_owned(),
            id: "file.ts:function:0:100".to_owned(),
            kind: FunctionKind::Function,
            start_line: 1,
            end_line: 10,
            start_byte: 0,
            end_byte: 100,
            metrics: FunctionMetrics {
                loc: 10,
                logical_loc: 5,
                function_length: 10,
                parameters: 2,
                max_nesting: 1,
                cyclomatic: 3,
                cognitive: 2,
                halstead_n1: 4,
                halstead_n2: 6,
                halstead_total_operators: 8,
                halstead_total_operands: 9,
                halstead_vocabulary: 10,
                halstead_length: 17,
                halstead_volume: 58.5,
                halstead_difficulty: 3.0,
                halstead_effort: 175.5,
                maintainability_index: 72.0,
                coverage: Some(0.8),
                crap: Some(3.2),
            },
            contributions: vec![MetricContribution {
                rule: "if".to_owned(),
                line: 3,
                nesting: 1,
                cognitive: 1,
                cyclomatic: 1,
            }],
            source_fingerprint: 12345,
        }
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "leadline-{}-{}-{}",
            name,
            std::process::id(),
            TEST_DIR_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn same_content_hits() {
        let mut cache = FileCache::open(&test_dir("missing"));
        let content = b"function foo() {}";
        cache.put("foo.ts", content, vec![sample_function("foo")]);
        assert_eq!(
            cache.get("foo.ts", content),
            Some(vec![sample_function("foo")])
        );
    }

    #[test]
    fn changed_content_misses() {
        let mut cache = FileCache::open(&test_dir("missing"));
        cache.put("foo.ts", b"function foo() {}", vec![sample_function("foo")]);
        assert_eq!(cache.get("foo.ts", b"function foo() { return 1; }"), None);
    }

    #[test]
    fn version_string_change_misses() {
        assert_ne!(
            content_key_for("0.1.0", b"function foo() {}"),
            content_key_for("0.2.0", b"function foo() {}")
        );
    }

    #[test]
    fn corrupt_file_opens_empty() {
        let dir = test_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CACHE_FILE_NAME), b"{ not valid json").unwrap();
        let cache = FileCache::open(&dir);
        assert_eq!(cache.get("foo.ts", b"function foo() {}"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unchanged_loaded_cache_is_not_rewritten() {
        let dir = test_dir("unchanged");
        let content = b"function foo() {}";
        let mut cache = FileCache::open(&dir);
        cache.put("foo.ts", content, vec![sample_function("foo")]);
        cache.save(&dir).unwrap();

        let mut loaded = FileCache::open(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        loaded.save(&dir).unwrap();

        assert!(!dir.exists());
    }

    #[test]
    fn put_marks_loaded_cache_dirty() {
        let dir = test_dir("dirty");
        let mut cache = FileCache::open(&dir);
        cache.put("foo.ts", b"foo", vec![sample_function("foo")]);
        cache.save(&dir).unwrap();

        let mut loaded = FileCache::open(&dir);
        loaded.put("bar.ts", b"bar", vec![sample_function("bar")]);
        loaded.save(&dir).unwrap();

        assert_eq!(
            FileCache::open(&dir).get("bar.ts", b"bar"),
            Some(vec![sample_function("bar")])
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = test_dir("roundtrip");
        assert_eq!(FileCache::open(&dir).get("foo.ts", b"a"), None);
        let mut cache = FileCache::open(&dir);
        let content = b"function foo() {}";
        cache.put("foo.ts", content, vec![sample_function("foo")]);
        cache.save(&dir).unwrap();
        let loaded = FileCache::open(&dir);
        assert_eq!(
            loaded.get("foo.ts", content),
            Some(vec![sample_function("foo")])
        );
        assert_eq!(loaded.get("foo.ts", b"changed"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
