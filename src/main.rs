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
use std::time::Instant;

/// CLI failure carrying a stable exit code.
///
/// Codes: 0 success / gate passed, 1 gate failed (returned as [`ExitCode`],
/// never an error), 2 usage or config error, 3 incomplete analysis or a
/// failed update, 4 external input error (coverage, scanner, SQL, and plan
/// artifacts), 5 internal error.
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
    #[cfg(windows)]
    leadline::update::cleanup_stale_backup();
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("leadline: {}", error.message);
            ExitCode::from(error.code)
        }
    }
}

/// Runs one command with local metrics recorded when they are enabled.
///
/// Telemetry is best-effort and never changes the outcome. `mcp` is skipped
/// here because its duration spans the whole server session; its tool calls
/// are recorded individually. CPU time and resident memory are sampled for the
/// duration of the command when metrics are enabled.
fn run(args: Vec<String>) -> Result<ExitCode, CliError> {
    leadline::telemetry::arm();
    let operation = args
        .first()
        .map_or("none", |raw| cli_operation(raw))
        .to_owned();
    let started = Instant::now();
    // `mcp` serves for the whole session, so its cost is sampled per tool call
    // rather than around the server loop.
    let sampler =
        (operation != "mcp").then(|| leadline::telemetry::Sampler::start("cli", &operation));
    let result = dispatch_command(args);
    if let Some(sampler) = sampler {
        let cost = sampler.finish();
        let outcome = outcome_label(&result);
        leadline::telemetry::record_process_cost("cli", &operation, outcome, &cost);
        leadline::telemetry::record_invocation("cli", &operation, outcome, started.elapsed());
    }
    result
}

/// Every command [`dispatch_command`] accepts, so metric labels stay bounded.
const CLI_OPERATIONS: [&str; 31] = [
    "analyze",
    "function",
    "changed",
    "diff",
    "check",
    "baseline",
    "hotspots",
    "risk",
    "security",
    "coupling",
    "dependencies",
    "unused",
    "report",
    "impact",
    "test-targets",
    "project",
    "debt",
    "snapshot",
    "mutation",
    "duplication",
    "policy",
    "sql-plan",
    "sql",
    "vulnerabilities",
    "doctor",
    "update",
    "mcp",
    "index",
    "skill",
    "version",
    "help",
];

/// Bounded operation label for one raw first argument.
fn cli_operation(raw: &str) -> &str {
    match raw {
        "--version" | "-V" => "version",
        "--skill" => "skill",
        "--help" | "-h" => "help",
        other if CLI_OPERATIONS.contains(&other) => other,
        _ => "other",
    }
}

/// Coarse outcome label for local metrics, mirroring the exit-code contract.
fn outcome_label(result: &Result<ExitCode, CliError>) -> &'static str {
    match result {
        Ok(code) if *code == ExitCode::SUCCESS => "success",
        Ok(code) if *code == ExitCode::from(1) => "gate_failed",
        Ok(code) if *code == ExitCode::from(3) => "incomplete",
        Ok(_) => "other",
        Err(error) => match error.code {
            2 => "usage_error",
            3 => "incomplete",
            4 => "input_error",
            _ => "internal_error",
        },
    }
}

/// Encodes one JSON document straight to stdout instead of building the whole
/// encoded string in memory first.
fn print_json<T: serde::Serialize>(value: &T, pretty: bool) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let encoded = if pretty {
        serde_json::to_writer_pretty(&mut output, value)
    } else {
        serde_json::to_writer(&mut output, value)
    };
    encoded.map_err(|error| CliError::internal(error.to_string()))?;
    use std::io::Write;
    output
        .write_all(b"\n")
        .map_err(|error| CliError::internal(error.to_string()))
}

