use serde::Serialize;
use std::collections::HashMap;

pub const METRIC_PROFILE: &str = "default";
pub const OUTPUT_SCHEMA_VERSION: u32 = 2;
pub const CYCLOMATIC_SPEC: &str = "default";
pub const COGNITIVE_SPEC: &str = "default";
pub const HALSTEAD_SPEC: &str = "default";
pub const MAINTAINABILITY_SPEC: &str = "default";
pub const CRAP_SPEC: &str = "default";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Go,
    Java,
    JavaScript,
    Rust,
    TypeScript,
    Tsx,
}

impl Language {
    /// Stable lowercase identifier for metrics labels.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::Go => "go",
            Language::Java => "java",
            Language::JavaScript => "javascript",
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
        }
    }
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
    Try,
    Throw,
    Arrow,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LogicalOperator {
    And,
    Or,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    Decision {
        kind: DecisionKind,
        nesting: u32,
        else_if: bool,
        line: u32,
    },
    Else {
        line: u32,
        nesting: u32,
    },
    Logical {
        operator: LogicalOperator,
        sequence: u32,
        line: u32,
        nesting: u32,
    },
    LabeledJump {
        line: u32,
        nesting: u32,
    },
    NestingDepth(u32),
    Operator(Span),
    Operand(Span),
}

#[derive(Debug)]
pub struct FunctionInput {
    pub name: String,
    pub id: String,
    pub kind: FunctionKind,
    pub language: Language,
    pub recursive: bool,
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
pub struct MetricContribution {
    pub rule: String,
    pub line: u32,
    pub nesting: u32,
    pub cognitive: u32,
    pub cyclomatic: u32,
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
    pub contributions: Vec<MetricContribution>,
    #[serde(skip)]
    pub source_fingerprint: u64,
}

/// Smallest span containing `line`, ties broken by the lexical id.
///
/// Used to attribute a line to its innermost containing function; `span` and
/// `id` keep it usable for both analyzed and project-joined function rows.
pub fn innermost_containing<T>(
    items: &[T],
    line: u32,
    span: impl Fn(&T) -> (u32, u32),
    id: impl Fn(&T) -> &str,
) -> Option<&T> {
    items
        .iter()
        .filter(|item| {
            let (start, end) = span(item);
            start <= line && line <= end
        })
        .min_by(|left, right| {
            let (left_start, left_end) = span(left);
            let (right_start, right_end) = span(right);
            left_end
                .saturating_sub(left_start)
                .cmp(&right_end.saturating_sub(right_start))
                .then_with(|| id(left).cmp(id(right)))
        })
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
    /// Threshold names this function fails, in gate order. `crap_unavailable`
    /// marks a CRAP gate with no coverage record for the function, which fails
    /// closed the same way an exceeded CRAP does.
    pub fn violation_reasons(&self, metrics: &FunctionMetrics) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if self
            .cognitive
            .is_some_and(|limit| metrics.cognitive > limit)
        {
            reasons.push("cognitive");
        }
        if self
            .cyclomatic
            .is_some_and(|limit| metrics.cyclomatic > limit)
        {
            reasons.push("cyclomatic");
        }
        if self
            .max_nesting
            .is_some_and(|limit| metrics.max_nesting > limit)
        {
            reasons.push("max_nesting");
        }
        if let Some(limit) = self.crap {
            match metrics.crap {
                Some(value) if value > limit => reasons.push("crap"),
                Some(_) => {}
                None => reasons.push("crap_unavailable"),
            }
        }
        reasons
    }

