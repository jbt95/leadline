use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use leadline::{analyze_path, analyze_source};
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn typescript_fixture(lines: usize) -> Vec<u8> {
    let mut source = String::with_capacity(lines * 16);
    for index in 0..lines / 5 {
        source.push_str(&format!(
            "function f{index}(x) {{\n  if (x && x > 1) {{\n    return x + 1;\n  }}\n}}\n"
        ));
    }
    source.into_bytes()
}

fn language_fixture(language: &str, functions: usize) -> (&'static str, Vec<u8>) {
    let mut source = String::with_capacity(functions * 80);
    match language {
        "java" => {
            source.push_str("class Fixture {\n");
            for index in 0..functions {
                source.push_str(&format!(
                    "int f{index}(int x) {{ if (x > 1) {{ return x + 1; }} return x; }}\n"
                ));
            }
            source.push_str("}\n");
            ("fixture.java", source.into_bytes())
        }
        "cpp" => {
            for index in 0..functions {
                source.push_str(&format!(
                    "int f{index}(int x) {{ if (x > 1) return x + 1; return x; }}\n"
                ));
            }
            ("fixture.cpp", source.into_bytes())
        }
        "javascript" => {
            for index in 0..functions {
                source.push_str(&format!(
                    "function f{index}(x) {{ if (x > 1) return x + 1; return x; }}\n"
                ));
            }
            ("fixture.js", source.into_bytes())
        }
        "typescript" => {
            for index in 0..functions {
                source.push_str(&format!(
                    "function f{index}(x: number): number {{ if (x > 1) return x + 1; return x; }}\n"
                ));
            }
            ("fixture.ts", source.into_bytes())
        }
        "tsx" => {
            for index in 0..functions {
                source.push_str(&format!(
                    "function F{index}(x: number) {{ return x > 1 ? <b>{{x}}</b> : <i>0</i>; }}\n"
                ));
            }
            ("fixture.tsx", source.into_bytes())
        }
        "rust" => {
            for index in 0..functions {
                source.push_str(&format!(
                    "fn f{index}(x: i32) -> i32 {{ if x > 1 {{ x + 1 }} else {{ x }} }}\n"
                ));
            }
            ("fixture.rs", source.into_bytes())
        }
        _ => unreachable!("known benchmark language"),
    }
}

fn assert_valid_fixture(path: &str, source: &[u8], expected_functions: usize) {
    let file = analyze_source(path, source).unwrap();
    assert!(
        file.parse_errors.is_empty(),
        "invalid benchmark fixture: {path}"
    );
    assert_eq!(file.functions.len(), expected_functions, "{path}");
}

fn source_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("typescript_analysis");
    group.sample_size(10);
    for lines in [10_000_usize, 100_000, 1_000_000] {
        let source = typescript_fixture(lines);
        assert_valid_fixture("fixture.ts", &source, lines / 5);
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(lines),
            &source,
            |bench, source| {
                bench.iter(|| black_box(analyze_source("fixture.ts", black_box(source)).unwrap()));
            },
        );
    }
    group.finish();
}

fn language_coverage(c: &mut Criterion) {
    let mut group = c.benchmark_group("language_analysis");
    group.sample_size(10);
    for language in ["cpp", "java", "javascript", "rust", "typescript", "tsx"] {
        let (path, source) = language_fixture(language, 2_000);
        assert_valid_fixture(path, &source, 2_000);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(language),
            &source,
            |bench, source| {
                bench.iter(|| black_box(analyze_source(path, black_box(source)).unwrap()));
            },
        );
    }
    group.finish();
}

fn parser_recovery(c: &mut Criterion) {
    let source = "function broken( { if (value &&) return value; }\n".repeat(2_000);
    let preflight = analyze_source("broken.ts", source.as_bytes()).unwrap();
    assert!(!preflight.parse_errors.is_empty());
    let mut group = c.benchmark_group("parser_recovery");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(source.len() as u64));
    group.bench_function("malformed-typescript", |bench| {
        bench
            .iter(|| black_box(analyze_source("broken.ts", black_box(source.as_bytes())).unwrap()));
    });
    group.finish();
}

fn repository_scaling(c: &mut Criterion) {
    let root = temporary_directory();
    for (files, lines_per_file) in [(1_usize, 10_000_usize), (100, 100), (1_000, 10)] {
        let directory = root.join(files.to_string());
        std::fs::create_dir(&directory).unwrap();
        let source = typescript_fixture(lines_per_file);
        for index in 0..files {
            std::fs::write(directory.join(format!("fixture-{index}.ts")), &source).unwrap();
        }
    }

    let mut analysis = c.benchmark_group("repository_analysis");
    analysis.sample_size(10);
    for files in [1_usize, 100, 1_000] {
        let directory = root.join(files.to_string());
        let preflight = analyze_path(&directory, None).unwrap();
        assert_eq!(preflight.files.len(), files);
        assert!(
            preflight
                .files
                .iter()
                .all(|file| file.parse_errors.is_empty())
        );
        assert_eq!(
            preflight
                .files
                .iter()
                .map(|file| file.functions.len())
                .sum::<usize>(),
            2_000
        );
        analysis.throughput(Throughput::Elements(files as u64));
        analysis.bench_with_input(
            BenchmarkId::new("10000-lines", files),
            &directory,
            |bench, directory| {
                bench.iter(|| black_box(analyze_path(black_box(directory), None).unwrap()));
            },
        );
    }
    analysis.finish();

    let report = analyze_path(&root.join("100"), None).unwrap();
    let mut serialization = c.benchmark_group("result_serialization");
    serialization.sample_size(10);
    let bytes = serde_json::to_vec(&report).unwrap().len() as u64;
    serialization.throughput(Throughput::Bytes(bytes));
    serialization.bench_function("100-files", |bench| {
        bench.iter(|| black_box(serde_json::to_vec(black_box(&report)).unwrap()));
    });
    serialization.finish();
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn join(&self, path: impl AsRef<std::path::Path>) -> PathBuf {
        self.0.join(path)
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
    let path = std::env::temp_dir().join(format!("leadline-bench-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    TemporaryDirectory(path)
}

criterion_group!(
    benches,
    source_scaling,
    language_coverage,
    parser_recovery,
    repository_scaling
);
criterion_main!(benches);
