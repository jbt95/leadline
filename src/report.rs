use crate::core::{AnalysisReport, FunctionAnalysis};
use crate::coupling::CouplingReport;
use crate::diff::ChangedReport;
use crate::graph::DependencyReport;
use crate::hotspots::HotspotReport;
use crate::impact::ImpactReport;
use crate::risk::RiskReport;
use std::fmt::Write;

pub fn terminal(report: &AnalysisReport) -> String {
    let mut output = String::new();
    for file in &report.files {
        if file.functions.is_empty() && file.parse_errors.is_empty() {
            continue;
        }
        let _ = writeln!(output, "{}\n", file.path);
        for diagnostic in &file.parse_errors {
            let _ = writeln!(
                output,
                "  Parse {} at {}:{}-{}:{}",
                diagnostic.kind,
                diagnostic.start_line,
                diagnostic.start_column,
                diagnostic.end_line,
                diagnostic.end_column
            );
        }
        if !file.parse_errors.is_empty() {
            output.push('\n');
        }
        for function in &file.functions {
            function_block(&mut output, function);
        }
    }
    output
}

pub fn terminal_changed(report: &ChangedReport) -> String {
    let mut output = String::new();
    for file in &report.parse_errors {
        let _ = writeln!(output, "{}\n", file.path);
        for (side, diagnostics) in [("before", &file.before), ("after", &file.after)] {
            for diagnostic in diagnostics {
                let _ = writeln!(
                    output,
                    "  Parse {} ({side}) at {}:{}-{}:{}",
                    diagnostic.kind,
                    diagnostic.start_line,
                    diagnostic.start_column,
                    diagnostic.end_line,
                    diagnostic.end_column
                );
            }
        }
        output.push('\n');
    }
    let mut current_path = "";
    for change in &report.functions {
        if change.path != current_path {
            current_path = &change.path;
            let _ = writeln!(output, "{}\n", change.path);
        }
        let _ = writeln!(output, "{}()", change.name);
        delta_line(
            &mut output,
            "Cognitive",
            change.before.as_ref().map(|f| f.metrics.cognitive as f64),
            change.after.as_ref().map(|f| f.metrics.cognitive as f64),
        );
        delta_line(
            &mut output,
            "Cyclomatic",
            change.before.as_ref().map(|f| f.metrics.cyclomatic as f64),
            change.after.as_ref().map(|f| f.metrics.cyclomatic as f64),
        );
        delta_line(
            &mut output,
            "CRAP",
            change.before.as_ref().and_then(|f| f.metrics.crap),
            change.after.as_ref().and_then(|f| f.metrics.crap),
        );
        delta_line(
            &mut output,
            "LOC",
            change.before.as_ref().map(|f| f.metrics.loc as f64),
            change.after.as_ref().map(|f| f.metrics.loc as f64),
        );
        output.push('\n');
    }
    output
}

pub fn terminal_coupling(report: &CouplingReport) -> String {
    let mut output = String::new();
    if !report.git_available {
        output.push_str("Git history unavailable (no repository or no commits).\n");
        return output;
    }
    let _ = writeln!(
        output,
        "Historically related files (target: {})\n",
        report.target
    );
    if report.related.is_empty() {
        let _ = writeln!(
            output,
            "No repeated co-changes found (try --min-cochanges 1)."
        );
        return output;
    }
    for related in &report.related {
        let _ = writeln!(
            output,
            "{:<40} {:>5.0}%   co-changes {} of {}   jaccard {:.0}%",
            related.path,
            related.directional * 100.0,
            related.co_changes,
            report.target_commits,
            related.jaccard * 100.0
        );
    }
    if report.truncated {
        let _ = writeln!(
            output,
            "\nShowing the top {} related files (raise --top for more).",
            report.related.len()
        );
    }
    output
}

