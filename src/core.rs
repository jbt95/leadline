use serde::Serialize;
use std::collections::{HashMap, HashSet};

pub const METRIC_PROFILE: &str = "default-v1";
pub const OUTPUT_SCHEMA_VERSION: u32 = 1;
pub const CYCLOMATIC_SPEC: &str = "default-v1";
pub const COGNITIVE_SPEC: &str = "default-v1";
pub const HALSTEAD_SPEC: &str = "default-v1";
pub const MAINTAINABILITY_SPEC: &str = "default-v1";
pub const CRAP_SPEC: &str = "default-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Java,
    JavaScript,
    TypeScript,
    Tsx,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionKind {
    Function,
    Method,
    Constructor,
    Lambda,
    Arrow,
    Anonymous,
}

impl FunctionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FunctionKind::Function => "function",
            FunctionKind::Method => "method",
            FunctionKind::Constructor => "constructor",
            FunctionKind::Lambda => "lambda",
            FunctionKind::Arrow => "arrow",
            FunctionKind::Anonymous => "anonymous",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionKind {
    If,
    Loop,
    Catch,
    Switch,
    Case,
    Ternary,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LogicalOperator {
    And,
    Or,
    Nullish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    Decision {
        kind: DecisionKind,
        nesting: u32,
        else_if: bool,
    },
    Else,
    Logical {
        operator: LogicalOperator,
        sequence: u32,
    },
    LabeledJump,
    NestingDepth(u32),
    Operator(Span),
    Operand(Span),
}

#[derive(Debug)]
pub struct FunctionInput {
    pub name: String,
    pub id: String,
    pub kind: FunctionKind,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    pub parameters: u32,
    pub logical_loc: u32,
    pub events: Vec<Event>,
    pub source_fingerprint: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FunctionMetrics {
    pub loc: u32,
    pub logical_loc: u32,
    pub function_length: u32,
    pub parameters: u32,
    pub max_nesting: u32,
    pub cyclomatic: u32,
    pub cognitive: u32,
    pub halstead_n1: u32,
    pub halstead_n2: u32,
    #[serde(rename = "halstead_N1")]
    pub halstead_total_operators: u32,
    #[serde(rename = "halstead_N2")]
    pub halstead_total_operands: u32,
    pub halstead_vocabulary: u32,
    pub halstead_length: u32,
    pub halstead_volume: f64,
    pub halstead_difficulty: f64,
    pub halstead_effort: f64,
    pub maintainability_index: f64,
    pub coverage: Option<f64>,
    pub crap: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FunctionAnalysis {
    pub name: String,
    pub id: String,
    pub kind: FunctionKind,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    pub metrics: FunctionMetrics,
    #[serde(skip)]
    pub source_fingerprint: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ParseDiagnostic {
    pub kind: &'static str,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FileAnalysis {
    pub path: String,
    pub language: Language,
    pub functions: Vec<FunctionAnalysis>,
    pub parse_errors: Vec<ParseDiagnostic>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MetricSpecs {
    pub cyclomatic: &'static str,
    pub cognitive: &'static str,
    pub halstead: &'static str,
    pub maintainability: &'static str,
    pub crap: &'static str,
}

impl Default for MetricSpecs {
    fn default() -> Self {
        MetricSpecs {
            cyclomatic: CYCLOMATIC_SPEC,
            cognitive: COGNITIVE_SPEC,
            halstead: HALSTEAD_SPEC,
            maintainability: MAINTAINABILITY_SPEC,
            crap: CRAP_SPEC,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AnalysisReport {
    pub schema_version: u32,
    pub metric_profile: &'static str,
    pub analyzer_version: &'static str,
    pub metric_specs: MetricSpecs,
    pub files: Vec<FileAnalysis>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Thresholds {
    pub cognitive: Option<u32>,
    pub cyclomatic: Option<u32>,
    pub crap: Option<f64>,
    pub max_nesting: Option<u32>,
}

impl Thresholds {
    pub fn violates(&self, metrics: &FunctionMetrics) -> bool {
        self.cognitive
            .is_some_and(|limit| metrics.cognitive > limit)
            || self
                .cyclomatic
                .is_some_and(|limit| metrics.cyclomatic > limit)
            || self
                .max_nesting
                .is_some_and(|limit| metrics.max_nesting > limit)
            || self
                .crap
                .is_some_and(|limit| metrics.crap.is_none_or(|value| value > limit))
    }

    pub fn is_empty(&self) -> bool {
        self.cognitive.is_none()
            && self.cyclomatic.is_none()
            && self.crap.is_none()
            && self.max_nesting.is_none()
    }
}

pub fn function_id(path: &str, kind: FunctionKind, start_byte: u64, end_byte: u64) -> String {
    format!("{}:{}:{start_byte}:{end_byte}", path, kind.as_str())
}

pub fn analyze_function(input: FunctionInput, source: &[u8]) -> FunctionAnalysis {
    let mut cyclomatic = 1;
    let mut cognitive = 0;
    let mut max_nesting = 0;
    let mut distinct_operators = HashSet::new();
    let mut distinct_operands = HashSet::new();
    let mut total_operators = 0;
    let mut total_operands = 0;
    let mut logical_sequences = HashMap::new();

    for event in &input.events {
        match *event {
            Event::Decision {
                kind,
                nesting,
                else_if,
            } => {
                if !matches!(kind, DecisionKind::Case) {
                    max_nesting = max_nesting.max(if else_if { nesting } else { nesting + 1 });
                }
                if matches!(
                    kind,
                    DecisionKind::If
                        | DecisionKind::Loop
                        | DecisionKind::Catch
                        | DecisionKind::Case
                        | DecisionKind::Ternary
                ) {
                    cyclomatic += 1;
                }
                cognitive += if else_if {
                    1
                } else if matches!(kind, DecisionKind::Case) {
                    0
                } else {
                    1 + nesting
                };
            }
            Event::Else | Event::LabeledJump => cognitive += 1,
            Event::Logical { operator, sequence } => {
                cyclomatic += 1;
                let previous = logical_sequences.insert(sequence, operator);
                if previous != Some(operator) {
                    cognitive += 1;
                }
            }
            Event::NestingDepth(depth) => max_nesting = max_nesting.max(depth),
            Event::Operator(span) => {
                total_operators += 1;
                distinct_operators.insert(&source[span.start..span.end]);
            }
            Event::Operand(span) => {
                total_operands += 1;
                distinct_operands.insert(&source[span.start..span.end]);
            }
        }
    }

    let n1 = distinct_operators.len() as u32;
    let n2 = distinct_operands.len() as u32;
    let vocabulary = n1 + n2;
    let length = total_operators + total_operands;
    let volume = if vocabulary == 0 {
        0.0
    } else {
        f64::from(length) * f64::from(vocabulary).log2()
    };
    let difficulty = if n2 == 0 {
        0.0
    } else {
        f64::from(n1) / 2.0 * f64::from(total_operands) / f64::from(n2)
    };
    let effort = difficulty * volume;
    let loc = input.end_line - input.start_line + 1;
    let maintainability_index = ((171.0
        - 5.2 * volume.max(1.0).ln()
        - 0.23 * f64::from(cyclomatic)
        - 16.2 * f64::from(loc).ln())
        * 100.0
        / 171.0)
        .clamp(0.0, 100.0);

    FunctionAnalysis {
        name: input.name,
        id: input.id,
        kind: input.kind,
        start_line: input.start_line,
        end_line: input.end_line,
        start_byte: input.start_byte,
        end_byte: input.end_byte,
        metrics: FunctionMetrics {
            loc,
            logical_loc: input.logical_loc,
            function_length: loc,
            parameters: input.parameters,
            max_nesting,
            cyclomatic,
            cognitive,
            halstead_n1: n1,
            halstead_n2: n2,
            halstead_total_operators: total_operators,
            halstead_total_operands: total_operands,
            halstead_vocabulary: vocabulary,
            halstead_length: length,
            halstead_volume: volume,
            halstead_difficulty: difficulty,
            halstead_effort: effort,
            maintainability_index,
            coverage: None,
            crap: None,
        },
        source_fingerprint: input.source_fingerprint,
    }
}

pub fn apply_coverage(function: &mut FunctionAnalysis, coverage: Option<f64>) {
    function.metrics.coverage = coverage;
    function.metrics.crap = coverage.map(|covered| {
        let complexity = f64::from(function.metrics.cyclomatic);
        complexity * complexity * (1.0 - covered).powi(3) + complexity
    });
}
