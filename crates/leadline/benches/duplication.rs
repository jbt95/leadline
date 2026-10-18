use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use leadline::config::DuplicationConfig;
use leadline::duplication::detect;
use leadline::source_snapshot::SourceEntry;
use std::hint::black_box;

const SHARED: &str = r#"
export function shared(input: number): number {
  let total = 0;
  for (let index = 0; index < input; index += 1) {
    if (index % 2 === 0) {
      total += index * 3;
    } else {
      total -= index - 1;
    }
  }
  return total;
}
"#;

fn entries(count: usize, repeated: bool) -> Vec<SourceEntry> {
    (0..count)
        .map(|index| {
            let source = if repeated && index % 2 == 1 {
                SHARED.to_owned()
            } else {
                format!(
                    "export function f{index}(value: number): number {{\n  let result = value;\n  if (value > {index}) {{ result += 1; }}\n  return result;\n}}\n"
                )
            };
            SourceEntry {
                path: format!("src/module-{index:05}.ts"),
                bytes: source.into_bytes(),
            }
        })
        .collect()
}

fn duplication_detection(c: &mut Criterion) {
    let config = DuplicationConfig {
        min_tokens: 12,
        min_lines: 5,
        excludes: Vec::new(),
    };
    let mut group = c.benchmark_group("duplication_detection");
    group.sample_size(10);
    for (label, repeated) in [("unique", false), ("repeated", true)] {
        for count in [500_usize, 2_000] {
            let entries = entries(count, repeated);
            group.throughput(Throughput::Elements(count as u64));
            group.bench_with_input(
                BenchmarkId::new(label, count),
                &entries,
                |bench, entries| {
                    bench.iter(|| black_box(detect(black_box(entries), &config)));
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, duplication_detection);
criterion_main!(benches);