pub fn terminal_dependencies(report: &DependencyReport) -> String {
    let mut output = String::new();
    let cycle_word = if report.cycles.len() == 1 {
        "cycle"
    } else {
        "cycles"
    };
    let _ = writeln!(
        output,
        "Dependencies ({} files, {} edges, {} {cycle_word})\n",
        report.files.len(),
        report.edges.len(),
        report.cycles.len()
    );
    let mut by_fan_in: Vec<&crate::graph::DependencyFile> = report.files.iter().collect();
    by_fan_in.sort_by(|left, right| {
        right
            .fan_in
            .cmp(&left.fan_in)
            .then(left.path.cmp(&right.path))
    });
    let mut by_fan_out: Vec<&crate::graph::DependencyFile> = report.files.iter().collect();
    by_fan_out.sort_by(|left, right| {
        right
            .fan_out
            .cmp(&left.fan_out)
            .then(left.path.cmp(&right.path))
    });
    let _ = writeln!(output, "Top fan-in");
    for file in by_fan_in.iter().take(5) {
        let _ = writeln!(
            output,
            "  {:<40} fan-in {} fan-out {}",
            file.path, file.fan_in, file.fan_out
        );
    }
    output.push('\n');
    let _ = writeln!(output, "Top fan-out");
    for file in by_fan_out.iter().take(5) {
        let _ = writeln!(
            output,
            "  {:<40} fan-in {} fan-out {}",
            file.path, file.fan_in, file.fan_out
        );
    }
    output.push('\n');
    if report.cycles.is_empty() {
        output.push_str("No cycles found.\n");
    } else {
        let _ = writeln!(output, "Cycles ({})", report.cycles.len());
        for (index, cycle) in report.cycles.iter().enumerate() {
            let _ = writeln!(output, "  {}. {}", index + 1, cycle.files.join(" -> "));
        }
    }
    output
}

pub fn terminal_impact(report: &ImpactReport) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "Impact (target: {})\n", report.target);
    let _ = writeln!(output, "Target: {}", report.target);
    let _ = writeln!(output, "  Fan-in: {}", report.fan_in);
    let _ = writeln!(output, "  Fan-out: {}", report.fan_out);
    let _ = writeln!(output, "  Direct dependents: {}", report.direct_dependents);
    let _ = writeln!(
        output,
        "  Blast radius: {} of {} files ({:.1}%)",
        report.blast_radius, report.files_analyzed, report.blast_radius_percent
    );
    if report.dependents.is_empty() {
        let _ = writeln!(output, "\nNo dependents found.");
    } else {
        let _ = writeln!(
            output,
            "\nDependents ({} shown, {} total)",
            report.dependents.len(),
            report.blast_radius
        );
        for dependent in &report.dependents {
            let _ = writeln!(
                output,
                "  {} (distance {})",
                dependent.path, dependent.distance
            );
        }
    }
    if report.truncated {
        let _ = writeln!(
            output,
            "\nShowing the top {} dependents (raise --top for more).",
            report.dependents.len()
        );
    }
    if !report.cycles.is_empty() {
        let _ = writeln!(
            output,
            "\nCycles involving target ({})",
            report.cycles.len()
        );
        for cycle in &report.cycles {
            let _ = writeln!(output, "  {}", cycle.join(" -> "));
        }
    }
    output
}

pub fn terminal_hotspots(report: &HotspotReport) -> String {
    let mut output = String::new();
    if !report.git_available {
        output.push_str(
            "Git history unavailable (no repository or no commits); ranking by complexity only.\n\n",
        );
    }
    if report.hotspots.is_empty() {
        return output;
    }
    let _ = writeln!(
        output,
        "Top engineering hotspots (window: {})\n",
        report.window
    );
    for (index, hotspot) in report.hotspots.iter().enumerate() {
        let _ = writeln!(output, "{}. {}", index + 1, hotspot.path);
        output.push('\n');
        match (hotspot.score, hotspot.changes) {
            (Some(score), Some(changes)) => {
                let _ = writeln!(
                    output,
                    "   Hotspot score      {score:>8}  (max cognitive {} x changes/{} {changes})",
                    hotspot.max_cognitive, report.window
                );
            }
            _ => {
                let _ = writeln!(
                    output,
                    "   Hotspot score            n/a  (Git history unavailable)"
                );
            }
        }
        let _ = writeln!(output, "   Cognitive          {:>8}", hotspot.max_cognitive);
        let _ = writeln!(
            output,
            "   Cyclomatic         {:>8}",
            hotspot.max_cyclomatic
        );
        let _ = writeln!(
            output,
            "   CRAP               {:>8}",
            hotspot
                .max_crap
                .map(|value| format!("{value:.1}"))
                .unwrap_or_else(|| "unavailable".to_owned())
        );
        let _ = writeln!(
            output,
            "   Coverage           {:>8}",
            hotspot
                .coverage
                .map(|value| format!("{:.1}%", value * 100.0))
                .unwrap_or_else(|| "unavailable".to_owned())
        );
        let _ = writeln!(output, "   LOC                {:>8}", hotspot.loc);
        let _ = writeln!(output, "   Functions          {:>8}", hotspot.functions);
        if let Some(changes) = hotspot.changes_30d {
            let _ = writeln!(output, "   Changes / 30 days  {changes:>8}");
        }
        if let Some(changes) = hotspot.changes_90d {
            let _ = writeln!(output, "   Changes / 90 days  {changes:>8}");
        }
        if let Some(changes) = hotspot.changes_365d {
            let _ = writeln!(output, "   Changes / 365 days {changes:>8}");
        }
        if let Some(contributors) = hotspot.contributors {
            let _ = writeln!(output, "   Contributors       {contributors:>8}");
        }
        if let Some(recent) = hotspot.recent_contributors {
            let _ = writeln!(output, "   Recent contributors {recent:>8}");
        }
        if let Some(days) = hotspot.days_since_last_change {
            let _ = writeln!(output, "   Last change        {days:>8} days ago");
        }
        output.push('\n');
    }
    if report.truncated {
        let _ = writeln!(
            output,
            "Showing the top {} of {} files (raise --limit for more).\n",
            report.hotspots.len(),
            report.files_analyzed
        );
    }
    output
}

