use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use leadline::core::{
    AnalysisReport, FileAnalysis, METRIC_PROFILE, MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use leadline::coupling::{CouplingOptions, analyze_coupling};
use leadline::history::{
    FileHistory, HISTORY_SCHEMA_VERSION, HistoryReport, HistoryWindow, analyze_history,
};
use leadline::hotspots::build;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const FILES_PER_ROUND: usize = 5;
const FILE_COUNT: usize = 50;

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temporary_directory() -> TemporaryDirectory {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("leadline-history-bench-{nonce}"));
    std::fs::create_dir_all(&path).unwrap();
    TemporaryDirectory(path)
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn source(index: usize, round: usize) -> String {
    format!(
        "export function f{index}(x: number): number {{\n  if (x > {round}) {{\n    return x + 1;\n  }}\n  return x;\n}}\n"
    )
}

/// Stages a synthetic repository once, outside every timed loop.
fn stage_repository(rounds: usize) -> TemporaryDirectory {
    let root = temporary_directory();
    let path = root.path();
    git(path, &["init", "-q"]);
    git(path, &["config", "user.email", "bench@example.invalid"]);
    git(path, &["config", "user.name", "Bench Author"]);
    for round in 0..rounds {
        for offset in 0..FILES_PER_ROUND {
            let index = (round * FILES_PER_ROUND + offset) % FILE_COUNT;
            let directory = path.join(format!("src/mod{}", index % 10));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join(format!("file{index}.ts")),
                source(index, round),
            )
            .unwrap();
        }
        git(path, &["add", "-A"]);
        let message = format!("round {round}");
        git(path, &["commit", "-q", "-m", &message]);
    }
    root
}

/// Cost of `git log` itself: subprocess, history walk, and output generation.
/// Comparing this with `history_analysis` isolates leadline's parsing and
/// aggregation overhead on the same repository and flags.
fn raw_git_log(c: &mut Criterion) {
    let repo = stage_repository(200);
    let mut group = c.benchmark_group("raw_git_log");
    group.sample_size(10);
    group.throughput(Throughput::Elements(200));
    group.bench_function("200-commits", |bench| {
        bench.iter(|| {
            let mut child = Command::new("git")
                .current_dir(repo.path())
                .args([
                    "log",
                    "--relative",
                    "--no-merges",
                    "--numstat",
                    "-z",
                    "-M30%",
                    "--format=%x01%H%x00%aN%x00%aE%x00%ct%x00",
                ])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdout = child.stdout.take().unwrap();
            std::io::copy(&mut stdout, &mut std::io::sink()).unwrap();
            assert!(child.wait().unwrap().success());
        });
    });
    group.finish();
}

/// End-to-end history loading and aggregation for two repository sizes.
fn history_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("history_analysis");
    group.sample_size(10);
    for rounds in [20_usize, 200] {
        let repo = stage_repository(rounds);
        let scope = repo.path().to_path_buf();
        let preflight = analyze_history(&scope).unwrap();
        assert!(preflight.available);
        let touched: u64 = preflight.files.iter().map(|file| file.commits).sum();
        assert_eq!(touched, (rounds * FILES_PER_ROUND) as u64);
        group.throughput(Throughput::Elements(rounds as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(rounds),
            &scope,
            |bench, scope| {
                bench.iter(|| black_box(analyze_history(black_box(scope)).unwrap()));
            },
        );
    }
    group.finish();
}

fn history_fixture(path: &str, changes: u64) -> FileHistory {
    FileHistory {
        path: path.to_owned(),
        commits: changes,
        changes_30d: changes / 4,
        changes_90d: changes,
        changes_365d: changes * 3,
        lines_added: changes * 12,
        lines_deleted: changes * 4,
        days_since_last_change: 2,
        age_days: 400,
        contributors: 5,
        recent_contributors: 3,
    }
}

/// Join and ranking cost on an already-analyzed project, with no Git access.
fn hotspot_scoring(c: &mut Criterion) {
    const FILES: usize = 2_000;
    let template = leadline::analyze_source("src/template.ts", source(0, 1).as_bytes()).unwrap();
    let files: Vec<FileAnalysis> = (0..FILES)
        .map(|index| FileAnalysis {
            path: format!("src/mod{}/file{index}.ts", index % 32),
            ..template.clone()
        })
        .collect();
    let analysis = AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: "bench",
        metric_specs: MetricSpecs::default(),
        files,
    };
    let history = HistoryReport {
        schema_version: HISTORY_SCHEMA_VERSION,
        analyzer_version: "bench",
        available: true,
        reference: "head-commit-time",
        head_commit: Some("bench".to_owned()),
        head_timestamp: Some(1_775_001_600),
        files: (0..FILES)
            .map(|index| {
                history_fixture(
                    &format!("src/mod{}/file{index}.ts", index % 32),
                    index as u64,
                )
            })
            .collect(),
    };
    let mut group = c.benchmark_group("hotspot_scoring");
    group.sample_size(10);
    group.throughput(Throughput::Elements(FILES as u64));
    group.bench_function("2000-files", |bench| {
        bench.iter(|| black_box(build(&analysis, &history, HistoryWindow::Days90, 10)));
    });
    group.finish();
}

/// Co-change indexing for one target over the same staged repository; the
/// target is written in round 0, so it has history in every size.
fn coupling_analysis(c: &mut Criterion) {
    let repo = stage_repository(200);
    let scope = repo.path().to_path_buf();
    let target = "src/mod0/file0.ts";
    let preflight = analyze_coupling(&scope, target, &CouplingOptions::default()).unwrap();
    assert!(preflight.git_available);
    assert!(preflight.target_commits > 0);
    let mut group = c.benchmark_group("coupling_analysis");
    group.sample_size(10);
    group.throughput(Throughput::Elements(200));
    group.bench_function("200-commits", |bench| {
        bench.iter(|| {
            black_box(
                analyze_coupling(
                    black_box(&scope),
                    black_box(target),
                    &CouplingOptions::default(),
                )
                .unwrap(),
            )
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    raw_git_log,
    history_analysis,
    hotspot_scoring,
    coupling_analysis
);
criterion_main!(benches);
