use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn discovery_honors_gitignore_and_generated_defaults() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("node_modules")).unwrap();
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(root.join(".gitignore"), "ignored.ts\n").unwrap();
    std::fs::write(root.join("src/main.ts"), "function main() {}\n").unwrap();
    std::fs::write(root.join("ignored.ts"), "function ignored() {}\n").unwrap();
    std::fs::write(
        root.join("node_modules/vendor.js"),
        "function vendor() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("dist/output.js"), "function output() {}\n").unwrap();

    let discovered = leadline::discovery::discover(&root).unwrap();
    assert_eq!(discovered, vec![root.join("src/main.ts")]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_finds_go_sources() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("vendor")).unwrap();
    std::fs::write(root.join("src/main.go"), "package main\n\nfunc main() {}\n").unwrap();
    std::fs::write(root.join("vendor/dep.go"), "package dep\n").unwrap();
    std::fs::write(root.join("notes.txt"), "not source\n").unwrap();
    let discovered = leadline::discovery::discover(&root).unwrap();
    assert_eq!(discovered, vec![root.join("src/main.go")]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_finds_rust_sources() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("target/debug/build.rs"), "fn build() {}\n").unwrap();
    std::fs::write(root.join("notes.txt"), "not source\n").unwrap();
    let discovered = leadline::discovery::discover(&root).unwrap();
    assert_eq!(discovered, vec![root.join("src/main.rs")]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_finds_c_sources() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.c"), "int main(void) { return 0; }\n").unwrap();
    std::fs::write(root.join("notes.txt"), "not source\n").unwrap();
    let discovered = leadline::discovery::discover(&root).unwrap();
    assert_eq!(discovered, vec![root.join("src/main.c")]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_finds_cpp_sources() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("include")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("include/api.h"), "int api();\n").unwrap();
    std::fs::write(root.join("src/main.cpp"), "int main() { return 0; }\n").unwrap();
    std::fs::write(root.join("notes.txt"), "not source\n").unwrap();
    let discovered = leadline::discovery::discover(&root).unwrap();
    assert_eq!(
        discovered,
        vec![root.join("include/api.h"), root.join("src/main.cpp")]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_ignored_file_is_analyzable() {
    let root = temporary_directory();
    std::fs::write(root.join(".gitignore"), "ignored.ts\n").unwrap();
    let file = root.join("ignored.ts");
    std::fs::write(&file, "function ignored() {}\n").unwrap();
    assert_eq!(leadline::discovery::discover(&file).unwrap(), vec![file]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn single_file_report_preserves_invoked_path() {
    let root = temporary_directory();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.ts");
    std::fs::write(&file, "function main() {}\n").unwrap();

    let single = leadline::analyze_path(&file, None).unwrap();
    assert_eq!(single.files.len(), 1);
    assert_eq!(single.files[0].path, leadline::normalize_path(&file));

    let directory = leadline::analyze_path(&root, None).unwrap();
    assert_eq!(directory.files.len(), 1);
    assert_eq!(directory.files[0].path, "src/main.ts");
    std::fs::remove_dir_all(root).unwrap();
}

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}