fn dispatch_command(args: Vec<String>) -> Result<ExitCode, CliError> {
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
        "security" => security_command(&args[1..]),
        "coupling" => coupling_command(&args[1..]),
        "dependencies" => dependencies_command(&args[1..]),
        "report" => report_command(&args[1..]),
        "unused" => unused_command(&args[1..]),
        "impact" => impact_command(&args[1..]),
        "test-targets" => test_targets_command(&args[1..]),
        "project" => project_command(&args[1..]),
        "debt" => debt_command(&args[1..]),
        "snapshot" => snapshot_command(&args[1..]),
        "mutation" => mutation_command(&args[1..]),
        "duplication" => duplication_command(&args[1..]),
        "policy" => policy_command(&args[1..]),
        "sql-plan" => sql_plan_command(&args[1..]),
        "sql" => sql_command(&args[1..]),
        "vulnerabilities" => vulnerabilities_command(&args[1..]),
        "doctor" => doctor_command(&args[1..]),
        "update" => update_command(&args[1..]),
        "mcp" => mcp_command(&args[1..]),
        "index" => index_command(&args[1..]),
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

fn mcp_command(args: &[String]) -> Result<ExitCode, CliError> {
    let options = leadline::mcp::parse_mcp_args(args).map_err(CliError::usage)?;
    let result = match options {
        None => leadline::mcp::serve(),
        Some(http) => leadline::mcp::serve_http(&http.host, http.port),
    };
    match result {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(error) => Err(CliError::internal(error.to_string())),
    }
}

fn index_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path: Option<String> = None;
    let mut output: Option<String> = None;
    let mut verify = false;
    let mut json = false;
    let mut position = 0;
    while position < args.len() {
        match args[position].as_str() {
            "--output" => {
                position += 1;
                let value = args
                    .get(position)
                    .ok_or_else(|| CliError::usage("--output requires a value"))?;
                output = Some(value.clone());
            }
            "--verify" => verify = true,
            "--json" => json = true,
            value if !value.starts_with('-') && path.is_none() => path = Some(value.to_owned()),
            value => return Err(CliError::usage(format!("unknown index option '{value}'"))),
        }
        position += 1;
    }
    let root = PathBuf::from(path.unwrap_or_else(|| ".".to_owned()));
    if !root.is_dir() {
        return Err(CliError::usage(format!(
            "index requires a directory: {}",
            root.display()
        )));
    }
    let config = load_config_for(&root)?;
    let dir = match output {
        Some(dir) => PathBuf::from(dir),
        None => root.join(
            config
                .as_ref()
                .and_then(|config| config.index.as_ref())
                .map(|index| index.path.as_str())
                .unwrap_or(leadline::index::DEFAULT_INDEX_DIR),
        ),
    };
    let fingerprint = leadline::config::fingerprint(&config_dir(&root));
    let excludes: Vec<String> = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.clone())
        .unwrap_or_default();
    let outcome = leadline::index::build(&leadline::index::IndexRequest {
        root: &root,
        index_dir: &dir,
        scope: &leadline::index::scope_label(&root),
        config_fingerprint: &fingerprint,
        excludes: &excludes,
        verify,
    })
    .map_err(|error| CliError::incomplete(error.to_string()))?;
    let payload = serde_json::json!({
        "index": {
            "path": outcome.path.to_string_lossy(),
            "scope": outcome.index.scope,
            "files": outcome.index.files.len(),
            "analyzed": outcome.reuse.analyzed,
            "reused": outcome.reuse.reused,
            "history": outcome.index.history.is_some(),
            "head_commit": outcome.index.history.as_ref().and_then(|facts| facts.head_commit.clone()),
            "verified": outcome.verified,
        }
    });
    if json {
        println!(
            "{}",
            serde_json::to_string(&payload)
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else {
        let row = &payload["index"];
        println!(
            "index {}: {} files ({} analyzed, {} reused), history {}, verified {}",
            row["path"].as_str().unwrap_or_default(),
            row["files"],
            row["analyzed"],
            row["reused"],
            row["history"],
            row["verified"],
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn analyze_command(args: &[String]) -> Result<ExitCode, CliError> {
    let options = CommonOptions::parse(args, "analyze")?;
    let config = load_config_for(&options.path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let index_dir = resolve_index_dir(&options.path, options.index_dir.as_deref(), config.as_ref());
    let report = analyze_with_index(
        &options.path,
        options.coverage.as_ref(),
        excludes,
        index_dir.as_deref(),
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
        // The warm path is scoped to analyze and check in this phase.
        let analyzed = analyze_with_index(&path, coverage.as_ref(), excludes, None)?;
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
        print_json(&report, true)?;
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
    let analysis = if path.is_file() {
        let scope = config_dir(&path);
        let analyzed = leadline::analyze_file(&path, &scope, coverage.as_ref())
            .map_err(|error| CliError::incomplete(error.to_string()))?;
        AnalysisReport {
            schema_version: OUTPUT_SCHEMA_VERSION,
            metric_profile: METRIC_PROFILE,
            analyzer_version: env!("CARGO_PKG_VERSION"),
            metric_specs: MetricSpecs::default(),
            files: vec![analyzed],
        }
    } else {
        // The warm path is scoped to analyze and check in this phase.
        analyze_with_index(&path, coverage.as_ref(), excludes, None)?
    };
    if analysis.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            path.display()
        )));
    }
    let mut report = leadline::analytics::analyze_risk(&analysis, &path, excludes, window)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if agent_json {
        report.risks.truncate(limit);
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::risk_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        print_json(&report, true)?;
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
        print_json(&report, true)?;
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
    leadline::telemetry::record_debt("cli", &report.summary);
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::debt_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        print_json(&report, true)?;
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
        print_json(&envelope, true)?;
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
                print_json(report, true)?;
            } else {
                print!("{}", leadline::report::terminal_duplication(report));
            }
            report.complete
        }
        leadline::analytics::DuplicationOutcome::Drift(report) => {
            if json {
                print_json(report, true)?;
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
        print_json(&report, true)?;
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

/// PostgreSQL plan regression checks over checked-in EXPLAIN artifacts.
///
/// Reads raw `EXPLAIN (FORMAT JSON)` directories only; never connects to a
/// database or executes SQL. Malformed plans are input errors (exit 4);
/// gate violations print the full report and exit 1.
fn sql_plan_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut current: Option<String> = None;
    let mut baseline: Option<String> = None;
    let mut limits = leadline::pg_plan::PlanLimits::default();
    let mut json = false;
    let mut agent_json = false;
    let mut sarif = false;
    let mut ci_format: Option<leadline::render::CiFormat> = None;
    let mut top: Option<usize> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--current" => {
                index += 1;
                current = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--current requires a directory"))?
                        .clone(),
                );
            }
            "--baseline" => {
                index += 1;
                baseline = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--baseline requires a directory"))?
                        .clone(),
                );
            }
            "--max-cost-increase-percent" => {
                index += 1;
                limits.max_cost_increase_percent = Some(parse_plan_limit(
                    args.get(index),
                    "--max-cost-increase-percent",
                )?);
            }
            "--max-plan-rows-ratio" => {
                index += 1;
                limits.max_plan_rows_ratio =
                    Some(parse_plan_limit(args.get(index), "--max-plan-rows-ratio")?);
            }
            "--max-estimate-error-ratio" => {
                index += 1;
                limits.max_estimate_error_ratio = Some(parse_plan_limit(
                    args.get(index),
                    "--max-estimate-error-ratio",
                )?);
            }
            "--json" => json = true,
            "--format" => {
                index += 1;
                parse_scan_format(args.get(index), &mut agent_json, &mut sarif, &mut ci_format)?;
            }
            "--top" => {
                index += 1;
                top = Some(parse_top(args.get(index), "--top")?);
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown sql-plan option '{value}'"
                )));
            }
        }
        index += 1;
    }
    check_output_format(json, agent_json, sarif, top)?;
    let (Some(current), Some(baseline)) = (current, baseline) else {
        return Err(CliError::usage(
            "sql-plan requires --current DIR and --baseline DIR",
        ));
    };
    let report = leadline::pg_plan::compare_plan_directories(
        Path::new(&current),
        Path::new(&baseline),
        &limits,
    )
    .map_err(|error| CliError::input(error.to_string()))?;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::pg_plan::agent_json(
                &report,
                top.unwrap_or(leadline::security::AGENT_DEFAULT_TOP)
            ))
            .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if let Some(format) = ci_format {
        let document = serde_json::json!({ "sql_violations": &report.violations });
        print_ci_document(
            &document,
            0,
            &leadline::render::GateLimits::default(),
            format,
        )?;
    } else if sarif {
        println!(
            "{}",
            serde_json::to_string(&leadline::sarif::pg_plan_to_sarif(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        print_json(&report, true)?;
    } else {
        print!("{}", leadline::pg_plan::terminal_text(&report));
    }
    if report.violations.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

/// Security finding triage over checked-in SARIF artifacts.
///
/// Reads untrusted scanner output only; never executes scanners or touches
/// the network. Enrichment joins one project build against normalized
/// findings. Malformed SARIF is an input error (exit 4); gate violations
/// print the full report and exit 1.
fn security_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut sarifs: Vec<PathBuf> = Vec::new();
    let mut baseline_sarifs: Vec<PathBuf> = Vec::new();
    let mut base: Option<String> = None;
    let mut staged = false;
    let mut target: Option<String> = None;
    let mut fail_on_severity = None;
    let mut new_only = false;
    let mut changed_only = false;
    let mut json = false;
    let mut agent_json = false;
    let mut sarif = false;
    let mut ci_format: Option<leadline::render::CiFormat> = None;
    let mut top: Option<usize> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--sarif" => {
                index += 1;
                sarifs.push(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--sarif requires a file"))?,
                ));
            }
            "--baseline-sarif" => {
                index += 1;
                baseline_sarifs
                    .push(PathBuf::from(args.get(index).ok_or_else(|| {
                        CliError::usage("--baseline-sarif requires a file")
                    })?));
            }
            "--base" => {
                index += 1;
                base = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--base requires a revision"))?
                        .clone(),
                );
            }
            "--staged" => staged = true,
            "--target" => {
                index += 1;
                target = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--target requires a revision"))?
                        .clone(),
                );
            }
            "--fail-on-severity" => {
                index += 1;
                fail_on_severity =
                    Some(parse_severity_flag(args.get(index), "--fail-on-severity")?);
            }
            "--new-only" => new_only = true,
            "--changed-only" => changed_only = true,
            "--json" => json = true,
            "--format" => {
                index += 1;
                parse_scan_format(args.get(index), &mut agent_json, &mut sarif, &mut ci_format)?;
            }
            "--top" => {
                index += 1;
                top = Some(parse_top(args.get(index), "--top")?);
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown security option '{value}'"
                )));
            }
        }
        index += 1;
    }
    check_output_format(json, agent_json, sarif, top)?;
    if sarifs.is_empty() {
        return Err(CliError::usage(
            "security requires at least one --sarif FILE",
        ));
    }
    // One assembly path shared with `check` and the MCP tool: read, enrich,
    // gate. The CLI only chooses the request, then prints the outcome.
    let outcome = leadline::security::assemble(&leadline::security::SecurityRequest {
        path,
        sarif: sarifs,
        baseline_sarif: baseline_sarifs,
        comparison: comparison_from_flags(&base, staged, &target, changed_only)?,
        gate: fail_on_severity.map(|minimum| leadline::security::SecurityGate {
            minimum,
            new_only,
            changed_only,
        }),
    })
    .map_err(|error| match error {
        leadline::security::SecurityError::Input(message) => CliError::input(message),
        leadline::security::SecurityError::Incomplete(message) => CliError::incomplete(message),
    })?;
    let report = outcome.report;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::security::agent_json(
                &report,
                top.unwrap_or(leadline::security::AGENT_DEFAULT_TOP)
            ))
            .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if let Some(format) = ci_format {
        let document = serde_json::json!({ "security_violations": &report.findings });
        print_ci_document(
            &document,
            0,
            &leadline::render::GateLimits::default(),
            format,
        )?;
    } else if sarif {
        println!(
            "{}",
            serde_json::to_string(&leadline::sarif::security_findings_to_sarif(
                &report.findings,
            ))
            .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        print_json(&report, true)?;
    } else {
        print!("{}", leadline::security::terminal_text(&report));
    }
    if outcome.violations.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

/// Reject mutually exclusive revision selectors and build the comparison.
///
/// `changed_only` adds the gate-narrowing requirement: without a comparison
/// every finding is unchanged, so the combination is a usage error instead of
/// a silent pass.
fn comparison_from_flags(
    base: &Option<String>,
    staged: bool,
    target: &Option<String>,
    changed_only: bool,
) -> Result<Option<leadline::security::ChangeComparison>, CliError> {
    if base.is_some() && staged {
        return Err(CliError::usage("--base and --staged are exclusive"));
    }
    if staged && target.is_some() {
        return Err(CliError::usage("--staged and --target are exclusive"));
    }
    if base.is_some() && target.is_some() {
        return Err(CliError::usage("--base and --target are exclusive"));
    }
    if changed_only && base.is_none() && !staged && target.is_none() {
        return Err(CliError::usage(
            "--changed-only requires --base REV, --staged, or --target REV",
        ));
    }
    Ok(match (base, staged, target) {
        (None, false, None) => None,
        (Some(base), false, None) => Some(leadline::security::ChangeComparison::Base(base.clone())),
        (None, true, None) => Some(leadline::security::ChangeComparison::Staged),
        (None, false, Some(revision)) => Some(leadline::security::ChangeComparison::Target(
            revision.clone(),
        )),
        _ => unreachable!("exclusive comparison flags are rejected above"),
    })
}

/// Maps the gate's thresholds onto the renderer's limits.
fn gate_limits(thresholds: &Thresholds) -> leadline::render::GateLimits {
    leadline::render::GateLimits {
        cognitive: thresholds.cognitive.map(f64::from),
        cyclomatic: thresholds.cyclomatic.map(f64::from),
        max_nesting: thresholds.max_nesting.map(f64::from),
        crap: thresholds.crap,
    }
}

/// Renders one CI format from a check-shaped document, using the same limits
/// the gate applied so a live run and `report --from` agree byte for byte.
fn print_ci_document(
    document: &serde_json::Value,
    files: usize,
    limits: &leadline::render::GateLimits,
    format: leadline::render::CiFormat,
) -> Result<(), CliError> {
    let findings =
        leadline::render::findings_from_check_json(document, limits).map_err(CliError::usage)?;
    let summary = leadline::render::GateSummary {
        passed: findings.is_empty(),
        files,
        violations: findings.len(),
    };
    print!("{}", leadline::render::render(&findings, &summary, format));
    Ok(())
}

/// Parse one `--format` value into its output flags: the `agent-json` and
/// `sarif` projections, or one of the CI-facing renderers.
fn parse_scan_format(
    raw: Option<&String>,
    agent_json: &mut bool,
    sarif: &mut bool,
    ci_format: &mut Option<leadline::render::CiFormat>,
) -> Result<(), CliError> {
    let value = raw.ok_or_else(|| CliError::usage("--format requires a value"))?;
    match value.as_str() {
        "agent-json" => *agent_json = true,
        "sarif" => *sarif = true,
        _ => match leadline::render::parse_format(value) {
            Some(format) => *ci_format = Some(format),
            None => {
                return Err(CliError::usage(format!(
                    "unknown --format '{value}': expected 'agent-json', 'sarif', 'codeclimate', \
                     'gitlab-codequality', 'github-annotations', 'github-summary', 'markdown', \
                     'badge', or 'compact'"
                )));
            }
        },
    }
    Ok(())
}

/// Repeated scanner artifact flags shared by `vulnerabilities` and `check`.
#[derive(Default)]
struct VulnerabilityArtifactArgs {
    osv: Vec<PathBuf>,
    trivy: Vec<PathBuf>,
    baseline_osv: Vec<PathBuf>,
    baseline_trivy: Vec<PathBuf>,
}

/// Consume one `--osv`/`--trivy`/`--baseline-osv`/`--baseline-trivy` flag.
/// Returns `true` when `arg` matched so callers continue the loop.
fn parse_vulnerability_artifact(
    arg: &str,
    args: &[String],
    index: &mut usize,
    out: &mut VulnerabilityArtifactArgs,
) -> Result<bool, CliError> {
    let target = match arg {
        "--osv" => &mut out.osv,
        "--trivy" => &mut out.trivy,
        "--baseline-osv" => &mut out.baseline_osv,
        "--baseline-trivy" => &mut out.baseline_trivy,
        _ => return Ok(false),
    };
    *index += 1;
    target
        .push(PathBuf::from(args.get(*index).ok_or_else(|| {
            CliError::usage(format!("{arg} requires a file"))
        })?));
    Ok(true)
}

/// Vulnerable dependency triage over checked-in scanner artifacts.
///
/// Reads untrusted OSV-Scanner/Trivy output only; never queries advisory
/// services, registries, or package managers. Malformed reports are input
/// errors (exit 4); gate violations print the full report and exit 1.
fn vulnerabilities_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut artifacts = VulnerabilityArtifactArgs::default();
    let mut base: Option<String> = None;
    let mut staged = false;
    let mut target: Option<String> = None;
    let mut fail_on_severity = None;
    let mut json = false;
    let mut agent_json = false;
    let mut sarif = false;
    let mut ci_format: Option<leadline::render::CiFormat> = None;
    let mut top: Option<usize> = None;
    let mut index = 0;
    while index < args.len() {
        if parse_vulnerability_artifact(&args[index], args, &mut index, &mut artifacts)? {
            index += 1;
            continue;
        }
        match args[index].as_str() {
            "--base" => {
                index += 1;
                base = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--base requires a revision"))?
                        .clone(),
                );
            }
            "--staged" => staged = true,
            "--target" => {
                index += 1;
                target = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--target requires a revision"))?
                        .clone(),
                );
            }
            "--fail-on-severity" => {
                index += 1;
                fail_on_severity =
                    Some(parse_severity_flag(args.get(index), "--fail-on-severity")?);
            }
            "--json" => json = true,
            "--format" => {
                index += 1;
                parse_scan_format(args.get(index), &mut agent_json, &mut sarif, &mut ci_format)?;
            }
            "--top" => {
                index += 1;
                top = Some(parse_top(args.get(index), "--top")?);
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!(
                    "unknown vulnerabilities option '{value}'"
                )));
            }
        }
        index += 1;
    }
    check_output_format(json, agent_json, sarif, top)?;
    if artifacts.osv.is_empty() && artifacts.trivy.is_empty() {
        return Err(CliError::usage(
            "vulnerabilities requires at least one --osv or --trivy FILE",
        ));
    }
    let config = load_config_for(&path)?;
    let minimum = fail_on_severity.or(config
        .as_ref()
        .and_then(|selected| selected.vulnerabilities.minimum_severity));
    let comparison = comparison_from_flags(&base, staged, &target, false)?;
    let outcome =
        leadline::vulnerabilities::assemble(&leadline::vulnerabilities::VulnerabilityRequest {
            path: path.clone(),
            osv: artifacts.osv,
            trivy: artifacts.trivy,
            baseline_osv: artifacts.baseline_osv,
            baseline_trivy: artifacts.baseline_trivy,
            comparison,
            gate: minimum,
        })
        .map_err(|error| CliError::input(error.to_string()))?;
    let report = outcome.report;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::vulnerabilities::agent_json(
                &report,
                top.unwrap_or(leadline::security::AGENT_DEFAULT_TOP)
            ))
            .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if let Some(format) = ci_format {
        let document = serde_json::json!({ "vulnerability_violations": &report.findings });
        print_ci_document(
            &document,
            0,
            &leadline::render::GateLimits::default(),
            format,
        )?;
    } else if sarif {
        println!(
            "{}",
            serde_json::to_string(&leadline::sarif::vulnerability_findings_to_sarif(
                &report.findings,
            ))
            .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        print_json(&report, true)?;
    } else {
        print!("{}", leadline::vulnerabilities::terminal_text(&report));
    }
    if minimum.is_some() && !outcome.violations.is_empty() {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Static PostgreSQL risk analysis over `.sql` text and host call sites.
///
/// Parses text only; never executes SQL or connects to a database.
/// Malformed SQL is an input error (exit 4); gate violations print the
/// full report and exit 1.
fn sql_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut large_offset: Option<u64> = None;
    let mut migration_roots: Option<Vec<String>> = None;
    let mut fail_on_severity = None;
    let mut json = false;
    let mut agent_json = false;
    let mut sarif = false;
    let mut ci_format: Option<leadline::render::CiFormat> = None;
    let mut top: Option<usize> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--large-offset" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--large-offset requires a count"))?;
                let value: u64 = raw
                    .parse()
                    .map_err(|_| CliError::usage("--large-offset requires a positive integer"))?;
                if value == 0 {
                    return Err(CliError::usage("--large-offset must be at least 1"));
                }
                large_offset = Some(value);
            }
            "--migration-root" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--migration-root requires a directory"))?;
                let root = leadline::config::normalize_migration_root(raw).map_err(|error| {
                    CliError::usage(format!("invalid --migration-root: {error}"))
                })?;
                migration_roots.get_or_insert_with(Vec::new).push(root);
            }
            "--fail-on-severity" => {
                index += 1;
                fail_on_severity =
                    Some(parse_severity_flag(args.get(index), "--fail-on-severity")?);
            }
            "--json" => json = true,
            "--format" => {
                index += 1;
                parse_scan_format(args.get(index), &mut agent_json, &mut sarif, &mut ci_format)?;
            }
            "--top" => {
                index += 1;
                top = Some(parse_top(args.get(index), "--top")?);
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!("unknown sql option '{value}'")));
            }
        }
        index += 1;
    }
    check_output_format(json, agent_json, sarif, top)?;
    let config = load_config_for(&path)?;
    let mut sql_config = config
        .as_ref()
        .map(|selected| selected.sql.clone())
        .unwrap_or_default();
    if let Some(threshold) = large_offset {
        sql_config.large_offset = threshold;
    }
    if let Some(roots) = migration_roots {
        sql_config.migration_roots = roots;
    }
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let report = leadline::sql::analyze_sql_path(&path, &sql_config, excludes)
        .map_err(|error| CliError::input(error.to_string()))?;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::sql::agent_json(
                &report,
                top.unwrap_or(leadline::security::AGENT_DEFAULT_TOP)
            ))
            .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if let Some(format) = ci_format {
        let document = serde_json::json!({ "sql_violations": &report.findings });
        print_ci_document(
            &document,
            0,
            &leadline::render::GateLimits::default(),
            format,
        )?;
    } else if sarif {
        println!(
            "{}",
            serde_json::to_string(&leadline::sarif::sql_findings_to_sarif(&report.findings))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        print_json(&report, true)?;
    } else {
        print!("{}", leadline::sql::terminal_text(&report));
    }
    if fail_on_severity
        .is_some_and(|minimum| !leadline::sql::sql_gate_violations(&report, minimum).is_empty())
    {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
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
    let key = leadline::resolve_scoped_target(&root, &target).map_err(|error| match error {
        leadline::ScopedTargetError::Io(message) => CliError::incomplete(message),
        other => CliError::usage(other.to_string()),
    })?;
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
        print_json(&report, true)?;
    } else {
        print!("{}", leadline::report::terminal_coupling(&report));
    }
    Ok(ExitCode::SUCCESS)
}

