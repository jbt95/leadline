use leadline::agent::{Budget, SortKey};
use leadline::config::{self, Config, RegressionLimits};
use leadline::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, METRIC_PROFILE, MetricSpecs,
    OUTPUT_SCHEMA_VERSION, Thresholds,
};
use leadline::coupling::CouplingOptions;
use leadline::coverage::CoverageMap;
use leadline::history::HistoryWindow;
use leadline::mutation::MutationInput;
use leadline::ownership::OwnershipMode;
use leadline::source_snapshot::SnapshotTarget;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// CLI failure carrying a stable exit code.
///
/// Codes: 0 success / gate passed, 1 gate failed (returned as [`ExitCode`],
/// never an error), 2 usage or config error, 3 incomplete analysis, 4
/// coverage input error, 5 internal error.
#[derive(Debug)]
struct CliError {
    code: u8,
    message: String,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    fn incomplete(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: message.into(),
        }
    }

    fn coverage(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            message: message.into(),
        }
    }

    fn input(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: 5,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<leadline::config::ConfigError> for CliError {
    fn from(error: leadline::config::ConfigError) -> Self {
        Self::usage(error.to_string())
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("leadline: {}", error.message);
            ExitCode::from(error.code)
        }
    }
}

fn run(args: Vec<String>) -> Result<ExitCode, CliError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(CliError::usage(usage()));
    };
    if args
        .iter()
        .skip(1)
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("{}", usage());
        return Ok(ExitCode::SUCCESS);
    }
    /// Canonical agent skill, baked in at compile time so `leadline skill`
    /// works wherever the binary runs.
    const SKILL_TEXT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/common/leadline-skill/SKILL.md"
    ));

    match command {
        "analyze" => analyze_command(&args[1..]),
        "function" => function_command(&args[1..]),
        "changed" | "diff" => changed_command(command, &args[1..]),
        "check" => check_command(&args[1..]),
        "baseline" => baseline_command(&args[1..]),
        "hotspots" => hotspots_command(&args[1..]),
        "risk" => risk_command(&args[1..]),
        "coupling" => coupling_command(&args[1..]),
        "dependencies" => dependencies_command(&args[1..]),
        "impact" => impact_command(&args[1..]),
        "test-targets" => test_targets_command(&args[1..]),
        "project" => project_command(&args[1..]),
        "debt" => debt_command(&args[1..]),
        "snapshot" => snapshot_command(&args[1..]),
        "mutation" => mutation_command(&args[1..]),
        "duplication" => duplication_command(&args[1..]),
        "policy" => policy_command(&args[1..]),
        "doctor" => doctor_command(&args[1..]),
        "mcp" => match leadline::mcp::serve() {
            Ok(()) => Ok(ExitCode::SUCCESS),
            Err(error) => Err(CliError::internal(error.to_string())),
        },
        "skill" | "--skill" => {
            print!("{SKILL_TEXT}");
            Ok(ExitCode::SUCCESS)
        }
        "version" | "--version" | "-V" => {
            println!("leadline {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        "--help" | "-h" | "help" => {
            println!("{}", usage());
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(CliError::usage(format!(
            "unknown command '{command}'\n\n{}",
            usage()
        ))),
    }
}

fn analyze_command(args: &[String]) -> Result<ExitCode, CliError> {
    let options = CommonOptions::parse(args, "analyze")?;
    let config = load_config_for(&options.path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let report = analyze_with_cache(
        &options.path,
        options.coverage.as_ref(),
        excludes,
        options.cache_dir.as_deref(),
    )?;
    if report.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            options.path.display()
        )));
    }
    options.print_report(&report, &config_thresholds(&config))?;
    Ok(ExitCode::SUCCESS)
}

fn function_command(args: &[String]) -> Result<ExitCode, CliError> {
    if args.len() < 2 || args[0].starts_with('-') || args[1].starts_with('-') {
        return Err(CliError::usage("function requires FILE and NAME"));
    }
    let file = PathBuf::from(&args[0]);
    let name = &args[1];
    let mut rest = Vec::new();
    let mut explain = false;
    for arg in &args[2..] {
        if arg == "--explain" {
            explain = true;
        } else {
            rest.push(arg.clone());
        }
    }
    let options = CommonOptions::parse_with_path(&rest, file.clone(), "function")?;
    // Config loads from the containing directory; explicit file paths still analyze.
    let _ = load_config_for(&file)?;
    let mut analyzed = leadline::analyze_file(&file, &config_dir(&file), options.coverage.as_ref())
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    analyzed.path = leadline::normalize_path(&file);
    analyzed.functions.retain(|function| function.name == *name);
    if analyzed.functions.is_empty() {
        return Err(CliError::usage(format!(
            "function '{name}' was not found in {}",
            file.display()
        )));
    }
    let report = AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        metric_specs: MetricSpecs::default(),
        files: vec![analyzed],
    };
    if explain && options.agent_json {
        let mut value = leadline::agent::analyze_agent_json_budgeted(&report, &options.budget);
        inject_contributions(&mut value, &report);
        println!(
            "{}",
            serde_json::to_string(&value).map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        options.print_report(&report, &Thresholds::default())?;
    }
    if explain && !options.agent_json && !options.json {
        for file in &report.files {
            for function in &file.functions {
                for contribution in &function.contributions {
                    println!(
                        "  {} line {} nesting {} +{} cog +{} cyc",
                        contribution.rule,
                        contribution.line,
                        contribution.nesting,
                        contribution.cognitive,
                        contribution.cyclomatic
                    );
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Copy per-function `contributions` into an agent-json payload, matched by
/// file path, function name, and start line.
fn inject_contributions(value: &mut serde_json::Value, report: &AnalysisReport) {
    let Some(files) = value
        .get_mut("files")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for file_value in files.iter_mut() {
        let path = file_value
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(functions) = file_value
            .get_mut("functions")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for function_value in functions.iter_mut() {
            let name = function_value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let line = function_value
                .get("line")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default() as u32;
            let contributions = report
                .files
                .iter()
                .filter(|file| file.path == path)
                .flat_map(|file| &file.functions)
                .find(|function| function.name == name && function.start_line == line)
                .map(|function| &function.contributions);
            if let Some(contributions) = contributions {
                function_value["contributions"] =
                    serde_json::to_value(contributions).unwrap_or(serde_json::Value::Null);
            }
        }
    }
}

fn changed_command(command: &str, args: &[String]) -> Result<ExitCode, CliError> {
    let mut base = None;
    let mut json = false;
    let mut agent_json = false;
    let mut path = PathBuf::from(".");
    let mut budget = Budget::default();
    let mut has_budget = false;
    let mut staged = false;
    let mut target = None;
    let mut renames = false;
    let mut explain = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--explain" => explain = true,
            "--staged" => staged = true,
            "--renames" => renames = true,
            "--target" => {
                index += 1;
                target = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--target requires a revision"))?
                        .clone(),
                );
            }
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                if value != "agent-json" {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'agent-json'"
                    )));
                }
                agent_json = true;
            }
            "--base" => {
                index += 1;
                base = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--base requires a revision"))?
                        .clone(),
                );
            }
            "--path" => {
                index += 1;
                path = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--path requires a path"))?,
                );
            }
            "--top" => {
                index += 1;
                budget.top = Some(parse_top(args.get(index), "--top")?);
                has_budget = true;
            }
            "--sort-by" => {
                index += 1;
                budget.sort_by = Some(parse_sort_key(args.get(index))?);
                has_budget = true;
            }
            "--min-crap" => {
                index += 1;
                budget.min_crap = Some(parse_floor(args.get(index), "--min-crap")?);
                has_budget = true;
            }
            "--min-delta" => {
                index += 1;
                budget.min_delta = Some(parse_floor(args.get(index), "--min-delta")?);
                has_budget = true;
            }
            value if !value.starts_with('-') && command == "diff" && base.is_none() => {
                base = Some(value.to_owned());
            }
            value => {
                return Err(CliError::usage(format!("unknown changed option '{value}'")));
            }
        }
        index += 1;
    }
    if json && agent_json {
        return Err(CliError::usage(
            "--json and --format agent-json are exclusive",
        ));
    }
    if has_budget && !agent_json {
        return Err(CliError::usage(
            "budget flags (--top, --sort-by, --min-crap, --min-delta) require --format agent-json",
        ));
    }
    let comparison_target = match (staged, target) {
        (true, Some(_)) => {
            return Err(CliError::usage("--staged and --target are exclusive"));
        }
        (true, None) => leadline::diff::ComparisonTarget::Index,
        (false, Some(revision)) => leadline::diff::ComparisonTarget::Revision(revision),
        (false, None) => leadline::diff::ComparisonTarget::Worktree,
    };
    let options = leadline::diff::ChangeOptions {
        base: base.unwrap_or_else(|| "HEAD~1".to_owned()),
        target: comparison_target,
        detect_renames: renames,
    };
    let report = leadline::diff::analyze_changes(&path, &options)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    budget.explain = explain;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::changed_agent_json_budgeted(
                &report, &budget
            ))
            .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        let terminal = leadline::report::terminal_changed(&report);
        if terminal.is_empty() {
            println!("No changed functions or parse errors found.");
        } else {
            print!("{terminal}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Ranked complexity-x-churn hotspots with the underlying dimensions exposed.
fn hotspots_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut limit = 10_usize;
    let mut window = HistoryWindow::Days90;
    let mut json = false;
    let mut agent_json = false;
    let mut coverage = CoverageMap::default();
    let mut has_coverage = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                if value != "agent-json" {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'agent-json'"
                    )));
                }
                agent_json = true;
            }
            "--limit" => {
                index += 1;
                limit = parse_top(args.get(index), "--limit")?;
            }
            "--since" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--since requires a window"))?;
                window = HistoryWindow::parse(value).ok_or_else(|| {
                    CliError::usage(format!(
                        "unknown --since '{value}': expected '30d', '90d', or '365d'"
                    ))
                })?;
            }
            "--lcov" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--lcov requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Lcov)?);
                has_coverage = true;
            }
            "--jacoco" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--jacoco requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Jacoco)?);
                has_coverage = true;
            }
            "--coverage" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--coverage requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Auto)?);
                has_coverage = true;
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown hotspots option '{value}'"
                )));
            }
        }
        index += 1;
    }
    if json && agent_json {
        return Err(CliError::usage(
            "--json and --format agent-json are exclusive",
        ));
    }
    let config = load_config_for(&path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let coverage = has_coverage.then_some(coverage);
    let (analysis, scope) = if path.is_file() {
        let scope = config_dir(&path);
        let analyzed = leadline::analyze_file(&path, &scope, coverage.as_ref())
            .map_err(|error| CliError::incomplete(error.to_string()))?;
        (
            AnalysisReport {
                schema_version: OUTPUT_SCHEMA_VERSION,
                metric_profile: METRIC_PROFILE,
                analyzer_version: env!("CARGO_PKG_VERSION"),
                metric_specs: MetricSpecs::default(),
                files: vec![analyzed],
            },
            scope,
        )
    } else {
        let analyzed = analyze_with_cache(&path, coverage.as_ref(), excludes, None)?;
        (analyzed, path.clone())
    };
    if analysis.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            path.display()
        )));
    }
    let history = leadline::history::analyze_history(&scope)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    let report = leadline::hotspots::build(&analysis, &history, window, limit);
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::hotspots_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        print!("{}", leadline::report::terminal_hotspots(&report));
    }
    Ok(ExitCode::SUCCESS)
}