/// Terminal summary of a canonical Project build.
pub fn terminal_project(report: &crate::project::Project) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Project ({}): {} files, {} functions, {} dependency edges\n",
        report.meta.generated_from,
        report.summary.files,
        report.summary.functions,
        report.summary.dependency_edges
    );
    if let Some(commit) = &report.meta.head_commit {
        let _ = writeln!(output, "HEAD             {commit}");
    }
    let _ = writeln!(output, "Risk model       {}", report.risk.model);
    let _ = writeln!(
        output,
        "Coverage         {}",
        report
            .summary
            .coverage_percent
            .map(|percent| format!("{percent:.1}%"))
            .unwrap_or_else(|| "n/a".to_owned())
    );
    let _ = writeln!(
        output,
        "Duplication      {} groups, {} duplicated lines{}",
        report.summary.duplication_groups,
        report.summary.duplicated_lines,
        if report.duplication.complete {
            ""
        } else {
            " (incomplete)"
        }
    );
    let _ = writeln!(
        output,
        "Policy           {} violations",
        report.summary.policy_violations
    );
    if !report.risk.rows.is_empty() {
        let _ = writeln!(output, "\nTop risks");
        for row in report.risk.rows.iter().take(5) {
            let _ = writeln!(output, "  {:>6.1}  {}", row.score, row.path);
        }
    }
    output
}

/// Terminal summary of a full-state debt comparison.
pub fn terminal_debt(report: &crate::debt::DebtReport) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Debt {} -> {} (model: {})\n",
        report.base,
        report.target_commit.as_deref().unwrap_or("worktree"),
        report.model
    );
    let _ = writeln!(
        output,
        "Debt findings    new {} / existing {} / resolved {} / unknown {}",
        report.summary.new,
        report.summary.existing,
        report.summary.resolved,
        report.summary.unknown
    );
    let _ = writeln!(
        output,
        "Risk changes     +{} / -{} / added {} / removed {}",
        report.summary.risk_increased,
        report.summary.risk_decreased,
        report.summary.risk_added,
        report.summary.risk_removed
    );
    if !report.findings.is_empty() {
        let _ = writeln!(output, "\nFindings");
        for finding in report.findings.iter().take(20) {
            let status = match finding.status {
                crate::debt::DebtStatus::New => "new",
                crate::debt::DebtStatus::Existing => "existing",
                crate::debt::DebtStatus::Resolved => "resolved",
            };
            let _ = writeln!(
                output,
                "  {:8} {} {} {} ({} -> {})",
                status,
                finding.path,
                finding.name,
                finding.dimension,
                finding
                    .before
                    .map(|value| format!("{value:.1}"))
                    .unwrap_or_else(|| "n/a".to_owned()),
                finding
                    .after
                    .map(|value| format!("{value:.1}"))
                    .unwrap_or_else(|| "n/a".to_owned())
            );
        }
    }
    if !report.risk_changes.is_empty() {
        let _ = writeln!(output, "\nRisk changes");
        for change in report.risk_changes.iter().take(20) {
            let status = match change.status {
                crate::debt::RiskChangeStatus::Added => "added",
                crate::debt::RiskChangeStatus::Increased => "increased",
                crate::debt::RiskChangeStatus::Decreased => "decreased",
                crate::debt::RiskChangeStatus::Removed => "removed",
                crate::debt::RiskChangeStatus::Unchanged => "unchanged",
            };
            let _ = writeln!(
                output,
                "  {:9} {} ({} -> {})",
                status,
                change.path,
                change
                    .before_score
                    .map(|value| format!("{value:.1}"))
                    .unwrap_or_else(|| "n/a".to_owned()),
                change
                    .after_score
                    .map(|value| format!("{value:.1}"))
                    .unwrap_or_else(|| "n/a".to_owned())
            );
        }
    }
    output
}