/// Unused files, dependencies, and exports relative to declared entry points.
///
/// Informational: it reports candidates and never gates, because dynamic
/// access can hide a use that static reachability cannot see.
fn unused_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut path = PathBuf::from(".");
    let mut has_path = false;
    let mut json = false;
    let mut agent_json = false;
    let mut include_tests = false;
    let mut entries: Vec<String> = Vec::new();
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
            "--include-tests" => include_tests = true,
            "--entry" => {
                index += 1;
                entries.push(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--entry requires a pattern"))?
                        .clone(),
                );
            }
            value if !value.starts_with('-') && !has_path => {
                path = PathBuf::from(value);
                has_path = true;
            }
            value => {
                return Err(CliError::usage(format!("unknown unused option '{value}'")));
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
    let mut unused_config = config
        .as_ref()
        .map(|selected| selected.unused.clone())
        .unwrap_or_default();
    if include_tests {
        unused_config.include_tests = true;
    }
    let report = leadline::unused::analyze_unused(&path, excludes, &entries, &unused_config)
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if report.files_analyzed == 0 {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            path.display()
        )));
    }
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::unused_agent_json(&report))
                .map_err(|error| CliError::internal(error.to_string()))?
        );
    } else if json {
        print_json(&report, true)?;
    } else {
        print!("{}", leadline::report::terminal_unused(&report));
    }
    Ok(ExitCode::SUCCESS)
}

