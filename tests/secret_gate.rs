#![cfg(unix)]

//! Secret-gate runner tests with fake scanner binaries: no network, no real
//! gitleaks, no real findings. Everything runs off a temporary `PATH`.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
mod common;
use common::{temporary_directory, write_executable};

fn runner() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("integrations/common/leadline-secret-check.sh")
}

/// Fake `gitleaks`: records argv, writes sentinel SARIF, exits as told.
fn fake_gitleaks(bin: &Path) {
    write_executable(
        &bin.join("gitleaks"),
        "#!/bin/sh\n\
         echo \"$@\" > \"$ARGS_FILE\"\n\
         previous=\"\"\n\
         for argument in \"$@\"; do\n\
           if [ \"$previous\" = \"--report-path\" ]; then printf 'SECRET_SENTINEL' > \"$argument\"; fi\n\
           previous=\"$argument\"\n\
         done\n\
         exit \"$EXIT_CODE\"\n",
    );
}

/// Fake `leadline`: records argv, exits as told.
fn fake_leadline(bin: &Path) {
    write_executable(
        &bin.join("leadline"),
        "#!/bin/sh\necho \"$@\" > \"$LEADLINE_ARGS_FILE\"\nexit \"$LEADLINE_EXIT\"\n",
    );
}

fn run_runner(root: &Path, mode: &str, extra_env: &[(&str, &str)]) -> Output {
    let bin = root.join("bin");
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(
        "PATH".to_owned(),
        format!("{}:{}:/usr/bin:/bin", bin.display(), git_directory()),
    );
    env.insert("TMPDIR".to_owned(), root.join("tmp").display().to_string());
    env.insert("LEADLINE_SECRET_MODE".to_owned(), mode.to_owned());
    env.insert(
        "LEADLINE_BIN".to_owned(),
        bin.join("leadline").display().to_string(),
    );
    env.insert(
        "ARGS_FILE".to_owned(),
        root.join("gitleaks.args").display().to_string(),
    );
    env.insert(
        "LEADLINE_ARGS_FILE".to_owned(),
        root.join("leadline.args").display().to_string(),
    );
    env.insert("EXIT_CODE".to_owned(), "1".to_owned());
    env.insert("LEADLINE_EXIT".to_owned(), "1".to_owned());
    for (key, value) in extra_env {
        env.insert((*key).to_owned(), (*value).to_owned());
    }
    std::fs::create_dir_all(root.join("tmp")).unwrap();
    Command::new(runner())
        .envs(&env)
        .current_dir(root)
        .output()
        .unwrap()
}

fn fixture_bin(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    fake_gitleaks(&bin);
    fake_leadline(&bin);
    bin
}

/// Directory holding the real `git`, so worktree-mode runner tests exercise
/// the preflight rather than a missing executable.
fn git_directory() -> String {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let path = String::from_utf8(output.stdout).unwrap();
    let path = path.trim();
    assert!(
        !path.is_empty(),
        "git is required for worktree runner tests"
    );
    Path::new(path).parent().unwrap().display().to_string()
}

/// Turn `root` into a repository with one commit so the worktree preflight
/// (which needs a HEAD to diff against) passes.
fn init_repository(root: &Path) {
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["init", "-q"]);
    run(&[
        "-c",
        "user.name=leadline-test",
        "-c",
        "user.email=leadline-test@example.com",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "init",
    ]);
}