/// Terminal summary of one duplication report.
pub fn terminal_duplication(report: &crate::duplication::DuplicationReport) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Duplication ({}){}",
        report.profile,
        if report.complete { "" } else { " [incomplete]" }
    );
    let _ = writeln!(
        output,
        "  {} groups, {} duplicated lines, {} tokens, {} comparisons",
        report.groups.len(),
        report.duplicated_lines,
        report.total_tokens,
        report.comparisons
    );
    for group in report.groups.iter().take(10) {
        let _ = writeln!(
            output,
            "  {} tokens, {} occurrences",
            group.token_count,
            group.occurrences.len()
        );
        for occurrence in group.occurrences.iter().take(5) {
            let _ = writeln!(
                output,
                "    {}:{}-{}",
                occurrence.path, occurrence.start_line, occurrence.end_line
            );
        }
    }
    output
}

/// Terminal summary of duplication drift.
pub fn terminal_duplication_drift(report: &crate::duplication::DuplicationDriftReport) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "Duplication drift (model: {})", report.model);
    let _ = writeln!(
        output,
        "  groups {}, added occurrences {}, removed occurrences {}",
        report.groups.len(),
        report.added_occurrences,
        report.removed_occurrences
    );
    for group in report.groups.iter().take(10) {
        let status = match group.status {
            Some(crate::duplication::GroupStatus::New) => "new",
            Some(crate::duplication::GroupStatus::Existing) => "existing",
            Some(crate::duplication::GroupStatus::Resolved) => "resolved",
            None => "unknown",
        };
        let _ = writeln!(
            output,
            "  {:8} {} tokens, {} occurrences",
            status,
            group.token_count,
            group.occurrences.len()
        );
    }
    output
}

/// Terminal summary of policy violations.
pub fn terminal_policy(report: &crate::policy::PolicyReport) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Policy: {} rules, {} info / {} warning / {} error",
        report.rules, report.info, report.warning, report.error
    );
    for violation in report.violations.iter().take(20) {
        let status = match violation.status {
            Some(crate::policy::PolicyStatus::New) => " [new]",
            Some(crate::policy::PolicyStatus::Existing) => " [existing]",
            Some(crate::policy::PolicyStatus::Resolved) => " [resolved]",
            None => "",
        };
        let _ = writeln!(
            output,
            "  {:8} {} -> {} ({}){}",
            match violation.severity {
                crate::config::Severity::Info => "info",
                crate::config::Severity::Warning => "warning",
                crate::config::Severity::Error => "error",
            },
            violation.source,
            violation.target,
            violation.rule,
            status
        );
    }
    output
}

/// Terminal summary of normalized mutation and test-map inputs.
pub fn terminal_mutation(
    report: &crate::mutation::MutationReport,
    relationships: Option<&crate::test_relationships::TestRelationshipReport>,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "Mutation (model: {})", report.model);
    let summary = &report.summary;
    let _ = writeln!(
        output,
        "  {} mutants: {} killed, {} timed out, {} survived, {} no coverage, {} unresolved",
        summary.total,
        summary.killed,
        summary.timed_out,
        summary.survived,
        summary.no_coverage,
        summary.unresolved
    );
    let _ = writeln!(
        output,
        "  score {} over {} scored mutants",
        summary
            .score
            .map(|score| format!("{score:.1}%"))
            .unwrap_or_else(|| "n/a".to_owned()),
        summary.scored_mutants
    );
    if let Some(relationships) = relationships {
        let _ = writeln!(
            output,
            "  test relationships: {} resolved, {} unresolved",
            relationships.relationships.len() as u64 - relationships.unresolved,
            relationships.unresolved
        );
    }
    output
}