/// Explainable change-risk ranking with exposed components.
fn risk_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut limit = 10_usize;
    let mut window = HistoryWindow::Days90;
    let mut json = false;
    let mut agent_json = false;
    let mut coverage = CoverageMap::default();
    let mut has_coverage = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                if value != "agent-json" {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'agent-json'"
                    )));
                }
                agent_json = true;
            }
            "--limit" => {
                index += 1;
                limit = parse_top(args.get(index), "--limit")?;
            }
            "--since" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--since requires a window"))?;
                window = HistoryWindow::parse(value).ok_or_else(|| {
                    CliError::usage(format!(
                        "unknown --since '{value}': expected '30d', '90d', or '365d'"
                    ))
                })?;
            }
            "--lcov" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--lcov requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Lcov)?);
                has_coverage = true;
            }
            "--jacoco" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--jacoco requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Jacoco)?);
                has_coverage = true;
            }
            "--coverage" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--coverage requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Auto)?);
                has_coverage = true;
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!("unknown risk option '{value}'")));
            }
        }
        index += 1;
    }
    if json && agent_json {
        return Err(CliError::usage(
            "--json and --format agent-json are exclusive",
        ));
    }
    let config = load_config_for(&path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let coverage = has_coverage.then_some(coverage);
    let (analysis, scope) = if path.is_file() {
        let scope = config_dir(&path);
        let analyzed = leadline::analyze_file(&path, &scope, coverage.as_ref())
            .map_err(|error| CliError::incomplete(error.to_string()))?;
        (
            AnalysisReport {
                schema_version: OUTPUT_SCHEMA_VERSION,
                metric_profile: METRIC_PROFILE,
                analyzer_version: env!("CARGO_PKG_VERSION"),
                metric_specs: MetricSpecs::default(),
                files: vec![analyzed],
            },
            scope,
        )
    } else {
        let analyzed = analyze_with_cache(&path, coverage.as_ref(), excludes, None)?;
        (analyzed, path.clone())
    };
    if analysis.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            path.display()
        )));
    }
    let graph = leadline::graph::analyze_dependencies(&scope, excludes)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    // ponytail: snapshot entries are loaded only for repo context (root,
    // config, mailmap); analysis still goes through the cache above.
    let (snapshot_context, snapshot) =
        leadline::source_snapshot::load(&path, SnapshotTarget::Worktree)
            .map_err(|error| CliError::incomplete(error.to_string()))?;
    let revision = snapshot.commit.clone().unwrap_or_else(|| "HEAD".to_owned());
    let (history, touches) = match snapshot_context.repo_root {
        Some(_) => {
            let git = leadline::history::analyze_git_at_with_mailmap(
                &snapshot_context,
                &revision,
                snapshot.mailmap_bytes.as_deref(),
            )
            .map_err(|error| CliError::incomplete(error.to_string()))?;
            (git.history, git.touches)
        }
        None => (
            leadline::history::HistoryReport {
                schema_version: leadline::history::HISTORY_SCHEMA_VERSION,
                analyzer_version: env!("CARGO_PKG_VERSION"),
                available: false,
                reference: "head-commit-time",
                head_commit: None,
                head_timestamp: None,
                files: Vec::new(),
            },
            Vec::new(),
        ),
    };
    let source_paths: Vec<String> = analysis
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    let ownership =
        leadline::ownership::build(&touches, &source_paths, OwnershipMode::AggregateOnly);
    let policy = leadline::policy::evaluate(&graph, &snapshot_context.config.architecture_rules);
    let mut report =
        leadline::risk::build(&analysis, &history, &graph, &ownership, &policy, window);
    if agent_json {
        report.risks.truncate(limit);
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::risk_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        let total = report.risks.len();
        report.risks.truncate(limit);
        print!("{}", leadline::report::terminal_risk(&report));
        if report.risks.len() < total {
            println!(
                "Showing the top {} of {} files (raise --limit for more).\n",
                report.risks.len(),
                total
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Canonical project build across every analytics section.
fn project_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut json = false;
    let mut agent_json = false;
    let mut window = HistoryWindow::Days90;
    let mut target = SnapshotTarget::Worktree;
    let mut mutation_inputs = Vec::new();
    let mut test_maps = Vec::new();
    let mut snapshots_path: Option<PathBuf> = None;
    let mut ownership_mode: Option<OwnershipMode> = None;
    let mut coverage = CoverageMap::default();
    let mut has_coverage = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                if value != "agent-json" {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'agent-json'"
                    )));
                }
                agent_json = true;
            }
            "--lcov" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--lcov requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Lcov)?);
                has_coverage = true;
            }
            "--jacoco" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--jacoco requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Jacoco)?);
                has_coverage = true;
            }
            "--coverage" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--coverage requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Auto)?);
                has_coverage = true;
            }
            "--since" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--since requires a window"))?;
                window = HistoryWindow::parse(value).ok_or_else(|| {
                    CliError::usage(format!(
                        "unknown --since '{value}': expected '30d', '90d', or '365d'"
                    ))
                })?;
            }
            "--target" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--target requires a revision"))?;
                target = SnapshotTarget::Revision(value.clone());
            }
            "--pit" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--pit requires a file"))?;
                mutation_inputs.push(MutationInput::Pit(PathBuf::from(value)));
            }
            "--stryker" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--stryker requires a file"))?;
                mutation_inputs.push(MutationInput::Stryker(PathBuf::from(value)));
            }
            "--test-map" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--test-map requires a file"))?;
                test_maps.push(PathBuf::from(value));
            }
            "--snapshots" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--snapshots requires a file"))?;
                snapshots_path = Some(PathBuf::from(value));
            }
            "--include-authors" => {
                set_ownership_mode(&mut ownership_mode, OwnershipMode::IncludeAuthors)?
            }
            "--anonymize-authors" => {
                set_ownership_mode(&mut ownership_mode, OwnershipMode::AnonymizeAuthors)?
            }
            "--exclude-git-identities" => {
                set_ownership_mode(&mut ownership_mode, OwnershipMode::AggregateOnly)?
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!("unknown project option '{value}'")));
            }
        }
        index += 1;
    }
    if json && agent_json {
        return Err(CliError::usage(
            "--json and --format agent-json are exclusive",
        ));
    }
    let request = leadline::analytics::ProjectRequest {
        path,
        target,
        window,
        mutation_inputs,
        test_maps,
        ownership_mode: ownership_mode.unwrap_or(OwnershipMode::AggregateOnly),
        snapshots_path,
        coverage: has_coverage.then_some(coverage),
    };
    let report = leadline::analytics::build(&request)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::project_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        print!("{}", leadline::report::terminal_project(&report));
    }
    Ok(ExitCode::SUCCESS)
}

