//! Real-world smoke + throughput over floating-main fixtures.
//!
//! Fixtures come from `scripts/realworld.sh` (shallow clones under
//! `target/realworld/`, honoring `LEADLINE_REALWORLD_DIR`). A missing
//! fixture dir skips gracefully with a pointer — never a failure — so this
//! bench is a no-op without network. Throughput floors live in
//! `benches/realworld.toml` and are enforced after each group.

use criterion::{Throughput, criterion_group, criterion_main};
use leadline::analyze_path;
use serde::Deserialize;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    subpath: String,
    #[allow(dead_code)]
    url: String,
    #[allow(dead_code)]
    branch: String,
    floor_files_per_sec: f64,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    fixture: Vec<Fixture>,
}

fn manifest() -> Manifest {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/realworld.toml");
    let text = std::fs::read_to_string(&path).expect("realworld manifest readable");
    toml::from_str(&text).expect("realworld manifest parses")
}

fn fixture_root() -> PathBuf {
    if let Ok(dir) = std::env::var("LEADLINE_REALWORLD_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/realworld")
}

fn analyze_smoke(name: &str, root: &std::path::Path) -> (usize, usize) {
    let report = analyze_path(root, None)
        .unwrap_or_else(|error| panic!("realworld/{name}: analysis failed: {error}"));
    let files = report.files.len();
    let functions: usize = report.files.iter().map(|file| file.functions.len()).sum();
    assert!(files > 0, "realworld/{name}: no files analyzed");
    assert!(functions > 0, "realworld/{name}: no functions found");
    let rerun = analyze_path(root, None).expect("realworld rerun succeeds");
    assert_eq!(
        serde_json::to_vec(&report).unwrap(),
        serde_json::to_vec(&rerun).unwrap(),
        "realworld/{name}: consecutive runs differ"
    );
    (files, functions)
}

/// Join smoke on one fixture: the risk model and the project join must
/// build over real data with non-empty risk rows. History is unavailable
/// (no revision walk here), so churn/ownership stay unknown by design.
fn join_smoke(path: &std::path::Path) {
    let analysis = analyze_path(path, None).expect("realworld join analysis succeeds");
    let graph =
        leadline::graph::analyze_dependencies(path, &[]).expect("realworld join graph succeeds");
    let history = leadline::history::HistoryReport {
        schema_version: leadline::history::HISTORY_SCHEMA_VERSION,
        analyzer_version: env!("CARGO_PKG_VERSION"),
        available: false,
        reference: "head-commit-time",
        head_commit: None,
        head_timestamp: None,
        files: Vec::new(),
    };
    let ownership = leadline::ownership::OwnershipReport {
        files: Vec::new(),
        modules: Vec::new(),
    };
    let policy = leadline::policy::evaluate(&graph, &[]);
    let risk = leadline::risk::build(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        leadline::history::HistoryWindow::Days90,
    );
    assert!(!risk.risks.is_empty(), "realworld join: empty risk rows");
    let duplication =
        leadline::duplication::detect(&[], &leadline::config::DuplicationConfig::default());
    let project = leadline::project::build(leadline::project::ProjectInputs {
        analysis: &analysis,
        generated_from: "realworld".to_owned(),
        git: None,
        graph: &graph,
        ownership: None,
        mutation: None,
        test_relationships: None,
        duplication: &duplication,
        policy: &policy,
        risk: &risk,
        snapshots: None,
    });
    assert!(
        !project.risk.rows.is_empty(),
        "realworld join: empty project risk rows"
    );
    println!("realworld: join smoke ok ({} risk rows)", risk.risks.len());
}

fn realworld(c: &mut criterion::Criterion) {
    let root = fixture_root();
    for fixture in &manifest().fixture {
        let path = root.join(&fixture.name).join(&fixture.subpath);
        if !path.is_dir() {
            println!(
                "realworld: skipping {} (missing {} — run scripts/realworld.sh)",
                fixture.name,
                path.display()
            );
            continue;
        }
        let (files, functions) = analyze_smoke(&fixture.name, &path);
        println!(
            "realworld: {} — {files} files, {functions} functions",
            fixture.name
        );
        if fixture.name == "tanstack-query" {
            join_smoke(&path);
        }
        let mut group = c.benchmark_group("realworld");
        group.throughput(Throughput::Elements(files as u64));
        group.bench_function(&fixture.name, |bench| {
            bench.iter(|| black_box(analyze_path(black_box(&path), None).unwrap()));
        });
        group.finish();
        enforce_floor(fixture, &path, files);
    }
}

fn enforce_floor(fixture: &Fixture, path: &std::path::Path, files: usize) {
    if fixture.floor_files_per_sec <= 0.0 {
        return;
    }
    let start = Instant::now();
    analyze_path(path, None).expect("realworld floor probe succeeds");
    let measured = files as f64 / start.elapsed().as_secs_f64();
    println!(
        "realworld: {} — {measured:.1} files/sec vs floor {:.1}",
        fixture.name, fixture.floor_files_per_sec
    );
    assert!(
        measured >= fixture.floor_files_per_sec,
        "realworld/{}: {measured:.1} files/sec below floor {:.1} (benches/realworld.toml); re-baseline per docs/realworld-benchmarks.md",
        fixture.name,
        fixture.floor_files_per_sec
    );
}

criterion_group!(benches, realworld);
criterion_main!(benches);