pub fn terminal_risk(report: &RiskReport) -> String {
    let mut output = String::new();
    if !report.git_available {
        output.push_str(
            "Git history unavailable (no repository or no commits); churn and ownership unknown.\n\n",
        );
    }
    if report.risks.is_empty() {
        return output;
    }
    let _ = writeln!(
        output,
        "Top change risks (window: {}, model: {})\n",
        report.window, report.model
    );
    for (index, risk) in report.risks.iter().enumerate() {
        let _ = writeln!(output, "{}. {}", index + 1, risk.path);
        output.push('\n');
        let _ = writeln!(output, "   Risk score       {:>8.1}", risk.score);
        let _ = writeln!(
            output,
            "   Complexity       {:>8}",
            risk_component(risk.components.complexity)
        );
        let _ = writeln!(
            output,
            "   CRAP             {:>8}",
            risk_component(risk.components.crap)
        );
        let _ = writeln!(
            output,
            "   Churn            {:>8}",
            risk_component(risk.components.churn)
        );
        let _ = writeln!(
            output,
            "   Impact           {:>8}",
            risk_component(risk.components.impact)
        );
        let _ = writeln!(
            output,
            "   Ownership        {:>8}",
            risk_component(risk.components.ownership)
        );
        let _ = writeln!(
            output,
            "   Policy           {:>8}",
            risk_component(risk.components.policy)
        );
        let _ = writeln!(output, "   Cognitive (max)  {:>8}", risk.raw.max_cognitive);
        let _ = writeln!(output, "   Cyclomatic (max) {:>8}", risk.raw.max_cyclomatic);
        let _ = writeln!(
            output,
            "   CRAP (max)       {:>8}",
            risk.raw
                .max_crap
                .map(|value| format!("{value:.1}"))
                .unwrap_or_else(|| "n/a".to_owned())
        );
        let _ = writeln!(
            output,
            "   Changes / {}  {:>8}",
            report.window,
            risk.raw
                .changes
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_owned())
        );
        let _ = writeln!(
            output,
            "   Contributors     {:>8}",
            risk.raw
                .contributors
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_owned())
        );
        let _ = writeln!(
            output,
            "   Blast radius     {:>8} of {} files ({:.1}%)",
            risk.raw.blast_radius,
            report.scope_files.max(1),
            risk.raw.blast_radius_percent
        );
        let _ = writeln!(
            output,
            "   Fan-in / out     {:>8} / {}",
            risk.raw.fan_in, risk.raw.fan_out
        );
        output.push('\n');
    }
    if report.truncated {
        let _ = writeln!(
            output,
            "Showing the top {} of {} files (raise --limit for more).\n",
            report.risks.len(),
            report.files_analyzed
        );
    }
    output
}

fn risk_component(value: Option<f64>) -> String {
    value
        .map(|score| format!("{score:.1}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn function_block(output: &mut String, function: &FunctionAnalysis) {
    let metrics = &function.metrics;
    let _ = writeln!(output, "{}()", function.name);
    let _ = writeln!(output, "  Cognitive:     {}", metrics.cognitive);
    let _ = writeln!(output, "  Cyclomatic:    {}", metrics.cyclomatic);
    let _ = writeln!(output, "  Halstead V:    {:.1}", metrics.halstead_volume);
    let _ = writeln!(output, "  LOC:           {}", metrics.loc);
    let _ = writeln!(output, "  Logical LOC:   {}", metrics.logical_loc);
    let _ = writeln!(output, "  Parameters:    {}", metrics.parameters);
    let _ = writeln!(output, "  Max nesting:   {}", metrics.max_nesting);
    let _ = writeln!(
        output,
        "  Coverage:      {}",
        metrics
            .coverage
            .map(|value| format!("{:.1}%", value * 100.0))
            .unwrap_or_else(|| "unavailable".to_owned())
    );
    let _ = writeln!(
        output,
        "  CRAP:          {}\n",
        metrics
            .crap
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "unavailable".to_owned())
    );
}

fn delta_line(output: &mut String, label: &str, before: Option<f64>, after: Option<f64>) {
    let before_text = before.map(format_number).unwrap_or_else(|| "—".to_owned());
    let after_text = after.map(format_number).unwrap_or_else(|| "—".to_owned());
    let change = before
        .zip(after)
        .map(|(left, right)| format!("{:+.1}", right - left))
        .unwrap_or_default();
    let _ = writeln!(
        output,
        "  {label:<12} {before_text:>6} → {after_text:<6} {change:>7}"
    );
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}
