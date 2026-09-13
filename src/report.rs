use crate::core::{AnalysisReport, FunctionAnalysis};
use crate::coupling::CouplingReport;
use crate::diff::ChangedReport;
use crate::graph::DependencyReport;
use crate::hotspots::HotspotReport;
use crate::impact::ImpactReport;
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
