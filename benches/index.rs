use criterion::{Criterion, criterion_group, criterion_main};
use leadline::index::refresh_files;
use leadline::source_snapshot::SourceEntry;

fn synthetic_repo(files: usize, functions: usize) -> Vec<SourceEntry> {
    (0..files)
        .map(|file| {
            let mut source = String::new();
            for function in 0..functions {
                source.push_str(&format!(
                    "export function f{function}(a: number) {{\n  let total = 0;\n  for (let i = 0; i < a; i++) {{ total += i; }}\n  return total;\n}}\n"
                ));
            }
            SourceEntry {
                path: format!("src/file{file}.ts"),
                bytes: source.into_bytes(),
            }
        })
        .collect()
}

fn index_refresh(c: &mut Criterion) {
    // Controller ruling: 120 x 12 keeps one iteration well under the criterion
    // budget while still exercising a repository-sized refresh.
    let entries = synthetic_repo(120, 12);
    let (_, index, _) = refresh_files(None, ".", "bench", &entries).unwrap();
    let mut group = c.benchmark_group("index_refresh");
    group.bench_function("cold_analyze", |bencher| {
        bencher.iter(|| refresh_files(None, ".", "bench", &entries).unwrap())
    });
    group.bench_function("warm_refresh", |bencher| {
        bencher.iter(|| refresh_files(Some(&index), ".", "bench", &entries).unwrap())
    });
    group.finish();
}

criterion_group!(benches, index_refresh);
criterion_main!(benches);