/// Re-render a saved `check --json` report through one CI format, without
/// re-analyzing. Threshold flags re-apply the gate's limits, because the
/// saved document carries metrics but not the reasons a row failed.
fn report_command(args: &[String]) -> Result<ExitCode, CliError> {
    let mut from: Option<PathBuf> = None;
    let mut format: Option<leadline::render::CiFormat> = None;
    let mut limits = leadline::render::GateLimits::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--from" => {
                index += 1;
                from = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| CliError::usage("--from requires a file"))?,
                ));
            }
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| CliError::usage("--format requires a value"))?;
                format = leadline::render::parse_format(value);
                if format.is_none() {
                    return Err(CliError::usage(format!(
                        "unknown --format '{value}': expected 'codeclimate', \
                         'gitlab-codequality', 'github-annotations', 'github-summary', \
                         'markdown', 'badge', or 'compact'"
                    )));
                }
            }
            "--cognitive" => {
                index += 1;
                limits.cognitive = Some(parse_floor(args.get(index), "--cognitive")?);
            }
            "--cyclomatic" => {
                index += 1;
                limits.cyclomatic = Some(parse_floor(args.get(index), "--cyclomatic")?);
            }
            "--max-nesting" => {
                index += 1;
                limits.max_nesting = Some(parse_floor(args.get(index), "--max-nesting")?);
            }
            "--crap" => {
                index += 1;
                limits.crap = Some(parse_floor(args.get(index), "--crap")?);
            }
            value => {
                return Err(CliError::usage(format!("unknown report option '{value}'")));
            }
        }
        index += 1;
    }
    let from = from.ok_or_else(|| CliError::usage("report requires --from FILE"))?;
    let format = format.ok_or_else(|| CliError::usage("report requires --format NAME"))?;
    let bytes = std::fs::read(&from)
        .map_err(|error| CliError::usage(format!("cannot read {}: {error}", from.display())))?;
    let document: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| CliError::usage(format!("{} is not JSON: {error}", from.display())))?;
    let files = document
        .get("files")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    print_ci_document(&document, files, &limits, format)?;
    Ok(ExitCode::SUCCESS)
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
        print_json(&report, true)?;
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
    let key = leadline::resolve_scoped_target(&root, &target).map_err(|error| match error {
        leadline::ScopedTargetError::Io(message) => CliError::incomplete(message),
        other => CliError::usage(other.to_string()),
    })?;
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
        print_json(&report, true)?;
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

/// Parse a non-negative finite `sql-plan` gate limit.
fn parse_plan_limit(raw: Option<&String>, flag: &str) -> Result<f64, CliError> {
    let value = parse_floor(raw, flag)?;
    if value < 0.0 {
        return Err(CliError::usage(format!("{flag} must be non-negative")));
    }
    Ok(value)
}

/// Parse `--fail-on-severity` / `--sql-fail-on-severity` gate levels.
fn parse_severity_flag(
    raw: Option<&String>,
    flag: &str,
) -> Result<leadline::security::SecuritySeverity, CliError> {
    let raw = raw.ok_or_else(|| CliError::usage(format!("{flag} requires a level")))?;
    leadline::security::parse_gate_severity(raw).ok_or_else(|| {
        CliError::usage(format!(
            "unknown {flag} '{raw}': expected 'low', 'medium', 'high', or 'critical'"
        ))
    })
}

