//! Test impact analysis: select the tests a change can affect.
//!
//! The crate is a skeleton. Selection, stable symbol keys, and the recorders
//! are the next project and get their own spec; what this pass proves is that
//! the six dependencies below are sufficient and legal — the parser and the
//! coverage and test-map readers (`parse`), the shared shapes (`model`), the
//! run settings (`config`), churn history (`codegraph`), the dependency graph
//! and the test targets (`analysis`), and the recorded test evidence
//! (`artifacts`) — and that nothing above `artifacts` is reachable from here.

use analysis::test_targets::{TestTarget, test_targets};
use artifacts::mutation::MutationInput;
use codegraph::history::HistoryWindow;
use config::Config;
use model::Result;
use model::core::{AnalysisReport, FileAnalysis};
use parse::coverage::CoverageMap;
use parse::{analysis_report, analyze_source};

/// The readers one test-impact selection starts from for a single source file.
#[derive(Debug)]
pub struct SelectionInputs {
    /// `parse`: the file's parsed facts.
    pub file: FileAnalysis,
    /// `model`: the same facts rolled into the report shape.
    pub report: AnalysisReport,
    /// `parse`: the coverage the run recorded, keyed by normalized path.
    pub coverage: CoverageMap,
    /// `analysis`: the functions that coverage leaves uncovered.
    pub test_targets: Vec<TestTarget>,
    /// `codegraph`: the churn window a change is scored over.
    pub window: HistoryWindow,
    /// `artifacts`: the recorded test evidence, when a run supplied any.
    pub mutation: Option<MutationInput>,
    /// `config`: the run's thresholds and exclusions.
    pub config: Config,
}

/// Reads [`SelectionInputs`] for one source file.
///
/// Nothing is selected here: this gathers the readers a selector will use, so
/// the dependency set is exercised rather than declared.
pub fn read_inputs(
    path: &str,
    source: &[u8],
    coverage: CoverageMap,
    window: HistoryWindow,
    mutation: Option<MutationInput>,
    config: Config,
) -> Result<SelectionInputs> {
    let file = analyze_source(path, source)?;
    let report = analysis_report(vec![file.clone()]);
    let test_targets = test_targets(&report, &coverage);
    Ok(SelectionInputs {
        file,
        report,
        coverage,
        test_targets,
        window,
        mutation,
        config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "fn covered() {}\nfn uncovered(x: bool) { if x { return; } }\n";
    const LCOV: &str = "SF:src/lib.rs\nDA:1,1\nDA:2,0\nend_of_record\n";

    #[test]
    fn reads_one_item_from_each_dependency() {
        let inputs = read_inputs(
            "src/lib.rs",
            SOURCE.as_bytes(),
            CoverageMap::from_lcov(LCOV).expect("valid LCOV"),
            HistoryWindow::parse("30d").expect("known window"),
            Some(MutationInput::Pit("mutation.json".into())),
            config::parse_str("").expect("empty TOML is valid"),
        )
        .expect("valid Rust source analyzes");

        assert_eq!(inputs.file.path, "src/lib.rs");
        assert_eq!(inputs.report.files.len(), 1);
        assert_eq!(inputs.window.label(), "30d");
        assert_eq!(
            inputs.mutation,
            Some(MutationInput::Pit("mutation.json".into()))
        );
        assert!(inputs.config.analysis_excludes.is_empty());
        // Line 1 is covered and line 2 is recorded with zero hits, so only
        // the second function is a target.
        assert_eq!(inputs.test_targets.len(), 1, "{:?}", inputs.test_targets);
        assert_eq!(inputs.test_targets[0].function, "uncovered");
        assert_eq!(inputs.test_targets[0].path, "src/lib.rs");
    }
}