fn set_ownership_mode(
    mode: &mut Option<OwnershipMode>,
    value: OwnershipMode,
) -> Result<(), CliError> {
    if mode.is_some() {
        return Err(CliError::usage(
            "--include-authors, --anonymize-authors, and --exclude-git-identities are exclusive",
        ));
    }
    *mode = Some(value);
    Ok(())
}

/// Full-state debt and risk comparison against a base revision.
fn debt_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut base = "HEAD~1".to_owned();
    let mut target = SnapshotTarget::Worktree;
    let mut target_flagged = false;
    let mut staged = false;
    let mut detect_renames = false;
    let mut fail_on_regression = false;
    let mut json = false;
    let mut agent_json = false;
    let mut window = HistoryWindow::Days90;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                if value != "agent-json" {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'agent-json'"
                    )));
                }
                agent_json = true;
            }
            "--base" => {
                index += 1;
                base = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--base requires a revision"))?
                    .clone();
            }
            "--staged" => {
                if target_flagged {
                    return Err(CliError::usage("--staged and --target are exclusive"));
                }
                staged = true;
                target = SnapshotTarget::Index;
            }
            "--target" => {
                if staged {
                    return Err(CliError::usage("--staged and --target are exclusive"));
                }
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--target requires a revision"))?;
                target_flagged = true;
                target = SnapshotTarget::Revision(value.clone());
            }
            "--renames" => detect_renames = true,
            "--fail-on-regression" => fail_on_regression = true,
            "--since" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--since requires a window"))?;
                window = HistoryWindow::parse(value).ok_or_else(|| {
                    CliError::usage(format!(
                        "unknown --since '{value}': expected '30d', '90d', or '365d'"
                    ))
                })?;
            }
            "--path" => {
                index += 1;
                path = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--path requires a path"))?,
                );
            }
            value => {
                return Err(CliError::usage(format!("unknown debt option '{value}'")));
            }
        }
        index += 1;
    }
    if json && agent_json {
        return Err(CliError::usage(
            "--json and --format agent-json are exclusive",
        ));
    }
    let request = leadline::analytics::DebtRequest {
        path,
        base,
        target,
        detect_renames,
        window,
        fail_on_regression,
    };
    let report = leadline::analytics::analyze_debt(&request)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::debt_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        print!("{}", leadline::report::terminal_debt(&report));
    }
    if fail_on_regression && (report.summary.new > 0 || report.summary.risk_increased > 0) {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// Captures one HEAD-keyed trend point into a snapshot store.
fn snapshot_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut output: Option<PathBuf> = None;
    let mut window = HistoryWindow::Days90;
    let mut replace = false;
    let mut mutation_inputs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                index += 1;
                output =
                    Some(PathBuf::from(args.get(index).ok_or_else(|| {
                        CliError::usage("--output requires a file")
                    })?));
            }
            "--since" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--since requires a window"))?;
                window = HistoryWindow::parse(value).ok_or_else(|| {
                    CliError::usage(format!(
                        "unknown --since '{value}': expected '30d', '90d', or '365d'"
                    ))
                })?;
            }
            "--replace" => replace = true,
            "--pit" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--pit requires a file"))?;
                mutation_inputs.push(MutationInput::Pit(PathBuf::from(value)));
            }
            "--stryker" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--stryker requires a file"))?;
                mutation_inputs.push(MutationInput::Stryker(PathBuf::from(value)));
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown snapshot option '{value}'"
                )));
            }
        }
        index += 1;
    }
    let output = output.ok_or_else(|| CliError::usage("snapshot requires --output FILE"))?;
    let outcome =
        leadline::analytics::capture_trend(&path, &output, window, &mutation_inputs, replace)
            .map_err(|error| CliError::incomplete(error.to_string()))?;
    let label = match outcome {
        leadline::snapshots::SnapshotOutcome::Added => "added",
        leadline::snapshots::SnapshotOutcome::Unchanged => "unchanged",
        leadline::snapshots::SnapshotOutcome::Replaced => "replaced",
    };
    println!("snapshot: {label} {}", output.display());
    Ok(ExitCode::SUCCESS)
}

