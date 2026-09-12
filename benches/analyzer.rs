use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use leadline::{analyze_path, analyze_source};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(lines: usize) -> Vec<u8> {
    let mut source = String::with_capacity(lines * 16);
    for index in 0..lines / 5 {
        source.push_str(&format!(
            "function f{index}(x) {{\n  if (x && x > 1) {{\n    return x + 1;\n  }}\n}}\n"
        ));
    }
    source.into_bytes()
}

fn analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("typescript_analysis");
    group.sample_size(10);
    for lines in [10_000_usize, 100_000, 1_000_000] {
        let source = fixture(lines);
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(lines),
            &source,
            |bench, source| {
                bench.iter(|| analyze_source("fixture.ts", source).unwrap());
            },
        );
    }
    group.finish();
}

fn repository(c: &mut Criterion) {
    let root = temporary_directory();
    let files = 100_u64;
    for index in 0..files {
        std::fs::write(root.join(format!("fixture-{index}.ts")), fixture(100)).unwrap();
    }
    let report = analyze_path(&root, None).unwrap();

    let mut analysis = c.benchmark_group("repository_analysis");
    analysis.sample_size(10);
    analysis.throughput(Throughput::Elements(files));
    analysis.bench_function("100-files-10000-lines", |bench| {
        bench.iter(|| analyze_path(&root, None).unwrap());
    });
    analysis.finish();

    let mut serialization = c.benchmark_group("result_serialization");
    let bytes = serde_json::to_vec(&report).unwrap().len() as u64;
    serialization.throughput(Throughput::Bytes(bytes));
    serialization.bench_function("100-files", |bench| {
        bench.iter(|| serde_json::to_vec(&report).unwrap());
    });
    serialization.finish();
    std::fs::remove_dir_all(root).unwrap();
}

fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("leadline-bench-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

criterion_group!(benches, analysis, repository);
criterion_main!(benches);