    pub fn violates(&self, metrics: &FunctionMetrics) -> bool {
        !self.violation_reasons(metrics).is_empty()
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

fn decision_rule(kind: DecisionKind, else_if: bool) -> &'static str {
    if else_if {
        return "else-if";
    }
    match kind {
        DecisionKind::If => "if",
        DecisionKind::Loop => "for",
        DecisionKind::Catch => "catch",
        DecisionKind::Switch => "switch",
        DecisionKind::Case => "case",
        DecisionKind::Ternary => "ternary",
        DecisionKind::Try => "try",
        DecisionKind::Throw => "throw",
        DecisionKind::Arrow => "->",
    }
}

pub fn analyze_function(input: FunctionInput, source: &[u8]) -> FunctionAnalysis {
    let mut cyclomatic = 1;
    let mut cognitive = 0;
    let mut max_nesting = 0;
    let mut contributions = Vec::new();
    let mut distinct_operators = Vec::new();
    let mut distinct_operands = Vec::new();
    let mut total_operators = 0;
    let mut total_operands = 0;
    let mut logical_sequences = HashMap::new();

    for event in &input.events {
        match *event {
            Event::Decision {
                kind,
                nesting,
                else_if,
                line,
            } => {
                // Case/Throw/Arrow/Try add no nesting level: Case is a label,
                // and Throw/Arrow/Try are non-structural leaves, so none of
                // them raise the enclosing depth the way If/Loop/Switch/
                // Ternary do.
                if !matches!(
                    kind,
                    DecisionKind::Case
                        | DecisionKind::Throw
                        | DecisionKind::Arrow
                        | DecisionKind::Try
                ) {
                    max_nesting = max_nesting.max(if else_if { nesting } else { nesting + 1 });
                }
                let cyclomatic_increment = match kind {
                    DecisionKind::If
                    | DecisionKind::Loop
                    | DecisionKind::Case
                    | DecisionKind::Ternary
                    | DecisionKind::Try
                    | DecisionKind::Throw
                    | DecisionKind::Arrow => 1,
                    DecisionKind::Catch if input.language != Language::Java => 1,
                    DecisionKind::Catch | DecisionKind::Switch => 0,
                };
                let cognitive_increment = if else_if {
                    1
                } else if matches!(
                    kind,
                    DecisionKind::Case
                        | DecisionKind::Throw
                        | DecisionKind::Arrow
                        | DecisionKind::Try
                ) {
                    0
                } else {
                    1 + nesting
                };
                cyclomatic += cyclomatic_increment;
                cognitive += cognitive_increment;
                contributions.push(MetricContribution {
                    rule: decision_rule(kind, else_if).to_owned(),
                    line,
                    nesting,
                    cognitive: cognitive_increment,
                    cyclomatic: cyclomatic_increment,
                });
            }
            Event::Else { line, nesting } => {
                cognitive += 1;
                contributions.push(MetricContribution {
                    rule: "else".to_owned(),
                    line,
                    nesting,
                    cognitive: 1,
                    cyclomatic: 0,
                });
            }
            Event::LabeledJump { line, nesting } => {
                cognitive += 1;
                contributions.push(MetricContribution {
                    rule: "labeled-jump".to_owned(),
                    line,
                    nesting,
                    cognitive: 1,
                    cyclomatic: 0,
                });
            }
            Event::Logical {
                operator,
                sequence,
                line,
                nesting,
            } => {
                cyclomatic += 1;
                let previous = logical_sequences.insert(sequence, operator);
                let cognitive_increment = u32::from(previous != Some(operator));
                cognitive += cognitive_increment;
                contributions.push(MetricContribution {
                    rule: match operator {
                        LogicalOperator::And => "&&-sequence",
                        LogicalOperator::Or => "||-sequence",
                    }
                    .to_owned(),
                    line,
                    nesting,
                    cognitive: cognitive_increment,
                    cyclomatic: 1,
                });
            }
            Event::NestingDepth(depth) => max_nesting = max_nesting.max(depth),
            Event::Operator(span) => {
                total_operators += 1;
                distinct_operators.push(&source[span.start..span.end]);
            }
            Event::Operand(span) => {
                total_operands += 1;
                distinct_operands.push(&source[span.start..span.end]);
            }
        }
    }

    if input.recursive {
        cognitive += 1;
        contributions.push(MetricContribution {
            rule: "recursion".to_owned(),
            line: input.start_line,
            nesting: 0,
            cognitive: 1,
            cyclomatic: 0,
        });
    }

    distinct_operators.sort_unstable();
    distinct_operators.dedup();
    distinct_operands.sort_unstable();
    distinct_operands.dedup();
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
        contributions,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn input(events: Vec<Event>) -> FunctionInput {
        FunctionInput {
            name: "f".to_owned(),
            id: "f".to_owned(),
            kind: FunctionKind::Function,
            language: Language::TypeScript,
            recursive: false,
            start_line: 1,
            end_line: 10,
            start_byte: 0,
            end_byte: 100,
            parameters: 0,
            logical_loc: 0,
            events,
            source_fingerprint: 0,
        }
    }

    #[test]
    fn nested_if_contributions_are_in_source_order() {
        let analysis = analyze_function(
            input(vec![
                Event::Decision {
                    kind: DecisionKind::If,
                    nesting: 0,
                    else_if: false,
                    line: 2,
                },
                Event::NestingDepth(1),
                Event::Decision {
                    kind: DecisionKind::If,
                    nesting: 1,
                    else_if: false,
                    line: 3,
                },
                Event::NestingDepth(2),
            ]),
            b"",
        );
        assert_eq!(
            analysis.contributions,
            vec![
                MetricContribution {
                    rule: "if".to_owned(),
                    line: 2,
                    nesting: 0,
                    cognitive: 1,
                    cyclomatic: 1,
                },
                MetricContribution {
                    rule: "if".to_owned(),
                    line: 3,
                    nesting: 1,
                    cognitive: 2,
                    cyclomatic: 1,
                },
            ]
        );
        assert_eq!(analysis.metrics.cognitive, 3);
        assert_eq!(analysis.metrics.cyclomatic, 3);
    }

    #[test]
    fn straight_line_function_has_no_contributions() {
        let source = b"return x;";
        let analysis = analyze_function(
            input(vec![
                Event::Operator(Span { start: 0, end: 6 }),
                Event::Operand(Span { start: 7, end: 8 }),
            ]),
            source,
        );
        assert!(analysis.contributions.is_empty());
        assert_eq!(analysis.metrics.cyclomatic, 1);
        assert_eq!(analysis.metrics.cognitive, 0);
    }

    #[test]
    fn java_catch_costs_no_cyclomatic_but_keeps_cognitive() {
        let analysis = analyze_function(
            {
                let mut base = input(vec![Event::Decision {
                    kind: DecisionKind::Catch,
                    nesting: 0,
                    else_if: false,
                    line: 5,
                }]);
                base.language = Language::Java;
                base
            },
            b"",
        );
        assert_eq!(analysis.metrics.cyclomatic, 1);
        assert_eq!(analysis.metrics.cognitive, 1);
        assert_eq!(analysis.contributions[0].rule, "catch");
        assert_eq!(analysis.contributions[0].cyclomatic, 0);
    }

    #[test]
    fn throw_and_arrow_cost_cyclomatic_only() {
        for kind in [DecisionKind::Throw, DecisionKind::Arrow] {
            let analysis = analyze_function(
                input(vec![Event::Decision {
                    kind,
                    nesting: 0,
                    else_if: false,
                    line: 5,
                }]),
                b"",
            );
            assert_eq!(analysis.metrics.cyclomatic, 2);
            assert_eq!(analysis.metrics.cognitive, 0);
        }
    }

    #[test]
    fn recursion_costs_one_cognitive_and_no_cyclomatic() {
        let mut base = input(vec![Event::Decision {
            kind: DecisionKind::If,
            nesting: 0,
            else_if: false,
            line: 2,
        }]);
        base.recursive = true;
        let analysis = analyze_function(base, b"");
        assert_eq!(analysis.metrics.cyclomatic, 2);
        assert_eq!(analysis.metrics.cognitive, 2);
        let last = analysis.contributions.last().unwrap();
        assert_eq!(last.rule, "recursion");
        assert_eq!((last.cognitive, last.cyclomatic), (1, 0));
    }

    #[test]
    fn non_recursive_function_has_no_recursion_contribution() {
        let analysis = analyze_function(input(vec![]), b"");
        assert!(analysis.contributions.iter().all(|c| c.rule != "recursion"));
        assert_eq!(analysis.metrics.cognitive, 0);
    }
}