/// Normalizes PIT/Stryker mutation reports and optional test maps.
fn mutation_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut json = false;
    let mut mutation_inputs = Vec::new();
    let mut test_maps = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--pit" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--pit requires a file"))?;
                mutation_inputs.push(MutationInput::Pit(PathBuf::from(value)));
            }
            "--stryker" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--stryker requires a file"))?;
                mutation_inputs.push(MutationInput::Stryker(PathBuf::from(value)));
            }
            "--test-map" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--test-map requires a file"))?;
                test_maps.push(PathBuf::from(value));
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown mutation option '{value}'"
                )));
            }
        }
        index += 1;
    }
    if mutation_inputs.is_empty() && test_maps.is_empty() {
        return Err(CliError::usage(
            "mutation requires at least one --pit/--stryker/--test-map input",
        ));
    }
    let (mutation, relationships) = leadline::analytics::analyze_mutation(
        &path,
        SnapshotTarget::Worktree,
        &mutation_inputs,
        &test_maps,
    )
    .map_err(|error| CliError::input(error.to_string()))?;
    if json {
        let envelope = serde_json::json!({
            "mutation": mutation,
            "test_relationships": relationships,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&envelope)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        print!(
            "{}",
            leadline::report::terminal_mutation(&mutation, relationships.as_ref())
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Token-clone detection, optionally with `new`/`existing`/`resolved` drift.
fn duplication_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut base: Option<String> = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--base" => {
                index += 1;
                base = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--base requires a revision"))?
                        .clone(),
                );
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown duplication option '{value}'"
                )));
            }
        }
        index += 1;
    }
    let outcome =
        leadline::analytics::analyze_duplication(&path, SnapshotTarget::Worktree, base.as_deref())
            .map_err(|error| CliError::incomplete(error.to_string()))?;
    let complete = match &outcome {
        leadline::analytics::DuplicationOutcome::Single(report) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(report)
                        .map_err(|error| CliError::internal(error.to_string()))?
                );
            } else {
                print!("{}", leadline::report::terminal_duplication(report));
            }
            report.complete
        }
        leadline::analytics::DuplicationOutcome::Drift(report) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(report)
                        .map_err(|error| CliError::internal(error.to_string()))?
                );
            } else {
                print!("{}", leadline::report::terminal_duplication_drift(report));
            }
            report.complete
        }
    };
    if !complete {
        return Ok(ExitCode::from(3));
    }
    Ok(ExitCode::SUCCESS)
}

/// Architecture-policy evaluation with optional drift and a gate.
fn policy_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut base: Option<String> = None;
    let mut json = false;
    let mut fail_on_violation = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--fail-on-violation" => fail_on_violation = true,
            "--base" => {
                index += 1;
                base = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--base requires a revision"))?
                        .clone(),
                );
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!("unknown policy option '{value}'")));
            }
        }
        index += 1;
    }
    let report =
        leadline::analytics::analyze_policy(&path, SnapshotTarget::Worktree, base.as_deref())
            .map_err(|error| CliError::incomplete(error.to_string()))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        print!("{}", leadline::report::terminal_policy(&report));
    }
    if fail_on_violation {
        let current_errors = report
            .violations
            .iter()
            .filter(|violation| {
                violation.severity == leadline::config::Severity::Error
                    && violation.status != Some(leadline::policy::PolicyStatus::Resolved)
            })
            .count();
        if current_errors > 0 {
            return Ok(ExitCode::from(1));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Historically related files for one target, from co-change history.
fn coupling_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut target: Option<String> = None;
    let mut root = PathBuf::from(".");
    let mut top = 20_usize;
    let mut min_co_changes = 2_u64;
    let mut json = false;
    let mut agent_json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                if value != "agent-json" {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'agent-json'"
                    )));
                }
                agent_json = true;
            }
            "--top" => {
                index += 1;
                top = parse_top(args.get(index), "--top")?;
            }
            "--min-cochanges" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--min-cochanges requires a count"))?;
                let parsed: u64 = raw
                    .parse()
                    .map_err(|_| CliError::usage("--min-cochanges requires a positive integer"))?;
                if parsed == 0 {
                    return Err(CliError::usage("--min-cochanges must be at least 1"));
                }
                min_co_changes = parsed;
            }
            "--path" => {
                index += 1;
                root = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--path requires a directory"))?,
                );
            }
            value if !value.starts_with('-') && target.is_none() => {
                target = Some(value.to_owned());
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown coupling option '{value}'"
                )));
            }
        }
        index += 1;
    }
    if json && agent_json {
        return Err(CliError::usage(
            "--json and --format agent-json are exclusive",
        ));
    }
    let Some(target) = target else {
        return Err(CliError::usage("coupling requires a TARGET file"));
    };
    if !root.is_dir() {
        return Err(CliError::usage(format!(
            "coupling scope '{}' is not a directory",
            root.display()
        )));
    }
    let key = resolve_scoped_target(&root, &target, "coupling")?;
    let options = CouplingOptions {
        min_co_changes,
        limit: top,
    };
    let report = leadline::coupling::analyze_coupling(&root, &key, &options)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::coupling_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        print!("{}", leadline::report::terminal_coupling(&report));
    }
    Ok(ExitCode::SUCCESS)
}

/// Resolve a user-supplied target to a scope-relative normalized path.
///
/// Validates the file exists, then canonicalizes both sides so symlinks and
/// Windows verbatim prefixes cannot split the join. Shared by `coupling` and
/// `impact` so canonical containment validation is implemented once.
fn resolve_scoped_target(root: &Path, target: &str, command: &str) -> Result<String, CliError> {
    let requested = Path::new(target);
    let absolute_target = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    if !absolute_target.is_file() {
        return Err(CliError::usage(format!(
            "{command} target '{target}' was not found under {}",
            root.display()
        )));
    }
    // Join keys are scope-relative; canonicalize both sides so symlinks and
    // Windows verbatim prefixes cannot split the join.
    let canonical_root =
        std::fs::canonicalize(root).map_err(|error| CliError::incomplete(error.to_string()))?;
    let canonical_target = std::fs::canonicalize(&absolute_target)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(CliError::usage(format!(
            "{command} target '{target}' is outside the scope {}",
            root.display()
        )));
    }
    Ok(leadline::normalized_relative_path(
        &canonical_target,
        &canonical_root,
    ))
}

/// Static dependency graph with fan-in/fan-out and cycles.
fn dependencies_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut json = false;
    let mut agent_json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                if value != "agent-json" {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'agent-json'"
                    )));
                }
                agent_json = true;
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown dependencies option '{value}'"
                )));
            }
        }
        index += 1;
    }
    if json && agent_json {
        return Err(CliError::usage(
            "--json and --format agent-json are exclusive",
        ));
    }
    let config = load_config_for(&path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let report = leadline::graph::analyze_dependencies(&path, excludes)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if report.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            path.display()
        )));
    }
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::dependencies_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        print!("{}", leadline::report::terminal_dependencies(&report));
    }
    Ok(ExitCode::SUCCESS)
}

/// Transitive dependents and blast radius for one target file.
fn impact_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut target: Option<String> = None;
    let mut root = PathBuf::from(".");
    let mut top = 20_usize;
    let mut json = false;
    let mut agent_json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                if value != "agent-json" {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'agent-json'"
                    )));
                }
                agent_json = true;
            }
            "--top" => {
                index += 1;
                top = parse_top(args.get(index), "--top")?;
            }
            "--path" => {
                index += 1;
                root = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--path requires a directory"))?,
                );
            }
            value if !value.starts_with('-') && target.is_none() => {
                target = Some(value.to_owned());
            }
            value => {
                return Err(CliError::usage(format!("unknown impact option '{value}'")));
            }
        }
        index += 1;
    }
    if json && agent_json {
        return Err(CliError::usage(
            "--json and --format agent-json are exclusive",
        ));
    }
    let Some(target) = target else {
        return Err(CliError::usage("impact requires a TARGET file"));
    };
    if !root.is_dir() {
        return Err(CliError::usage(format!(
            "impact scope '{}' is not a directory",
            root.display()
        )));
    }
    let key = resolve_scoped_target(&root, &target, "impact")?;
    let config = load_config_for(&root)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let graph = leadline::graph::analyze_dependencies(&root, excludes)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    let Some(report) = leadline::impact::analyze_impact(&graph, &key, top) else {
        return Err(CliError::usage(format!(
            "impact target '{target}' was not found in the dependency graph under {}",
            root.display()
        )));
    };
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::impact_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        print!("{}", leadline::report::terminal_impact(&report));
    }
    Ok(ExitCode::SUCCESS)
}

/// Parse `--top N` / `--limit N`; zero keeps nothing, so it is a usage error.
fn parse_top(raw: Option<&String>, flag: &str) -> Result<usize, CliError> {
    let raw = raw.ok_or_else(|| CliError::usage(format!("{flag} requires a count")))?;
    let top: usize = raw
        .parse()
        .map_err(|_| CliError::usage(format!("{flag} requires a positive integer")))?;
    if top == 0 {
        return Err(CliError::usage(format!("{flag} must be at least 1")));
    }
    Ok(top)
}

