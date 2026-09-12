use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn changed_analysis_compares_function_versions() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(
        root.join("app.ts"),
        "function choose(x: number) {\n  if (x > 0) return 1;\n  return 0;\n}\n",
    )
    .unwrap();
    git(&root, &["add", "app.ts"]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("app.ts"),
        "function choose(x: number) {\n  if (x > 0) {\n    if (x > 1) return 2;\n    return 1;\n  }\n  return 0;\n}\n",
    )
    .unwrap();

    let report = leadline::diff::analyze_changed(&root, "HEAD").unwrap();
    assert_eq!(report.functions.len(), 1);
    let change = &report.functions[0];
    assert_eq!(change.name, "choose");
    assert_eq!(change.before.as_ref().unwrap().metrics.cyclomatic, 2);
    assert_eq!(change.after.as_ref().unwrap().metrics.cyclomatic, 3);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_analysis_honors_scope_and_reports_parse_errors() {
    let root = temporary_directory();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::create_dir_all(root.join("first")).unwrap();
    std::fs::create_dir_all(root.join("second")).unwrap();
    std::fs::write(root.join("first/a.ts"), "function a() { return 1; }\n").unwrap();
    std::fs::write(root.join("second/b.ts"), "function b() { return 1; }\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("first/a.ts"),
        "function a(x: boolean) { if (x) return 1; return 0; }\n",
    )
    .unwrap();
    std::fs::write(root.join("second/b.ts"), "function b( { return 1; }\n").unwrap();

    let scoped = leadline::diff::analyze_changed(&root.join("first"), "HEAD").unwrap();
    assert_eq!(scoped.functions.len(), 1);
    assert_eq!(scoped.functions[0].path, "first/a.ts");
    assert!(scoped.parse_errors.is_empty());

    let complete = leadline::diff::analyze_changed(&root, "HEAD").unwrap();
    assert_eq!(complete.parse_errors.len(), 1);
    assert_eq!(complete.parse_errors[0].path, "second/b.ts");
    assert!(!complete.parse_errors[0].after.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_analysis_rejects_option_like_revisions() {
    let error = leadline::diff::analyze_changed(Path::new("."), "--help").unwrap_err();
    assert!(error.to_string().contains("unsupported characters"));
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-git-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}
