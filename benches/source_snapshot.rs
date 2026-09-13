use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use leadline::analyze_sources;
use leadline::graph::analyze_dependencies_from_sources;
use leadline::source_snapshot::SourceEntry;
use std::hint::black_box;

fn entries(count: usize) -> Vec<SourceEntry> {
    (0..count)
        .map(|index| {
            let source = format!(
                "import './module-{:05}';\nexport function f{index}(x: number) {{ return x > {index} ? x : {index}; }}\n",
                (index + 1) % count
            );
            SourceEntry {
                path: format!("src/module-{index:05}.ts"),
                bytes: source.into_bytes(),
            }
        })
        .collect()
}

fn snapshot_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_analysis");
    group.sample_size(10);
    for count in [1_000_usize, 10_000] {
        let entries = entries(count);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &entries,
            |bench, entries| {
                bench.iter(|| black_box(analyze_sources(black_box(entries), None).unwrap()));
            },
        );
    }
    group.finish();
}

fn snapshot_dependencies(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_dependencies");
    group.sample_size(10);
    for count in [1_000_usize, 10_000] {
        let entries = entries(count);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &entries,
            |bench, entries| {
                bench.iter(|| {
                    black_box(analyze_dependencies_from_sources(black_box(entries)).unwrap())
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, snapshot_analysis, snapshot_dependencies);
criterion_main!(benches);
