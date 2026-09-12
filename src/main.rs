use leadline::config::{self, Config};
use leadline::core::{
    AnalysisReport, METRIC_PROFILE, MetricSpecs, OUTPUT_SCHEMA_VERSION, Thresholds,
};
use leadline::coverage::CoverageMap;
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
    let report =
        leadline::analyze_path_with_excludes(&options.path, options.coverage.as_ref(), excludes)
            .map_err(|error| CliError::incomplete(error.to_string()))?;
    if report.files.is_empty() {
        return Err(CliError::incomplete(format!(
            "no supported files found under {}",
            options.path.display()
        )));
    }
    options.print_report(&report)?;
    Ok(ExitCode::SUCCESS)
}

fn function_command(args: &[String]) -> Result<ExitCode, CliError> {
    if args.len() < 2 || args[0].starts_with('-') || args[1].starts_with('-') {
        return Err(CliError::usage("function requires FILE and NAME"));
    }
    let file = PathBuf::from(&args[0]);
    let name = &args[1];
    let options = CommonOptions::parse_with_path(&args[2..], file.clone(), "function")?;
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
    options.print_report(&report)?;
    Ok(ExitCode::SUCCESS)
}

fn changed_command(command: &str, args: &[String]) -> Result<ExitCode, CliError> {
    let mut base = None;
    let mut json = false;
    let mut agent_json = false;
    let mut path = PathBuf::from(".");
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
    let report = leadline::diff::analyze_changed(&path, base.as_deref().unwrap_or("HEAD~1"))
        .map_err(|error| CliError::incomplete(error.to_string()))?;
    if agent_json {
        println!(
            "{}",
            serde_json::to_string(&leadline::agent::changed_agent_json(&report))
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
    let mut common = Vec::new();
    let mut index = 0;
    while index < args.len() {
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
    if thresholds.is_empty() {
        return Err(CliError::usage(
            "check requires at least one metric threshold",
        ));
    }

    let options = CommonOptions::parse(&common, "check")?;
    let config = load_config_for(&options.path)?;
    if let Some(config) = &config {
        apply_config(&mut thresholds, config);
    }
    let excludes: &[String] = config
        .as_ref()
        .map(|selected| selected.analysis_excludes.as_slice())
        .unwrap_or(&[]);
    let mut report =
        leadline::analyze_path_with_excludes(&options.path, options.coverage.as_ref(), excludes)
            .map_err(|error| CliError::incomplete(error.to_string()))?;
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
    options.print_report_or(&report, has_findings, "No violations.")?;
    if has_findings {
        return Ok(ExitCode::from(1));
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

struct CommonOptions {
    path: PathBuf,
    json: bool,
    agent_json: bool,
    pretty: bool,
    coverage: Option<CoverageMap>,
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
        let mut coverage = CoverageMap::default();
        let mut has_coverage = false;
        let mut has_path = path != Path::new(".");
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
        Ok(Self {
            path,
            json,
            agent_json,
            // `analyze --json` stays pretty-printed; other commands stay compact.
            pretty: command == "analyze",
            coverage: has_coverage.then_some(coverage),
        })
    }

    /// JSON output keeps its per-command encoding; agent-json is compact.
    fn print_report(&self, report: &AnalysisReport) -> Result<(), CliError> {
        self.print_report_or(report, true, "")
    }

    fn print_report_or(
        &self,
        report: &AnalysisReport,
        has_findings: bool,
        empty_text: &str,
    ) -> Result<(), CliError> {
        if self.agent_json {
            println!(
                "{}",
                serde_json::to_string(&leadline::agent::analyze_agent_json(report))
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
    "Usage:\n  leadline analyze [PATH] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline function FILE NAME [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline check [PATH] [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]\n  leadline changed [--base REV] [--path PATH] [--json] [--format agent-json]\n  leadline diff [REV] [--path PATH] [--json] [--format agent-json]\n  leadline doctor [PATH]\n  leadline mcp\n  leadline skill\n  leadline version\n  leadline --version"
}