/// Parse `--sort-by crap|cognitive|cyclomatic`.
fn parse_sort_key(raw: Option<&String>) -> Result<SortKey, CliError> {
    let raw = raw.ok_or_else(|| CliError::usage("--sort-by requires a key"))?;
    SortKey::parse(raw).ok_or_else(|| {
        CliError::usage(format!(
            "unknown --sort-by '{raw}': expected one of 'crap', 'cognitive', 'cyclomatic'"
        ))
    })
}

/// Parse `--min-crap X` / `--min-delta D` filter floors.
fn parse_floor(raw: Option<&String>, flag: &str) -> Result<f64, CliError> {
    let raw = raw.ok_or_else(|| CliError::usage(format!("{flag} requires a limit")))?;
    let value: f64 = raw
        .parse()
        .map_err(|_| CliError::usage(format!("{flag} requires a numeric limit")))?;
    if !value.is_finite() {
        return Err(CliError::usage(format!(
            "{flag} requires a finite numeric limit"
        )));
    }
    Ok(value)
}

/// Config thresholds as engine thresholds for SARIF; absent config means empty.
fn config_thresholds(config: &Option<Config>) -> Thresholds {
    let Some(selected) = config else {
        return Thresholds::default();
    };
    // ponytail: field copy; From impl if a third Thresholds-shaped type appears.
    Thresholds {
        cognitive: selected.thresholds.cognitive,
        cyclomatic: selected.thresholds.cyclomatic,
        crap: selected.thresholds.crap,
        max_nesting: selected.thresholds.max_nesting,
    }
}

/// Fill gaps from `leadline.toml`; explicit CLI flags win.
fn apply_config(thresholds: &mut Thresholds, config: &Config) {
    if thresholds.cognitive.is_none() {
        thresholds.cognitive = config.thresholds.cognitive;
    }
    if thresholds.cyclomatic.is_none() {
        thresholds.cyclomatic = config.thresholds.cyclomatic;
    }
    if thresholds.crap.is_none() {
        thresholds.crap = config.thresholds.crap;
    }
    if thresholds.max_nesting.is_none() {
        thresholds.max_nesting = config.thresholds.max_nesting;
    }
}

fn check_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut thresholds = Thresholds::default();
    let mut base: Option<String> = None;
    let mut baseline: Option<String> = None;
    let mut regressions = false;
    let mut common = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--base" {
            index += 1;
            base = Some(
                args.get(index)
                    .ok_or_else(|| CliError::usage("--base requires a revision"))?
                    .clone(),
            );
            index += 1;
            continue;
        }
        if args[index] == "--baseline" {
            index += 1;
            baseline = Some(
                args.get(index)
                    .ok_or_else(|| CliError::usage("--baseline requires a file"))?
                    .clone(),
            );
            index += 1;
            continue;
        }
        if args[index] == "--regressions" {
            regressions = true;
            index += 1;
            continue;
        }
        let target = match args[index].as_str() {
            "--cognitive" => Some(&mut thresholds.cognitive),
            "--cyclomatic" => Some(&mut thresholds.cyclomatic),
            "--max-nesting" => Some(&mut thresholds.max_nesting),
            "--crap" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--crap requires a limit"))?;
                let value: f64 = raw
                    .parse()
                    .map_err(|_| CliError::usage("--crap requires a numeric limit"))?;
                if !value.is_finite() || value < 0.0 {
                    return Err(CliError::usage(
                        "--crap requires a finite, non-negative limit",
                    ));
                }
                thresholds.crap = Some(value);
                None
            }
            _ => {
                common.push(args[index].clone());
                None
            }
        };
        if let Some(target) = target {
            index += 1;
            let raw = args
                .get(index)
                .ok_or_else(|| CliError::usage("metric threshold requires a limit"))?;
            *target = Some(
                raw.parse()
                    .map_err(|_| CliError::usage("metric threshold requires a numeric limit"))?,
            );
        }
        index += 1;
    }
    if regressions && base.is_none() && baseline.is_none() {
        return Err(CliError::usage(
            "--regressions requires --base REV or --baseline FILE",
        ));
    }
    if base.is_some() && baseline.is_some() {
        return Err(CliError::usage("--base and --baseline are exclusive"));
    }
    if thresholds.is_empty() && !regressions {
        return Err(CliError::usage(
            "check requires at least one metric threshold (e.g. --cognitive 15 --cyclomatic 10 --crap 30 --max-nesting 5), or --regressions with --base/--baseline",
        ));
    }

    let options = CommonOptions::parse(&common, "check")?;
    let config = load_config_for(&options.path)?;
    if let Some(config) = &config {
        apply_config(&mut thresholds, config);
    }
    let regression_limits = config
        .as_ref()
        .map(|selected| selected.regressions.clone())
        .unwrap_or_default();
    if let Some(base) = base {
        return check_changed(
            &options,
            &base,
            &thresholds,
            regressions.then_some(&regression_limits),
        );
    }
    if let Some(baseline) = baseline {
        return check_baseline(
            &options,
            &baseline,
            &thresholds,
            regressions.then_some(&regression_limits),
        );
    }
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let mut report = analyze_with_cache(
        &options.path,
        options.coverage.as_ref(),
        excludes,
        options.cache_dir.as_deref(),
    )?;
    if report.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            options.path.display()
        )));
    }
    report.files.retain_mut(|file| {
        file.functions
            .retain(|function| thresholds.violates(&function.metrics));
        !file.functions.is_empty() || !file.parse_errors.is_empty()
    });
    let has_findings = !report.files.is_empty();
    options.print_report_or(&report, has_findings, "No violations.", &thresholds)?;
    if has_findings {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// `check --base REV`: gate after-side absolute and optional delta violations.
fn check_changed(
    options: &CommonOptions,
    base: &str,
    thresholds: &Thresholds,
    regression_limits: Option<&RegressionLimits>,
) -> Result<ExitCode, CliError> {
    let mut changed = leadline::diff::analyze_changed(&options.path, base)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if let Some(coverage) = options.coverage.as_ref() {
        for change in &mut changed.functions {
            apply_coverage_to_side(coverage, &change.path, &mut change.before);
            apply_coverage_to_side(coverage, &change.path, &mut change.after);
        }
    }
    let mut grouped: BTreeMap<String, Vec<FunctionAnalysis>> = BTreeMap::new();
    for change in &changed.functions {
        let Some(after) = change.after.as_ref() else {
            continue;
        };
        let absolute = thresholds.violates(&after.metrics);
        let delta = regression_limits.is_some_and(|limits| {
            change
                .before
                .as_ref()
                .is_some_and(|before| leadline::diff::regression_violates(before, after, limits))
        });
        if absolute || delta {
            grouped
                .entry(change.path.clone())
                .or_default()
                .push(after.clone());
        }
    }
    let files: Vec<FileAnalysis> = grouped
        .into_iter()
        .filter_map(|(path, functions)| {
            let language = leadline::parser::detect_language(&path)?;
            Some(FileAnalysis {
                path,
                language,
                functions,
                parse_errors: Vec::new(),
            })
        })
        .collect();
    let report = AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        metric_specs: MetricSpecs::default(),
        files,
    };
    let has_findings = !report.files.is_empty();
    let mut output_thresholds = thresholds.clone();
    if let Some(limits) = regression_limits {
        for change in &changed.functions {
            let Some((before, after)) = change.before.as_ref().zip(change.after.as_ref()) else {
                continue;
            };
            let dimensions = leadline::diff::regression_dimensions(before, after, limits);
            if dimensions[0] && output_thresholds.cognitive.is_none() {
                output_thresholds.cognitive = Some(0);
            }
            if dimensions[1] && output_thresholds.cyclomatic.is_none() {
                output_thresholds.cyclomatic = Some(0);
            }
            if dimensions[2] && output_thresholds.crap.is_none() {
                output_thresholds.crap = Some(0.0);
            }
            if dimensions[3] && output_thresholds.max_nesting.is_none() {
                output_thresholds.max_nesting = Some(0);
            }
        }
    }
    options.print_report_or(&report, has_findings, "No violations.", &output_thresholds)?;
    if has_findings {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// `check --baseline FILE`: gate current functions against a saved snapshot.
///
/// Paired functions fail on absolute violations or (with `--regressions`)
/// positive deltas over the snapshot; new functions fail only on absolute
/// thresholds; deleted functions are ignored.
fn check_baseline(
    options: &CommonOptions,
    baseline_path: &str,
    thresholds: &Thresholds,
    regression_limits: Option<&RegressionLimits>,
) -> Result<ExitCode, CliError> {
    let baseline = leadline::baseline::Baseline::read(Path::new(baseline_path))
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    let config = load_config_for(&options.path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let report = analyze_with_cache(
        &options.path,
        options.coverage.as_ref(),
        excludes,
        options.cache_dir.as_deref(),
    )?;
    if report.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            options.path.display()
        )));
    }
    let regressions = match regression_limits {
        Some(limits) => baseline.compare(&report, limits),
        None => Vec::new(),
    };
    let regressed: BTreeSet<(String, String)> = regressions
        .iter()
        .map(|finding| (finding.path.clone(), finding.id.clone()))
        .collect();
    let mut output_thresholds = thresholds.clone();
    if let Some(limits) = regression_limits {
        for finding in &regressions {
            let dimensions = leadline::diff::regression_dimensions(
                &finding.before.to_analysis(),
                &finding.after,
                limits,
            );
            if dimensions[0] && output_thresholds.cognitive.is_none() {
                output_thresholds.cognitive = Some(0);
            }
            if dimensions[1] && output_thresholds.cyclomatic.is_none() {
                output_thresholds.cyclomatic = Some(0);
            }
            if dimensions[2] && output_thresholds.crap.is_none() {
                output_thresholds.crap = Some(0.0);
            }
            if dimensions[3] && output_thresholds.max_nesting.is_none() {
                output_thresholds.max_nesting = Some(0);
            }
        }
    }
    let mut filtered = report;
    filtered.files.retain_mut(|file| {
        file.functions.retain(|function| {
            thresholds.violates(&function.metrics)
                || (regression_limits.is_some()
                    && regressed.contains(&(file.path.clone(), function.id.clone())))
        });
        !file.functions.is_empty() || !file.parse_errors.is_empty()
    });
    let has_findings = !filtered.files.is_empty();
    options.print_report_or(
        &filtered,
        has_findings,
        "No violations.",
        &output_thresholds,
    )?;
    if has_findings {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// `baseline PATH --output FILE`: snapshot current function metrics.
///
/// Writes schema-versioned JSON atomically (sibling temp file, then
/// rename). Review the file, commit it, and gate later edits with
/// `check --baseline FILE --regressions`.
fn baseline_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut seen_path = false;
    let mut output: Option<String> = None;
    let mut coverage = CoverageMap::default();
    let mut has_coverage = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                index += 1;
                output = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--output requires a file"))?
                        .clone(),
                );
            }
            "--lcov" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--lcov requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Lcov)?);
                has_coverage = true;
            }
            "--jacoco" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--jacoco requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Jacoco)?);
                has_coverage = true;
            }
            "--coverage" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--coverage requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Auto)?);
                has_coverage = true;
            }
            value if !value.starts_with('-') && !seen_path => {
                path = PathBuf::from(value);
                seen_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown baseline option '{value}'"
                )));
            }
        }
        index += 1;
    }
    let Some(output) = output else {
        return Err(CliError::usage("baseline requires --output FILE"));
    };
    let config = load_config_for(&path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let report = analyze_with_cache(&path, has_coverage.then_some(&coverage), excludes, None)?;
    if report.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            path.display()
        )));
    }
    let baseline = leadline::baseline::Baseline::from_report(&report);
    let count = baseline.functions.len();
    baseline
        .write(Path::new(&output))
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    println!("Wrote {count} functions to {output}");
    Ok(ExitCode::SUCCESS)
}