/// Shared `--json` / `--format agent-json|sarif` exclusivity plus `--top` guard.
fn check_output_format(
    json: bool,
    agent_json: bool,
    sarif: bool,
    top: Option<usize>,
) -> Result<(), CliError> {
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
    if top.is_some() && !agent_json {
        return Err(CliError::usage("--top requires --format agent-json"));
    }
    Ok(())
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
    let mut sarifs: Vec<PathBuf> = Vec::new();
    let mut baseline_sarifs: Vec<PathBuf> = Vec::new();
    let mut fail_on_severity = None;
    let mut new_only = false;
    let mut changed_only = false;
    let mut sql_flag = false;
    let mut sql_fail_on_severity = None;
    let mut common = Vec::new();
    let mut index = 0;
    let mut artifacts = VulnerabilityArtifactArgs::default();
    while index < args.len() {
        let mut artifact_index = index;
        if parse_vulnerability_artifact(&args[index], args, &mut artifact_index, &mut artifacts)? {
            index = artifact_index + 1;
            continue;
        }
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
        if args[index] == "--sarif" {
            index += 1;
            sarifs.push(PathBuf::from(
                args.get(index)
                    .ok_or_else(|| CliError::usage("--sarif requires a file"))?,
            ));
            index += 1;
            continue;
        }
        if args[index] == "--baseline-sarif" {
            index += 1;
            baseline_sarifs
                .push(PathBuf::from(args.get(index).ok_or_else(|| {
                    CliError::usage("--baseline-sarif requires a file")
                })?));
            index += 1;
            continue;
        }
        if args[index] == "--fail-on-severity" {
            index += 1;
            fail_on_severity = Some(parse_severity_flag(args.get(index), "--fail-on-severity")?);
            index += 1;
            continue;
        }
        if args[index] == "--new-only" {
            new_only = true;
            index += 1;
            continue;
        }
        if args[index] == "--changed-only" {
            changed_only = true;
            index += 1;
            continue;
        }
        if args[index] == "--sql" {
            sql_flag = true;
            index += 1;
            continue;
        }
        if args[index] == "--sql-fail-on-severity" {
            index += 1;
            sql_fail_on_severity = Some(parse_severity_flag(
                args.get(index),
                "--sql-fail-on-severity",
            )?);
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
    if changed_only && base.is_none() {
        return Err(CliError::usage("--changed-only requires --base REV"));
    }
    let has_vuln_input = !artifacts.osv.is_empty() || !artifacts.trivy.is_empty();
    let options = CommonOptions::parse(&common, "check")?;
    let config = load_config_for(&options.path)?;
    if let Some(config) = &config {
        apply_config(&mut thresholds, config);
    }
    if thresholds.is_empty() && !regressions && sarifs.is_empty() && !has_vuln_input && !sql_flag {
        return Err(CliError::usage(
            "check requires at least one metric threshold (e.g. --cognitive 15 --cyclomatic 10 --crap 30 --max-nesting 5), a [thresholds.function] section in leadline.toml, --regressions with --base/--baseline, --sarif FILE, --osv/--trivy FILE, or --sql",
        ));
    }

    let regression_limits = config
        .as_ref()
        .map(|selected| selected.regressions.clone())
        .unwrap_or_default();
    let security = if sarifs.is_empty() {
        None
    } else {
        let gate = fail_on_severity.map(|minimum| leadline::security::SecurityGate {
            minimum,
            new_only,
            changed_only,
        });
        let comparison = base
            .as_ref()
            .map(|revision| leadline::security::ChangeComparison::Base(revision.clone()));
        Some(
            leadline::security::assemble(&leadline::security::SecurityRequest {
                path: options.path.clone(),
                sarif: sarifs,
                baseline_sarif: baseline_sarifs,
                comparison,
                gate,
            })
            .map_err(|error| CliError::input(error.message().to_owned()))?,
        )
    };
    let security_failed = security
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    let vulnerabilities = if has_vuln_input {
        let minimum = fail_on_severity.or(config
            .as_ref()
            .and_then(|selected| selected.vulnerabilities.minimum_severity));
        let comparison = base
            .as_ref()
            .map(|revision| leadline::security::ChangeComparison::Base(revision.clone()));
        Some(
            leadline::vulnerabilities::assemble(&leadline::vulnerabilities::VulnerabilityRequest {
                path: options.path.clone(),
                osv: artifacts.osv,
                trivy: artifacts.trivy,
                baseline_osv: artifacts.baseline_osv,
                baseline_trivy: artifacts.baseline_trivy,
                comparison,
                gate: minimum,
            })
            .map_err(|error| CliError::input(error.to_string()))?,
        )
    } else {
        None
    };
    let vulnerabilities_failed = vulnerabilities
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    let sql = if sql_flag {
        let sql_config = config
            .as_ref()
            .map(|selected| selected.sql.clone())
            .unwrap_or_default();
        let excludes: &[String] = config
            .as_ref()
            .map(|selected| selected.analysis_excludes.as_slice())
            .unwrap_or(&[]);
        let report = leadline::sql::analyze_sql_path(&options.path, &sql_config, excludes)
            .map_err(|error| CliError::input(error.to_string()))?;
        Some(leadline::sql::outcome(report, sql_fail_on_severity))
    } else {
        None
    };
    let sql_failed = sql
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    if let Some(base) = base {
        return check_changed(
            &options,
            &base,
            &thresholds,
            regressions.then_some(&regression_limits),
            security.as_ref(),
            vulnerabilities.as_ref(),
            sql.as_ref(),
        );
    }
    if let Some(baseline) = baseline {
        return check_baseline(
            &options,
            &baseline,
            &thresholds,
            regressions.then_some(&regression_limits),
            security.as_ref(),
            vulnerabilities.as_ref(),
            sql.as_ref(),
        );
    }
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let index_dir = resolve_index_dir(&options.path, options.index_dir.as_deref(), config.as_ref());
    let mut report = analyze_with_index(
        &options.path,
        options.coverage.as_ref(),
        excludes,
        index_dir.as_deref(),
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
    options.print_report_or_with_security(
        &report,
        ScannerReports {
            security: security.as_ref(),
            vulnerabilities: vulnerabilities.as_ref(),
            sql: sql.as_ref(),
        },
        has_findings,
        "No violations.",
        MetricGate {
            thresholds: &thresholds,
            regressions: &[],
        },
    )?;
    if has_findings || security_failed || vulnerabilities_failed || sql_failed {
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
    security: Option<&leadline::security::SecurityOutcome>,
    vulnerabilities: Option<&leadline::vulnerabilities::VulnerabilityOutcome>,
    sql: Option<&leadline::sql::SqlOutcome>,
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
    let regressions: Vec<leadline::sarif::FunctionViolation<'_>> = regression_limits
        .map(|limits| {
            changed
                .functions
                .iter()
                .filter_map(|change| {
                    let before = change.before.as_ref()?;
                    let after = change.after.as_ref()?;
                    Some(leadline::sarif::regression_violations(
                        before,
                        after,
                        limits,
                        &change.path,
                    ))
                })
                .flatten()
                .collect()
        })
        .unwrap_or_default();
    let security_failed = security
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    let vulnerabilities_failed = vulnerabilities
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    let sql_failed = sql
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    options.print_report_or_with_security(
        &report,
        ScannerReports {
            security,
            vulnerabilities,
            sql,
        },
        has_findings,
        "No violations.",
        MetricGate {
            thresholds,
            regressions: &regressions,
        },
    )?;
    if has_findings || security_failed || vulnerabilities_failed || sql_failed {
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
    security: Option<&leadline::security::SecurityOutcome>,
    vulnerabilities: Option<&leadline::vulnerabilities::VulnerabilityOutcome>,
    sql: Option<&leadline::sql::SqlOutcome>,
) -> Result<ExitCode, CliError> {
    let baseline = leadline::baseline::Baseline::read(Path::new(baseline_path))
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    let config = load_config_for(&options.path)?;
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let index_dir = resolve_index_dir(&options.path, options.index_dir.as_deref(), config.as_ref());
    // The warm path is scoped to analyze and check in this phase.
    let report = analyze_with_index(
        &options.path,
        options.coverage.as_ref(),
        excludes,
        index_dir.as_deref(),
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
    let violations: Vec<leadline::sarif::FunctionViolation<'_>> = regression_limits
        .map(|limits| {
            regressions
                .iter()
                .flat_map(|finding| {
                    leadline::sarif::regression_violations(
                        &finding.before.to_analysis(),
                        &finding.after,
                        limits,
                        &finding.path,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
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
    let vulnerabilities_failed = vulnerabilities
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    let security_failed = security
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    let sql_failed = sql
        .as_ref()
        .is_some_and(|outcome| !outcome.violations.is_empty());
    options.print_report_or_with_security(
        &filtered,
        ScannerReports {
            security,
            vulnerabilities,
            sql,
        },
        has_findings,
        "No violations.",
        MetricGate {
            thresholds,
            regressions: &violations,
        },
    )?;
    if has_findings || security_failed || vulnerabilities_failed || sql_failed {
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
    // The warm path is scoped to analyze and check in this phase.
    let report = analyze_with_index(&path, has_coverage.then_some(&coverage), excludes, None)?;
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
    // The warm path is scoped to analyze and check in this phase.
    let report = analyze_with_index(&path, Some(&coverage), excludes, None)?;
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
            "cpp",
            "doctor.cpp",
            "int check(bool value) { if (value) { return 1; } return 0; }",
        ),
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
        (
            "rust",
            "doctor.rs",
            "fn check(x: bool) -> i32 { if x { 1 } else { 0 } }",
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

fn update_command(args: &[String]) -> Result<ExitCode, CliError> {
    use leadline::update::IntegrationState;

    let mut integrations = false;
    for argument in args {
        match argument.as_str() {
            "--integrations" => integrations = true,
            _ => {
                return Err(CliError::usage(format!(
                    "unknown update option '{argument}'"
                )));
            }
        }
    }
    let base_url = std::env::var("LEADLINE_BASE_URL")
        .unwrap_or_else(|_| leadline::update::DEFAULT_BASE_URL.to_owned());
    match leadline::update::run(&base_url) {
        Ok(leadline::update::Outcome::Current(version)) => {
            println!("leadline {version} is the latest release.");
        }
        Ok(leadline::update::Outcome::Updated { from, to, path }) => {
            println!("leadline {to} (updated from {from})");
            println!("Installed to {}", path.display());
        }
        Err(error) => return Err(CliError::incomplete(format!("update failed: {error}"))),
    }
    if !integrations {
        return Ok(ExitCode::SUCCESS);
    }

    let mut failed = Vec::new();
    let mut restarts = Vec::new();
    for outcome in &leadline::update::update_integrations() {
        match outcome.state {
            IntegrationState::Skipped => continue,
            IntegrationState::Refreshed | IntegrationState::Manual => {
                println!("{}: {}", outcome.harness, outcome.detail);
            }
            IntegrationState::Failed => {
                eprintln!("{}: {}", outcome.harness, outcome.detail);
                failed.push(outcome.harness);
            }
        }
        if outcome.restart {
            restarts.push(outcome.harness);
        }
    }
    if !restarts.is_empty() {
        println!("Restart required: {}.", restarts.join(", "));
    }
    if failed.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Err(CliError::incomplete(format!(
            "binary update succeeded; failed harness integrations: {}",
            failed.join(", ")
        )))
    }
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

/// `--index DIR` when given, else the configured `[index].path` for this
/// analysis root, else `None` (cold analysis).
fn resolve_index_dir(path: &Path, flag: Option<&Path>, config: Option<&Config>) -> Option<PathBuf> {
    if let Some(dir) = flag {
        return Some(dir.to_owned());
    }
    config
        .and_then(|selected| selected.index.as_ref())
        .map(|index| config_dir(path).join(index.path.as_str()))
}

/// Analyze with the analysis index.
///
/// The caller resolves the index directory with [`resolve_index_dir`]; `None`
/// means cold analysis. Coverage merges into metrics after analysis, so
/// indexed (pre-coverage) functions would carry stale CRAP: any coverage
/// input bypasses the index. Index write failures warn on stderr and never
/// fail the run.
fn analyze_with_index(
    path: &Path,
    coverage: Option<&CoverageMap>,
    excludes: &[String],
    index_dir: Option<&Path>,
) -> Result<AnalysisReport, CliError> {
    let Some(dir) = index_dir.filter(|_| coverage.is_none()) else {
        return leadline::analyze_path_with_excludes(path, coverage, excludes)
            .map_err(|error| CliError::incomplete(error.to_string()));
    };
    let previous = leadline::index::AnalysisIndex::open(dir);
    let fingerprint = leadline::config::fingerprint(&config_dir(path));
    let (report, index, reuse) = if path.is_file() {
        leadline::index::refresh_file_index(path, &previous, &fingerprint)
            .map_err(|error| CliError::incomplete(error.to_string()))?
    } else {
        let entries = leadline::index::load_entries(path, excludes)
            .map_err(|error| CliError::incomplete(error.to_string()))?;
        leadline::index::refresh_files(
            Some(&previous),
            &leadline::index::scope_label(path),
            &fingerprint,
            &entries,
        )
        .map_err(|error| CliError::incomplete(error.to_string()))?
    };
    // A fully reused run rebuilds the same index; rewriting it costs a full
    // serialize and file write per invocation for no change. Files with parse
    // errors are re-analyzed but never stored, so compare the rebuilt index
    // against the stored one instead of counting analyzed files.
    if index != previous
        && let Err(error) = index.save(dir)
    {
        eprintln!(
            "leadline: index warning: cannot save index in {}: {error}",
            dir.display()
        );
    }
    eprintln!(
        "leadline: index {} reused, {} analyzed",
        reuse.reused, reuse.analyzed
    );
    Ok(report)
}

struct CommonOptions {
    path: PathBuf,
    json: bool,
    agent_json: bool,
    sarif: bool,
    /// CI-facing renderer selected by `--format`; mutually exclusive with the
    /// json, agent-json, and sarif projections.
    ci_format: Option<leadline::render::CiFormat>,
    pretty: bool,
    coverage: Option<CoverageMap>,
    budget: Budget,
    index_dir: Option<PathBuf>,
}

/// Optional scanner finding families merged into `check` output.
#[derive(Clone, Copy, Default)]
struct ScannerReports<'a> {
    security: Option<&'a leadline::security::SecurityOutcome>,
    vulnerabilities: Option<&'a leadline::vulnerabilities::VulnerabilityOutcome>,
    sql: Option<&'a leadline::sql::SqlOutcome>,
}

/// Metric gate inputs for one `check` run: the real absolute thresholds plus
/// this run's explicit regression violations. SARIF projects both; terminal
/// and JSON output ignore them because the report is already filtered.
#[derive(Clone, Copy)]
struct MetricGate<'a> {
    thresholds: &'a Thresholds,
    regressions: &'a [leadline::sarif::FunctionViolation<'a>],
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
        let mut ci_format: Option<leadline::render::CiFormat> = None;
        let mut coverage = CoverageMap::default();
        let mut has_coverage = false;
        let mut has_path = path != Path::new(".");
        let mut budget = Budget::default();
        let mut has_budget = false;
        let mut index_dir: Option<PathBuf> = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--json" => json = true,
                "--format" => {
                    index += 1;
                    parse_scan_format(
                        args.get(index),
                        &mut agent_json,
                        &mut sarif,
                        &mut ci_format,
                    )?;
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
                "--index" => {
                    index += 1;
                    index_dir =
                        Some(PathBuf::from(args.get(index).ok_or_else(|| {
                            CliError::usage("--index requires a directory")
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
        if index_dir.is_some() && command != "analyze" && command != "check" {
            return Err(CliError::usage(format!(
                "--index is only supported on analyze and check, not {command}"
            )));
        }
        Ok(Self {
            path,
            json,
            agent_json,
            sarif,
            ci_format,
            // `analyze --json` stays pretty-printed; other commands stay compact.
            pretty: command == "analyze",
            coverage: has_coverage.then_some(coverage),
            budget,
            index_dir,
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
            print_json(
                &leadline::agent::analyze_agent_json_budgeted(report, &self.budget),
                false,
            )?;
        } else if self.sarif {
            print_json(
                &leadline::sarif::analysis_to_sarif(report, thresholds),
                false,
            )?;
        } else if self.json {
            print_json(report, self.pretty)?;
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

    /// Scanner outcomes merged into `check` output; each is `None` when its
    /// input was absent, in which case output is exactly [`print_report_or`].
    /// Present families add `security_violations` / `vulnerability_violations` /
    /// `sql_violations` to `--json` and `--format agent-json`, merge their
    /// SARIF rules/results into one run, and append terminal sections when
    /// they have findings.
    fn print_report_or_with_security(
        &self,
        report: &AnalysisReport,
        scanners: ScannerReports<'_>,
        has_findings: bool,
        empty_text: &str,
        gate: MetricGate<'_>,
    ) -> Result<(), CliError> {
        let ScannerReports {
            security,
            vulnerabilities,
            sql,
        } = scanners;
        // Severity counts per scanner family and parse-error counts per
        // language; fixed label sets only, paths never leave this function.
        fn push_severity_counts<T>(
            kind: &'static str,
            violations: &[T],
            severity: impl Fn(&T) -> &'static str,
            out: &mut Vec<(&'static str, &'static str, u64)>,
        ) {
            for level in ["unknown", "low", "medium", "high", "critical"] {
                let count = violations
                    .iter()
                    .filter(|violation| severity(violation) == level)
                    .count() as u64;
                out.push((kind, level, count));
            }
        }
        let mut severity: Vec<(&str, &str, u64)> = Vec::new();
        if let Some(security) = security {
            push_severity_counts(
                "security",
                &security.violations,
                |violation| violation.severity.as_str(),
                &mut severity,
            );
        }
        if let Some(vulnerabilities) = vulnerabilities {
            push_severity_counts(
                "vulnerability",
                &vulnerabilities.violations,
                |violation| violation.severity.as_str(),
                &mut severity,
            );
        }
        if let Some(sql) = sql {
            push_severity_counts(
                "sql",
                &sql.violations,
                |violation| violation.severity.as_str(),
                &mut severity,
            );
        }
        let mut languages: Vec<(&str, u64)> = Vec::new();
        for file in &report.files {
            let count = file.parse_errors.len() as u64;
            if count == 0 {
                continue;
            }
            let language = file.language.as_str();
            match languages.iter_mut().find(|(known, _)| *known == language) {
                Some(slot) => slot.1 += count,
                None => languages.push((language, count)),
            }
        }
        leadline::telemetry::record_check_findings(
            "cli",
            &leadline::telemetry::CheckFindings {
                functions: report
                    .files
                    .iter()
                    .map(|file| file.functions.len() as u64)
                    .sum(),
                parse_errors: report
                    .files
                    .iter()
                    .map(|file| file.parse_errors.len() as u64)
                    .sum(),
                security: security.map_or(0, |outcome| outcome.violations.len() as u64),
                vulnerabilities: vulnerabilities
                    .map_or(0, |outcome| outcome.violations.len() as u64),
                sql: sql.map_or(0, |outcome| outcome.violations.len() as u64),
                severity: &severity,
                languages: &languages,
            },
        );
        let internal = |error: serde_json::Error| CliError::internal(error.to_string());
        if let Some(format) = self.ci_format {
            // One document, one code path: the CI renderers read exactly what
            // `--json` writes, so `report --from` over a saved run produces
            // byte-identical output to a live run with the same limits.
            let mut document = serde_json::to_value(report).map_err(internal)?;
            if let Some(security) = security {
                document["security_violations"] =
                    serde_json::to_value(&security.violations).map_err(internal)?;
            }
            if let Some(vulnerabilities) = vulnerabilities {
                document["vulnerability_violations"] =
                    serde_json::to_value(&vulnerabilities.violations).map_err(internal)?;
            }
            if let Some(sql) = sql {
                document["sql_violations"] =
                    serde_json::to_value(&sql.violations).map_err(internal)?;
            }
            print_ci_document(
                &document,
                report.files.len(),
                &gate_limits(gate.thresholds),
                format,
            )?;
        } else if self.agent_json {
            let mut value = serde_json::to_value(leadline::agent::analyze_agent_json_budgeted(
                report,
                &self.budget,
            ))
            .map_err(internal)?;
            if let Some(security) = security {
                value["security_violations"] =
                    serde_json::to_value(&security.violations).map_err(internal)?;
            }
            if let Some(vulnerabilities) = vulnerabilities {
                value["vulnerability_violations"] =
                    serde_json::to_value(&vulnerabilities.violations).map_err(internal)?;
            }
            if let Some(sql) = sql {
                value["sql_violations"] =
                    serde_json::to_value(&sql.violations).map_err(internal)?;
            }
            println!("{value}");
        } else if self.sarif {
            // Scanner families contribute gate violations only: `check`
            // exits 0 when nothing failed, so emitting informational findings
            // would contradict the exit code and the documented contract.
            let mut document = serde_json::to_value(leadline::sarif::gate_to_sarif(
                report,
                gate.thresholds,
                gate.regressions,
            ))
            .map_err(internal)?;
            if let Some(security) = security {
                let findings = leadline::sarif::security_findings_to_sarif(&security.violations);
                merge_sarif_run(&mut document, &findings);
            }
            if let Some(vulnerabilities) = vulnerabilities {
                let findings =
                    leadline::sarif::vulnerability_findings_to_sarif(&vulnerabilities.violations);
                merge_sarif_run(&mut document, &findings);
            }
            if let Some(sql) = sql {
                let findings = leadline::sarif::sql_findings_to_sarif(&sql.violations);
                merge_sarif_run(&mut document, &findings);
            }
            println!("{document}");
        } else if self.json {
            let mut value = serde_json::to_value(report).map_err(internal)?;
            if let Some(security) = security {
                value["security_violations"] =
                    serde_json::to_value(&security.violations).map_err(internal)?;
            }
            if let Some(vulnerabilities) = vulnerabilities {
                value["vulnerability_violations"] =
                    serde_json::to_value(&vulnerabilities.violations).map_err(internal)?;
            }
            if let Some(sql) = sql {
                value["sql_violations"] =
                    serde_json::to_value(&sql.violations).map_err(internal)?;
            }
            if self.pretty {
                println!("{value:#}");
            } else {
                println!("{value}");
            }
        } else if has_findings {
            self.print_report_or(report, true, empty_text, gate.thresholds)?;
            print_scanner_sections(security, vulnerabilities, sql);
        } else if has_scanner_findings(security, vulnerabilities, sql) {
            print_scanner_sections(security, vulnerabilities, sql);
        } else {
            println!("{empty_text}");
        }
        Ok(())
    }
}

/// Print scanner terminal sections for findings that exist.
fn has_scanner_findings(
    security: Option<&leadline::security::SecurityOutcome>,
    vulnerabilities: Option<&leadline::vulnerabilities::VulnerabilityOutcome>,
    sql: Option<&leadline::sql::SqlOutcome>,
) -> bool {
    security.is_some_and(|outcome| !outcome.report.findings.is_empty())
        || vulnerabilities.is_some_and(|outcome| !outcome.report.findings.is_empty())
        || sql.is_some_and(|outcome| !outcome.report.findings.is_empty())
}

/// Print scanner terminal sections for findings that exist.
fn print_scanner_sections(
    security: Option<&leadline::security::SecurityOutcome>,
    vulnerabilities: Option<&leadline::vulnerabilities::VulnerabilityOutcome>,
    sql: Option<&leadline::sql::SqlOutcome>,
) {
    if let Some(security) = security
        && !security.report.findings.is_empty()
    {
        print!("{}", leadline::security::terminal_text(&security.report));
    }
    if let Some(vulnerabilities) = vulnerabilities
        && !vulnerabilities.report.findings.is_empty()
    {
        print!(
            "{}",
            leadline::vulnerabilities::terminal_text(&vulnerabilities.report)
        );
    }
    if let Some(sql) = sql
        && !sql.report.findings.is_empty()
    {
        print!("{}", leadline::sql::terminal_text(&sql.report));
    }
}

/// Merge one scanner SARIF document's rules and results into a run.
fn merge_sarif_run(document: &mut serde_json::Value, extra: &serde_json::Value) {
    if let (Some(runs), Some(extra_runs)) = (
        document
            .get_mut("runs")
            .and_then(|runs| runs.as_array_mut()),
        extra.get("runs").and_then(|runs| runs.as_array()),
    ) && let (Some(run), Some(extra_run)) = (runs.first_mut(), extra_runs.first())
    {
        if let (Some(rules), Some(extra_rules)) = (
            run.pointer_mut("/tool/driver/rules")
                .and_then(|rules| rules.as_array_mut()),
            extra_run
                .pointer("/tool/driver/rules")
                .and_then(|rules| rules.as_array()),
        ) {
            rules.extend(extra_rules.iter().cloned());
        }
        if let (Some(results), Some(extra_results)) = (
            run.get_mut("results")
                .and_then(|results| results.as_array_mut()),
            extra_run
                .get("results")
                .and_then(|results| results.as_array()),
        ) {
            results.extend(extra_results.iter().cloned());
        }
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
    "Usage:\n  leadline analyze [PATH] [--json] [--format agent-json|sarif] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--index DIR]\n  leadline function FILE NAME [--json] [--format agent-json] [--explain] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline check [PATH] [--base REV | --baseline FILE] [--regressions] [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N] [--sarif FILE] [--baseline-sarif FILE] [--fail-on-severity low|medium|high|critical] [--new-only] [--changed-only] [--osv FILE] [--trivy FILE] [--baseline-osv FILE] [--baseline-trivy FILE] [--sql] [--sql-fail-on-severity low|medium|high|critical] [--json] [--format agent-json|sarif] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--index DIR]\n  leadline changed [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--min-delta D]\n  leadline diff [REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by crap|cognitive|cyclomatic] [--min-crap X] [--min-delta D]\n  leadline hotspots [PATH] [--limit N] [--since 30d|90d|365d] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
  leadline security [PATH] --sarif FILE [--baseline-sarif FILE] [--base REV | --staged | --target REV] [--fail-on-severity low|medium|high|critical] [--new-only] [--changed-only] [--json] [--format agent-json|sarif] [--top N]\n  leadline risk [PATH] [--limit N] [--since 30d|90d|365d] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline coupling TARGET [--path ROOT] [--top N] [--min-cochanges N] [--json] [--format agent-json]
  leadline dependencies [PATH] [--json] [--format agent-json]
  leadline impact TARGET [--path ROOT] [--top N] [--json] [--format agent-json]\n  leadline project [PATH] [--target REV] [--since 30d|90d|365d] [--pit FILE]... [--stryker FILE]... [--test-map FILE]... [--snapshots FILE] [--include-authors | --anonymize-authors | --exclude-git-identities] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--json] [--format agent-json]\n  leadline debt [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--since 30d|90d|365d] [--fail-on-regression] [--json] [--format agent-json]\n  leadline snapshot [PATH] --output FILE [--since 30d|90d|365d] [--pit FILE]... [--stryker FILE]... [--replace]\n  leadline mutation [PATH] (--pit FILE | --stryker FILE)... [--test-map FILE]... [--json]\n  leadline duplication [PATH] [--base REV] [--json]\n  leadline policy [PATH] [--base REV] [--fail-on-violation] [--json]\n  leadline sql [PATH] [--large-offset N] [--migration-root DIR] [--fail-on-severity low|medium|high|critical] [--json] [--format agent-json|sarif] [--top N]\n  leadline vulnerabilities [PATH] --osv FILE [--trivy FILE] [--baseline-osv FILE] [--baseline-trivy FILE] [--base REV | --staged | --target REV] [--fail-on-severity low|medium|high|critical] [--json] [--format agent-json|sarif] [--top N]\n  leadline sql-plan --current DIR --baseline DIR [--max-cost-increase-percent N] [--max-plan-rows-ratio N] [--max-estimate-error-ratio N] [--json] [--format agent-json|sarif] [--top N]\n  leadline doctor [PATH]\n  leadline test-targets [PATH] (--coverage FILE | --lcov FILE | --jacoco FILE) [--top N] [--format agent-json]\n  leadline baseline [PATH] --output FILE [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline mcp [--port [N] [--host ADDR]]\n  leadline index [PATH] [--output DIR] [--verify] [--json]\n  leadline skill\n  leadline update [--integrations]\n  leadline version\n  leadline --version"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_labels_follow_the_exit_code_contract() {
        assert_eq!(outcome_label(&Ok(ExitCode::SUCCESS)), "success");
        assert_eq!(outcome_label(&Ok(ExitCode::from(1))), "gate_failed");
        assert_eq!(outcome_label(&Ok(ExitCode::from(3))), "incomplete");
        assert_eq!(outcome_label(&Err(CliError::usage("x"))), "usage_error");
        assert_eq!(outcome_label(&Err(CliError::incomplete("x"))), "incomplete");
        assert_eq!(outcome_label(&Err(CliError::input("x"))), "input_error");
        assert_eq!(
            outcome_label(&Err(CliError::internal("x"))),
            "internal_error"
        );
    }

    #[test]
    fn operation_labels_stay_bounded_to_known_commands() {
        assert_eq!(cli_operation("analyze"), "analyze");
        assert_eq!(cli_operation("sql-plan"), "sql-plan");
        assert_eq!(cli_operation("--version"), "version");
        assert_eq!(cli_operation("-V"), "version");
        assert_eq!(cli_operation("--skill"), "skill");
        assert_eq!(cli_operation("--help"), "help");
        assert_eq!(cli_operation("-h"), "help");
        assert_eq!(cli_operation("help"), "help");
        assert_eq!(cli_operation("definitely-not-a-command"), "other");
    }

    /// The operation label list and the dispatch arms must stay in sync in
    /// both directions: a missing entry would record a real command as
    /// `other`, and a stale entry would label a usage error with a command
    /// that no longer exists.
    #[test]
    fn cli_operations_stay_in_sync_with_dispatch() {
        let source = include_str!("main.rs");
        let body: String = source
            .split_once("fn dispatch_command")
            .expect("dispatch_command must exist")
            .1
            .lines()
            .take_while(|line| *line != "}")
            .collect::<Vec<_>>()
            .join("\n");
        let mut checked = 0;
        for line in body.lines() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with('"') || !trimmed.contains("=>") {
                continue;
            }
            let (arms, _) = trimmed.split_once("=>").expect("arm must have =>");
            for literal in arms.split('"').skip(1).step_by(2) {
                assert_ne!(
                    cli_operation(literal),
                    "other",
                    "dispatch arm '{literal}' has no CLI_OPERATIONS entry"
                );
                checked += 1;
            }
        }
        assert!(checked >= CLI_OPERATIONS.len());
        // A future arm wrapped across lines still yields a command-shaped
        // literal, so scan the whole body as a second forward check.
        for literal in body.split('"').skip(1).step_by(2) {
            let command_shaped = !literal.is_empty()
                && literal.chars().all(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
                });
            if command_shaped {
                assert_ne!(
                    cli_operation(literal),
                    "other",
                    "dispatch literal '{literal}' has no CLI_OPERATIONS entry"
                );
            }
        }
        for operation in CLI_OPERATIONS {
            assert!(
                body.contains(&format!("\"{operation}\"")),
                "CLI_OPERATIONS entry '{operation}' has no dispatch arm"
            );
        }
    }
}