#[test]
fn secret_runner_uses_redacted_staged_scan() {
    let root = temporary_directory();
    fixture_bin(&root);
    let output = run_runner(&root, "staged", &[]);
    assert_eq!(output.status.code(), Some(1));
    let gitleaks_args = std::fs::read_to_string(root.join("gitleaks.args")).unwrap();
    assert!(gitleaks_args.contains("--redact"), "{gitleaks_args}");
    assert!(gitleaks_args.contains("--staged"), "{gitleaks_args}");
    assert!(
        gitleaks_args.contains("--report-format sarif"),
        "{gitleaks_args}"
    );
    let leadline_args = std::fs::read_to_string(root.join("leadline.args")).unwrap();
    assert!(leadline_args.contains("security"), "{leadline_args}");
    assert!(leadline_args.contains("--staged"), "{leadline_args}");
    assert!(leadline_args.contains("--changed-only"), "{leadline_args}");
    assert!(
        leadline_args.contains("--fail-on-severity low"),
        "{leadline_args}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_uses_worktree_scan_with_base_target() {
    let root = temporary_directory();
    fixture_bin(&root);
    init_repository(&root);
    let output = run_runner(&root, "worktree", &[]);
    assert_eq!(output.status.code(), Some(1));
    let gitleaks_args = std::fs::read_to_string(root.join("gitleaks.args")).unwrap();
    assert!(gitleaks_args.contains("--redact"), "{gitleaks_args}");
    assert!(!gitleaks_args.contains("--staged"), "{gitleaks_args}");
    let leadline_args = std::fs::read_to_string(root.join("leadline.args")).unwrap();
    assert!(leadline_args.contains("--base HEAD"), "{leadline_args}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_skips_worktree_gate_outside_a_repository() {
    let root = temporary_directory();
    fixture_bin(&root);
    let output = run_runner(&root, "worktree", &[]);
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a git repository"), "{stderr}");
    assert!(stderr.contains("worktree secret gate skipped"), "{stderr}");
    assert!(
        !root.join("gitleaks.args").exists(),
        "the preflight must skip the scanner"
    );
    assert!(
        !root.join("leadline.args").exists(),
        "the preflight must skip the gate"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_skips_worktree_gate_without_commits() {
    let root = temporary_directory();
    fixture_bin(&root);
    let init = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["init", "-q"])
        .output()
        .unwrap();
    assert!(init.status.success());
    let output = run_runner(&root, "worktree", &[]);
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no commits"), "{stderr}");
    assert!(stderr.contains("worktree secret gate skipped"), "{stderr}");
    assert!(
        !root.join("gitleaks.args").exists(),
        "the preflight must skip the scanner"
    );
    assert!(
        !root.join("leadline.args").exists(),
        "the preflight must skip the gate"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_reports_missing_git_in_worktree_mode() {
    let root = temporary_directory();
    let bin = fixture_bin(&root);
    // PATH without git: the preflight must stop before gitleaks runs.
    let output = Command::new(runner())
        .env("PATH", &bin)
        .env("TMPDIR", root.join("tmp"))
        .env("LEADLINE_SECRET_MODE", "worktree")
        .env("LEADLINE_BIN", bin.join("leadline"))
        .env("LEADLINE_EXIT", "1")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(127));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("git not found"), "{stderr}");
    assert!(
        !root.join("gitleaks.args").exists(),
        "the preflight must skip the scanner"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_never_prints_raw_report() {
    let root = temporary_directory();
    fixture_bin(&root);
    init_repository(&root);
    for mode in ["staged", "worktree"] {
        let output = run_runner(&root, mode, &[("LEADLINE_EXIT", "0")]);
        assert_eq!(output.status.code(), Some(0));
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stdout.contains("SECRET_SENTINEL"), "stdout: {stdout}");
        assert!(!stderr.contains("SECRET_SENTINEL"), "stderr: {stderr}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_cleans_temporary_file() {
    let root = temporary_directory();
    fixture_bin(&root);
    let output = run_runner(&root, "staged", &[("LEADLINE_EXIT", "0")]);
    assert_eq!(output.status.code(), Some(0));
    let leftovers: Vec<_> = std::fs::read_dir(root.join("tmp"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_fails_when_gitleaks_is_missing() {
    let root = temporary_directory();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    fake_leadline(&bin);
    for tool in ["mktemp", "rm"] {
        let system = ["/usr/bin", "/bin"]
            .iter()
            .map(|dir| format!("{dir}/{tool}"))
            .find(|candidate| Path::new(candidate).exists())
            .unwrap_or_else(|| panic!("system {tool} not found"));
        std::os::unix::fs::symlink(system, bin.join(tool)).unwrap();
    }
    let tmp = root.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let output = Command::new(runner())
        .env("PATH", &bin)
        .env("TMPDIR", &tmp)
        .env("LEADLINE_SECRET_MODE", "staged")
        .env("LEADLINE_BIN", bin.join("leadline"))
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(127));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_maps_scanner_failure_to_input_error() {
    let root = temporary_directory();
    fixture_bin(&root);
    let output = run_runner(&root, "staged", &[("EXIT_CODE", "2")]);
    assert_eq!(output.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("SECRET_SENTINEL"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn secret_runner_rejects_bad_mode() {
    let root = temporary_directory();
    fixture_bin(&root);
    let output = run_runner(&root, "bogus", &[]);
    assert_eq!(output.status.code(), Some(2));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pre_commit_delegates_to_staged_runner() {
    let root = temporary_directory();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // Fake runner: records the mode it receives, exits 1 like a finding.
    write_executable(
        &bin.join("runner"),
        r#"#!/bin/sh
printf '%s\n' "$LEADLINE_SECRET_MODE" > "$MODE_FILE"
exit 1
"#,
    );
    // The hook resolves the repo root via git; give it a repository.
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["init", "-q"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let hook = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("integrations/git-hooks/pre-commit");
    let output = Command::new(&hook)
        .env("LEADLINE_SECRET_RUNNER", bin.join("runner"))
        .env("MODE_FILE", root.join("mode"))
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        std::fs::read_to_string(root.join("mode")).unwrap(),
        "staged\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn hook_manifest_text(path: &str) -> serde_json::Value {
    let text =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn native_hook_manifests_register_secret_gate_once() {
    let claude = hook_manifest_text("integrations/claude-code/hooks/hooks.json");
    let stop = claude["hooks"]["Stop"].as_array().unwrap();
    let secret_stops: Vec<_> = stop
        .iter()
        .flat_map(|entry| entry["hooks"].as_array().unwrap().iter())
        .filter(|hook| {
            hook["command"]
                .as_str()
                .is_some_and(|command| command.contains("leadline-secret-check.sh"))
        })
        .collect();
    assert_eq!(secret_stops.len(), 1);
    let gemini = hook_manifest_text("integrations/gemini/hooks/hooks.json");
    let after = gemini["hooks"]["AfterAgent"].as_array().unwrap();
    let secret_afters: Vec<_> = after
        .iter()
        .flat_map(|group| group["hooks"].as_array().unwrap().iter())
        .filter(|hook| {
            hook["command"]
                .as_str()
                .is_some_and(|command| command.contains("leadline-secret-check.sh"))
        })
        .collect();
    assert_eq!(secret_afters.len(), 1);
    // Gemini extension hooks must resolve scripts from the extension itself.
    assert!(
        secret_afters[0]["command"]
            .as_str()
            .unwrap()
            .contains("${extensionPath}"),
        "Gemini hooks must use ${{extensionPath}}"
    );
    let cline = hook_manifest_text("integrations/cline/plugin/cline-hooks.example.json");
    let secret_clines: Vec<_> = cline["hooks"]
        .as_object()
        .unwrap()
        .values()
        .filter(|hook| {
            hook["command"]
                .as_str()
                .is_some_and(|command| command.contains("leadline-secret-check.sh"))
        })
        .collect();
    assert_eq!(secret_clines.len(), 1);
}

/// Run a hook script the test just copied into place. Linux refuses `exec`
/// with `ETXTBSY` while a concurrently forking test thread still holds the
/// script's open write descriptor, so retry that transient error instead of
/// failing the suite.
fn run_copied_hook(program: &Path, cwd: &Path, configure: impl Fn(&mut Command)) -> Output {
    for _ in 0..10 {
        let mut command = Command::new(program);
        command.current_dir(cwd);
        configure(&mut command);
        match command.output() {
            Err(error) if error.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => return result.unwrap(),
        }
    }
    panic!("hook script stayed busy (ETXTBSY) after retries");
}

#[test]
fn native_hook_wrappers_delegate_worktree_mode() {
    let root = temporary_directory();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    write_executable(
        &bin.join("runner"),
        r#"#!/bin/sh
printf '%s\n' "$LEADLINE_SECRET_MODE" > "$MODE_FILE"
echo "runner diagnostic" >&2
exit "$RUNNER_EXIT"
"#,
    );
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for wrapper in [
        "integrations/claude-code/hooks/leadline-secret-check.sh",
        "integrations/gemini/hooks/leadline-secret-check.sh",
        "integrations/cline/plugin/leadline-secret-check.sh",
    ] {
        // Roots containing spaces must survive quoting.
        let spaced = root.join("with space");
        std::fs::create_dir_all(&spaced).unwrap();
        let local = spaced.join("leadline-secret-check.sh");
        std::fs::copy(manifest.join(wrapper), &local).unwrap();
        let mut permissions = std::fs::metadata(&local).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&local, permissions).unwrap();
        // Without the override, without a vendored runner, and without the
        // shared directory the wrapper surfaces a visible non-blocking error
        // (exit 1) so a packaging miss cannot loop a Stop hook.
        let missing = spaced.join("missing");
        std::fs::create_dir_all(&missing).unwrap();
        let missing_local = missing.join("leadline-secret-check.sh");
        std::fs::copy(manifest.join(wrapper), &missing_local).unwrap();
        let mut missing_permissions = std::fs::metadata(&missing_local).unwrap().permissions();
        missing_permissions.set_mode(0o755);
        std::fs::set_permissions(&missing_local, missing_permissions).unwrap();
        let output = run_copied_hook(&missing_local, &spaced, |command| {
            command.env("PATH", "/usr/bin:/bin");
        });
        assert_eq!(output.status.code(), Some(1), "{wrapper}");
        assert!(
            !String::from_utf8_lossy(&output.stderr).is_empty(),
            "{wrapper}: an unavailable runner must say why"
        );
        // A packaged extension vendors the runner at `<hook>/../common`; the
        // wrapper must prefer that copy over the repository layout.
        let packaged = spaced.join("packaged");
        std::fs::create_dir_all(packaged.join("hooks")).unwrap();
        std::fs::create_dir_all(packaged.join("common")).unwrap();
        let packaged_local = packaged.join("hooks/leadline-secret-check.sh");
        std::fs::copy(manifest.join(wrapper), &packaged_local).unwrap();
        let mut packaged_permissions = std::fs::metadata(&packaged_local).unwrap().permissions();
        packaged_permissions.set_mode(0o755);
        std::fs::set_permissions(&packaged_local, packaged_permissions).unwrap();
        let vendored = packaged.join("common/leadline-secret-check.sh");
        std::fs::copy(bin.join("runner"), &vendored).unwrap();
        let mut vendored_permissions = std::fs::metadata(&vendored).unwrap().permissions();
        vendored_permissions.set_mode(0o755);
        std::fs::set_permissions(&vendored, vendored_permissions).unwrap();
        let output = run_copied_hook(&packaged_local, &spaced, |command| {
            command
                .env("MODE_FILE", spaced.join("mode_packaged"))
                .env("RUNNER_EXIT", "0")
                .env("PATH", "/usr/bin:/bin");
        });
        assert_eq!(output.status.code(), Some(0), "{wrapper}");
        assert_eq!(
            std::fs::read_to_string(spaced.join("mode_packaged")).unwrap(),
            "worktree\n",
            "{wrapper}"
        );
        // A copied extension no longer sits beside integrations/common; the
        // host project's checkout still provides the runner.
        let project = spaced.join("project");
        std::fs::create_dir_all(project.join("integrations/common")).unwrap();
        let project_runner = project.join("integrations/common/leadline-secret-check.sh");
        std::fs::copy(bin.join("runner"), &project_runner).unwrap();
        let mut runner_permissions = std::fs::metadata(&project_runner).unwrap().permissions();
        runner_permissions.set_mode(0o755);
        std::fs::set_permissions(&project_runner, runner_permissions).unwrap();
        let fallback = spaced.join("fallback");
        std::fs::create_dir_all(&fallback).unwrap();
        let fallback_local = fallback.join("leadline-secret-check.sh");
        std::fs::copy(manifest.join(wrapper), &fallback_local).unwrap();
        let mut fallback_permissions = std::fs::metadata(&fallback_local).unwrap().permissions();
        fallback_permissions.set_mode(0o755);
        std::fs::set_permissions(&fallback_local, fallback_permissions).unwrap();
        let output = run_copied_hook(&fallback_local, &spaced, |command| {
            command
                .env("GEMINI_PROJECT_DIR", &project)
                .env("MODE_FILE", spaced.join("mode_project"))
                .env("RUNNER_EXIT", "0")
                .env("PATH", "/usr/bin:/bin");
        });
        assert_eq!(output.status.code(), Some(0), "{wrapper}");
        assert_eq!(
            std::fs::read_to_string(spaced.join("mode_project")).unwrap(),
            "worktree\n",
            "{wrapper}"
        );
        // Findings (runner exit 1) reach the host as exit 2, the only
        // blocking status in the shared hook protocol. Clean runs exit 0;
        // an unavailable scanner (127), a failed scan (4), or a missing git
        // comparison target (3) stays visible and non-blocking (exit 1).
        for (exit, wrapper_exit, mode_file) in [
            (0, 0, "mode_zero"),
            (1, 2, "mode_one"),
            (3, 1, "mode_three"),
            (4, 1, "mode_four"),
            (127, 1, "mode_unavailable"),
        ] {
            let output = run_copied_hook(&local, &spaced, |command| {
                command
                    .env("LEADLINE_SECRET_RUNNER", bin.join("runner"))
                    .env("MODE_FILE", spaced.join(mode_file))
                    .env("RUNNER_EXIT", exit.to_string())
                    .env("PATH", "/usr/bin:/bin");
            });
            assert_eq!(output.status.code(), Some(wrapper_exit), "{wrapper}");
            assert_eq!(
                std::fs::read_to_string(spaced.join(mode_file)).unwrap(),
                "worktree\n",
                "{wrapper}"
            );
            if exit != 0 {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("runner diagnostic"),
                    "{wrapper}: runner exit {exit} must surface its diagnostics"
                );
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// The Claude Code marketplace copies only the plugin directory, so the
/// plugin ships its own runner copy; keep it byte-identical to the canonical
/// shared runner or the packaged gate silently drifts.
#[test]
fn claude_plugin_ships_the_shared_runner() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let canonical =
        std::fs::read(manifest.join("integrations/common/leadline-secret-check.sh")).unwrap();
    let vendored =
        std::fs::read(manifest.join("integrations/claude-code/common/leadline-secret-check.sh"))
            .expect("the Claude Code plugin must vendor the shared runner");
    assert_eq!(
        vendored, canonical,
        "the vendored runner drifted from integrations/common"
    );
}

#[test]
fn typescript_adapters_register_shared_secret_gate() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let core =
        std::fs::read_to_string(manifest.join("integrations/agent-adapter-ts/core/index.ts"))
            .unwrap();
    assert!(core.contains("export async function runSecretGate"));
    assert!(core.contains("execFile"));
    assert!(!core.contains("shell: true"));
    assert!(core.contains("LEADLINE_SECRET_MODE"));
    // integrations/pi/index.ts re-exports the adapter registration, so the
    // gate flows through with no change there.
    for adapter in [
        "integrations/agent-adapter-ts/pi/index.ts",
        "integrations/opencode/plugin-v2/index.ts",
        "integrations/opencode/plugin/leadline.ts",
    ] {
        let text = std::fs::read_to_string(manifest.join(adapter)).unwrap();
        assert_eq!(
            text.matches("name: \"leadline_secret_check\"").count(),
            1,
            "{adapter} must register the gate exactly once"
        );
        // Explicit tools must surface an unavailable scanner, never "clean":
        // the shared formatter owns the three-state mapping.
        assert!(
            text.contains("secretGateMessage"),
            "{adapter} must use the shared status formatter"
        );
        assert!(
            !text.contains("leadline secret gate: clean"),
            "{adapter} must not collapse unavailable to clean"
        );
    }
    assert!(core.contains("leadline secret gate unavailable"));
}

#[test]
fn compatibility_matrix_names_secret_gate_mode() {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("integrations/COMPATIBILITY.md"),
    )
    .unwrap();
    let rows: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with('|'))
        .skip(2)
        .collect();
    assert!(rows.len() >= 10, "matrix must list every host");
    for row in rows {
        let lower = row.to_ascii_lowercase();
        assert!(
            lower.contains("block")
                || lower.contains("warn after edit")
                || lower.contains("manual tool"),
            "row must name its secret-gate mode: {row}"
        );
        assert!(
            !lower.contains("supported"),
            "row must not hide behind vague supported text: {row}"
        );
    }
}