fn apply_coverage_to_side(coverage: &CoverageMap, path: &str, side: &mut Option<FunctionAnalysis>) {
    if let Some(function) = side.as_mut() {
        coverage.apply_function(path, function);
    }
}

/// Default cap for `test-targets` rows, mirroring the MCP truncation bound so
/// agent output stays bounded even without `--top`.
const TEST_TARGETS_MAX: usize = 200;

fn test_targets_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut seen_path = false;
    let mut agent_json = false;
    let mut top: Option<usize> = None;
    let mut coverage = CoverageMap::default();
    let mut has_coverage = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" => {
                index += 1;
                match args.get(index).map(String::as_str) {
                    Some("agent-json") => agent_json = true,
                    Some(value) => {
                        return Err(CliError::usage(format!(
                            "unknown --format '{value}': expected 'agent-json'"
                        )));
                    }
                    None => return Err(CliError::usage("--format requires a value")),
                }
            }
            "--top" => {
                index += 1;
                top = Some(parse_top(args.get(index), "--top")?);
            }
            "--lcov" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--lcov requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Lcov)?);
                has_coverage = true;
            }
            "--jacoco" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--jacoco requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Jacoco)?);
                has_coverage = true;
            }
            "--coverage" => {
                index += 1;
                let file = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--coverage requires a file"))?;
                coverage.merge(load_coverage(file, CoverageFormat::Auto)?);
                has_coverage = true;
            }
            value if !value.starts_with('-') && !seen_path => {
                path = PathBuf::from(value);
                seen_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown test-targets option '{value}'"
                )));
            }
        }
        index += 1;
    }
    if !has_coverage {
        return Err(CliError::usage(
            "test-targets requires coverage: pass --coverage, --lcov, or --jacoco",
        ));
    }
    let config = load_config_for(&path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let report = analyze_with_cache(&path, Some(&coverage), excludes, None)?;
    if report.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            path.display()
        )));
    }
    let targets = leadline::test_targets::test_targets(&report, &coverage);
    let total = targets.len();
    let limit = top.unwrap_or(TEST_TARGETS_MAX).min(total);
    let truncated = total > limit;
    let kept = &targets[..limit];
    if agent_json {
        let value = serde_json::json!({
            "schema_version": OUTPUT_SCHEMA_VERSION,
            "analyzer_version": env!("CARGO_PKG_VERSION"),
            "metric_profile": METRIC_PROFILE,
            "tool": "test-targets",
            "path": path.display().to_string(),
            "targets": kept,
            "truncated": truncated,
            "total": total,
        });
        println!(
            "{}",
            serde_json::to_string(&value).map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if kept.is_empty() {
        println!("No test targets: no known-zero-hit decision lines.");
    } else {
        for target in kept {
            println!(
                "{}:{} {} (crap {}, coverage {}): {} uncovered, {} unknown",
                target.path,
                target.line,
                target.function,
                target
                    .crap
                    .map_or("n/a".to_owned(), |crap| format!("{crap:.1}")),
                target.coverage.map_or("n/a".to_owned(), |coverage| format!(
                    "{:.0}%",
                    coverage * 100.0
                )),
                target.uncovered.len(),
                target.unknown.len()
            );
        }
        if truncated {
            println!("... truncated: showing {limit} of {total} (use --top N)");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn doctor_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut seen_path = false;
    for arg in args {
        if arg.starts_with('-') {
            return Err(CliError::usage(format!("unknown doctor option '{arg}'")));
        }
        if seen_path {
            return Err(CliError::usage("doctor takes at most one path"));
        }
        path = PathBuf::from(arg);
        seen_path = true;
    }

    let mut incomplete = false;
    for (language, file, snippet) in [
        (
            "java",
            "Doctor.java",
            "class Doctor { void check() { int x = 1; } }",
        ),
        (
            "javascript",
            "doctor.js",
            "function check(x) { if (x) { return 1; } return 0; }",
        ),
        (
            "typescript",
            "doctor.ts",
            "function check(x: boolean): number { if (x) { return 1; } return 0; }",
        ),
        (
            "tsx",
            "doctor.tsx",
            "function check(x: boolean): number { if (x) { return 1; } return 0; }",
        ),
    ] {
        let outcome = match leadline::analyze_source(file, snippet.as_bytes()) {
            Ok(report) if !report.functions.is_empty() => {
                Ok(Some(format!("{} function", report.functions.len())))
            }
            Ok(_) => Err(String::from("no functions found")),
            Err(error) => Err(error.to_string()),
        };
        report_doctor(&format!("parser: {language}"), outcome, &mut incomplete);
    }

    report_doctor(
        "coverage: lcov",
        CoverageMap::from_lcov("TN:\nSF:src/doctor.ts\nDA:1,1\nend_of_record\n")
            .map(|_| None)
            .map_err(|error| error.to_string()),
        &mut incomplete,
    );
    report_doctor(
        "coverage: jacoco",
        CoverageMap::from_jacoco_xml(
            r#"<?xml version="1.0"?><report><package name="com/example"><sourcefile name="Doctor.java"><line nr="1" mi="0" ci="1"/></sourcefile></package></report>"#,
        )
        .map(|_| None)
        .map_err(|error| error.to_string()),
        &mut incomplete,
    );

    let git = std::process::Command::new("git").arg("--version").output();
    report_doctor(
        "git:",
        match git {
            Ok(output) if output.status.success() => Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            )),
            _ => Err(String::from("git executable unavailable")),
        },
        &mut incomplete,
    );
    let dir = config_dir(&path);
    match config::load_from(&dir) {
        Ok(None) => println!("config: OK (no leadline.toml in {})", dir.display()),
        Ok(Some(_)) => println!("config: OK (leadline.toml in {} is valid)", dir.display()),
        Err(error) => {
            println!("config: INVALID ({error})");
            return Err(CliError::usage(format!("invalid leadline.toml: {error}")));
        }
    }

    let mut coverage_inputs = Vec::new();
    for candidate in [
        "coverage/lcov.info",
        "lcov.info",
        "coverage.xml",
        "jacoco.xml",
    ] {
        if dir.join(candidate).is_file() {
            coverage_inputs.push(candidate);
        }
    }
    if coverage_inputs.is_empty() {
        println!("harness: no coverage inputs found (informational)");
    } else {
        println!(
            "harness: coverage inputs present: {} (informational)",
            coverage_inputs.join(", ")
        );
    }

    if incomplete {
        return Err(CliError::incomplete("doctor self-check failed"));
    }
    Ok(ExitCode::SUCCESS)
}

/// Print one `doctor` self-check line; failures flip the incomplete flag.
fn report_doctor(label: &str, outcome: Result<Option<String>, String>, incomplete: &mut bool) {
    match outcome {
        Ok(Some(detail)) => println!("{label} OK ({detail})"),
        Ok(None) => println!("{label} OK"),
        Err(detail) => {
            println!("{label} FAILED ({detail})");
            *incomplete = true;
        }
    }
}

/// Containing directory for path-based lookups: the directory itself,
/// else the parent of a file path, else ".".
fn config_dir(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    }
}

/// Analysis root for `leadline.toml`: the directory itself, or the parent of a file path.
fn load_config_for(path: &Path) -> Result<Option<Config>, CliError> {
    let dir = config_dir(path);
    let dir = if dir.is_dir() {
        dir
    } else {
        PathBuf::from(".")
    };
    Ok(config::load_from(&dir)?)
}

/// Analyze with an optional [`leadline::cache::FileCache`].
///
/// Coverage merges into metrics after analysis, so cached (pre-coverage)
/// functions would carry stale CRAP: any coverage input bypasses the cache.
/// Cache I/O failures warn on stderr and fall back to a fresh analysis.
fn analyze_with_cache(
    path: &Path,
    coverage: Option<&CoverageMap>,
    excludes: &[String],
    cache_dir: Option<&Path>,
) -> Result<AnalysisReport, CliError> {
    let Some(dir) = cache_dir.filter(|_| coverage.is_none()) else {
        return leadline::analyze_path_with_excludes(path, coverage, excludes)
            .map_err(|error| CliError::incomplete(error.to_string()));
    };
    let mut cache = leadline::cache::FileCache::open(dir);
    let mut files = Vec::new();
    if path.is_file() {
        let bytes = std::fs::read(path).map_err(|error| CliError::incomplete(error.to_string()))?;
        let key = leadline::normalize_path(path);
        match cache
            .get(&key, &bytes)
            .and_then(|hit| cached_file(&key, hit))
        {
            Some(hit) => files.push(hit),
            None => {
                let analyzed = leadline::analyze_source(&key, &bytes)
                    .map_err(|error| CliError::incomplete(error.to_string()))?;
                if analyzed.parse_errors.is_empty() {
                    cache.put(&key, &bytes, analyzed.functions.clone());
                }
                files.push(analyzed);
            }
        }
    } else {
        let discovered = leadline::discovery::discover_with_excludes(path, excludes)
            .map_err(|error| CliError::incomplete(error.to_string()))?;
        for file in &discovered {
            let bytes =
                std::fs::read(file).map_err(|error| CliError::incomplete(error.to_string()))?;
            let key = leadline::normalized_relative_path(file, path);
            match cache
                .get(&key, &bytes)
                .and_then(|hit| cached_file(&key, hit))
            {
                Some(hit) => files.push(hit),
                None => {
                    let analyzed = leadline::analyze_file(file, path, None)
                        .map_err(|error| CliError::incomplete(error.to_string()))?;
                    if analyzed.parse_errors.is_empty() {
                        cache.put(&key, &bytes, analyzed.functions.clone());
                    }
                    files.push(analyzed);
                }
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
    }
    if let Err(error) = cache.save(dir) {
        eprintln!(
            "leadline: cache warning: cannot save cache in {}: {error}",
            dir.display()
        );
    }
    Ok(AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        metric_specs: MetricSpecs::default(),
        files,
    })
}

/// Rebuild a [`FileAnalysis`] from cached functions; `None` when the cached
/// path names no supported language (falls back to a fresh analysis).
fn cached_file(key: &str, functions: Vec<FunctionAnalysis>) -> Option<FileAnalysis> {
    let language = leadline::parser::detect_language(key)?;
    Some(FileAnalysis {
        path: key.to_owned(),
        language,
        functions,
        parse_errors: Vec::new(),
    })
}

struct CommonOptions {
    path: PathBuf,
    json: bool,
    agent_json: bool,
    sarif: bool,
    pretty: bool,
    coverage: Option<CoverageMap>,
    budget: Budget,
    cache_dir: Option<PathBuf>,
}

impl CommonOptions {
    fn parse(args: &[String], command: &str) -> Result<Self, CliError> {
        Self::parse_with_path(args, PathBuf::from("."), command)
    }

    fn parse_with_path(
        args: &[String],
        mut path: PathBuf,
        command: &str,
    ) -> Result<Self, CliError> {
        let mut json = false;
        let mut agent_json = false;
        let mut sarif = false;
        let mut coverage = CoverageMap::default();
        let mut has_coverage = false;
        let mut has_path = path != Path::new(".");
        let mut budget = Budget::default();
        let mut has_budget = false;
        let mut cache_dir: Option<PathBuf> = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--json" => json = true,
                "--format" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| CliError::usage("--format requires a value"))?;
                    match value.as_str() {
                        "agent-json" => agent_json = true,
                        "sarif" => sarif = true,
                        _ => {
                            return Err(CliError::usage(format!(
                                "unknown --format '{value}': expected 'agent-json' or 'sarif'"
                            )));
                        }
                    }
                }
                "--top" => {
                    index += 1;
                    budget.top = Some(parse_top(args.get(index), "--top")?);
                    has_budget = true;
                }
                "--sort-by" => {
                    index += 1;
                    budget.sort_by = Some(parse_sort_key(args.get(index))?);
                    has_budget = true;
                }
                "--min-crap" => {
                    index += 1;
                    budget.min_crap = Some(parse_floor(args.get(index), "--min-crap")?);
                    has_budget = true;
                }
                "--cache-dir" => {
                    index += 1;
                    cache_dir =
                        Some(PathBuf::from(args.get(index).ok_or_else(|| {
                            CliError::usage("--cache-dir requires a directory")
                        })?));
                }
                "--lcov" => {
                    index += 1;
                    let file = args
                        .get(index)
                        .ok_or_else(|| CliError::usage("--lcov requires a file"))?;
                    coverage.merge(load_coverage(file, CoverageFormat::Lcov)?);
                    has_coverage = true;
                }
                "--jacoco" => {
                    index += 1;
                    let file = args
                        .get(index)
                        .ok_or_else(|| CliError::usage("--jacoco requires a file"))?;
                    coverage.merge(load_coverage(file, CoverageFormat::Jacoco)?);
                    has_coverage = true;
                }
                "--coverage" => {
                    index += 1;
                    let file = args
                        .get(index)
                        .ok_or_else(|| CliError::usage("--coverage requires a file"))?;
                    coverage.merge(load_coverage(file, CoverageFormat::Auto)?);
                    has_coverage = true;
                }
                value if !value.starts_with('-') && !has_path => {
                    path = PathBuf::from(value);
                    has_path = true;
                }
                value => {
                    return Err(CliError::usage(format!(
                        "unknown {command} option '{value}'"
                    )));
                }
            }
            index += 1;
        }
        if json && agent_json {
            return Err(CliError::usage(
                "--json and --format agent-json are exclusive",
            ));
        }
        if json && sarif {
            return Err(CliError::usage("--json and --format sarif are exclusive"));
        }
        if agent_json && sarif {
            return Err(CliError::usage(
                "--format agent-json and --format sarif are exclusive",
            ));
        }
        if sarif && command != "analyze" && command != "check" {
            return Err(CliError::usage(format!(
                "--format sarif is only supported on analyze and check, not {command}"
            )));
        }
        if has_budget && !agent_json {
            return Err(CliError::usage(
                "budget flags (--top, --sort-by, --min-crap) require --format agent-json",
            ));
        }
        if cache_dir.is_some() && command != "analyze" && command != "check" {
            return Err(CliError::usage(format!(
                "--cache-dir is only supported on analyze and check, not {command}"
            )));
        }
        Ok(Self {
            path,
            json,
            agent_json,
            sarif,
            // `analyze --json` stays pretty-printed; other commands stay compact.
            pretty: command == "analyze",
            coverage: has_coverage.then_some(coverage),
            budget,
            cache_dir,
        })
    }

    /// JSON output keeps its per-command encoding; agent-json and SARIF stay compact.
    fn print_report(
        &self,
        report: &AnalysisReport,
        thresholds: &Thresholds,
    ) -> Result<(), CliError> {
        self.print_report_or(report, true, "", thresholds)
    }

    fn print_report_or(
        &self,
        report: &AnalysisReport,
        has_findings: bool,
        empty_text: &str,
        thresholds: &Thresholds,
    ) -> Result<(), CliError> {
        if self.agent_json {
            println!(
                "{}",
                serde_json::to_string(&leadline::agent::analyze_agent_json_budgeted(
                    report,
                    &self.budget
                ))
                .map_err(|error| CliError::internal(error.to_string()))?
            );
        } else if self.sarif {
            println!(
                "{}",
                serde_json::to_string(&leadline::sarif::analysis_to_sarif(report, thresholds))
                    .map_err(|error| CliError::internal(error.to_string()))?
            );
        } else if self.json {
            let encoded = if self.pretty {
                serde_json::to_string_pretty(report)
            } else {
                serde_json::to_string(report)
            };
            println!(
                "{}",
                encoded.map_err(|error| CliError::internal(error.to_string()))?
            );
        } else if has_findings {
            let terminal = leadline::report::terminal(report);
            if terminal.is_empty() {
                println!("No supported functions or parse errors found.");
            } else {
                print!("{terminal}");
            }
        } else {
            println!("{empty_text}");
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum CoverageFormat {
    Lcov,
    Jacoco,
    Auto,
}

fn load_coverage(path: &str, format: CoverageFormat) -> Result<CoverageMap, CliError> {
    let is_lcov = match format {
        CoverageFormat::Lcov => true,
        CoverageFormat::Jacoco => false,
        CoverageFormat::Auto if path.ends_with(".info") => true,
        CoverageFormat::Auto if path.ends_with(".xml") => false,
        CoverageFormat::Auto => {
            return Err(CliError::coverage(format!(
                "cannot detect coverage format for '{path}': use .info for LCOV or .xml for JaCoCo"
            )));
        }
    };
    if is_lcov {
        let text = std::fs::read_to_string(path).map_err(|error| {
            CliError::coverage(format!("cannot read LCOV file '{path}': {error}"))
        })?;
        CoverageMap::from_lcov(&text).map_err(|error| {
            CliError::coverage(format!("cannot parse LCOV file '{path}': {error}"))
        })
    } else {
        let text = std::fs::read_to_string(path).map_err(|error| {
            CliError::coverage(format!("cannot read JaCoCo file '{path}': {error}"))
        })?;
        CoverageMap::from_jacoco_xml(&text).map_err(|error| {
            CliError::coverage(format!("cannot parse JaCoCo file '{path}': {error}"))
        })
    }
}

fn usage() -> &'static str {
    "Usage:\n  leadline analyze [PATH] [--json] [--format agent-json|sarif] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--cache-dir DIR]\n  leadline function FILE NAME [--json] [--format agent-json] [--explain] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline check [PATH] [--base REV | --baseline FILE] [--regressions] [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N] [--json] [--format agent-json|sarif] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--cache-dir DIR]\n  leadline changed [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--min-delta D]\n  leadline diff [REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--min-delta D]\n  leadline hotspots [PATH] [--limit N] [--since 30d|90d|365d] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
  leadline risk [PATH] [--limit N] [--since 30d|90d|365d] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline coupling TARGET [--path ROOT] [--top N] [--min-cochanges N] [--json] [--format agent-json]
  leadline dependencies [PATH] [--json] [--format agent-json]
  leadline impact TARGET [--path ROOT] [--top N] [--json] [--format agent-json]\n  leadline project [PATH] [--target REV] [--since 30d|90d|365d] [--pit FILE]... [--stryker FILE]... [--test-map FILE]... [--snapshots FILE] [--include-authors | --anonymize-authors | --exclude-git-identities] [--json] [--format agent-json]\n  leadline debt [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--since 30d|90d|365d] [--fail-on-regression] [--json] [--format agent-json]\n  leadline snapshot [PATH] --output FILE [--since 30d|90d|365d] [--pit FILE]... [--stryker FILE]... [--replace]\n  leadline mutation [PATH] (--pit FILE | --stryker FILE)... [--test-map FILE]... [--json]\n  leadline duplication [PATH] [--base REV] [--json]\n  leadline policy [PATH] [--base REV] [--fail-on-violation] [--json]\n  leadline doctor [PATH]\n  leadline test-targets [PATH] (--coverage FILE | --lcov FILE | --jacoco FILE) [--top N] [--format agent-json]\n  leadline baseline [PATH] --output FILE [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline mcp\n  leadline skill\n  leadline version\n  leadline --version"
}
