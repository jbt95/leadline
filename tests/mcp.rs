mod common;

use leadline::mcp::handle_request;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn request(method: &str, params: Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string()
}

/// Call one analyzer tool from inside an `execute` script.
///
/// This is the only route a client has to the twenty tools, so every behavior
/// test goes through the script sandbox too. `serde_json` renders arguments as
/// compact JSON, which is also a valid JavaScript object literal.
fn call_tool(name: &str, arguments: Value) -> Value {
    let code = format!("return await tools.{name}({arguments});");
    let raw = request(
        "tools/call",
        serde_json::json!({ "name": "execute", "arguments": { "code": code } }),
    );
    let response = handle_request(&raw).expect("tools/call must respond");
    serde_json::from_str(&response).unwrap()
}

/// Run a raw script body through `execute`.
fn execute_code(code: &str) -> Value {
    let raw = request(
        "tools/call",
        serde_json::json!({ "name": "execute", "arguments": { "code": code } }),
    );
    let response = handle_request(&raw).expect("tools/call must respond");
    serde_json::from_str(&response).unwrap()
}

/// The `execute` tool spec from `tools/list`.
fn execute_spec() -> Value {
    let response: Value = serde_json::from_str(
        &handle_request(&request("tools/list", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    let tools = result_of(&response)["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1, "only `execute` is advertised: {tools:?}");
    assert_eq!(tools[0]["name"], "execute");
    tools[0].clone()
}

/// A `tools/list` response, for assertions that need the whole array.
fn execute_list_response() -> Value {
    serde_json::from_str(&handle_request(&request("tools/list", serde_json::json!({}))).unwrap())
        .unwrap()
}

/// The generated declaration block inside `execute`'s description.
fn execute_declarations() -> String {
    execute_spec()["description"]
        .as_str()
        .expect("execute must carry a description")
        .to_owned()
}

fn result_of(response: &Value) -> &Value {
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected result, got {response}"));
    // Tools/call results carry the MCP CallToolResult envelope; handshake
    // and tools/list results keep their plain shape.
    result.get("structuredContent").unwrap_or(result)
}

fn error_of(response: &Value) -> &Value {
    response
        .get("error")
        .unwrap_or_else(|| panic!("expected error, got {response}"))
}

fn fixture_dir(source: &str) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("leadline-mcp-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sample.ts"), source).unwrap();
    dir
}

/// Repository fixture directory that removes itself when the test ends,
/// including on panic, so interrupted runs do not leave directories inside
/// the repository.
struct Fixture(PathBuf);

impl Fixture {
    fn create(path: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl std::ops::Deref for Fixture {
    type Target = std::path::Path;

    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Fixture {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git_fixture() -> Fixture {
    let root = Fixture::create(common::temporary_directory());
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "test@example.com"]);
    run_git(&root, &["config", "user.name", "Test"]);
    root
}

fn run_git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn envelope_ok(result: &Value) {
    assert_eq!(result["schema_version"], 2);
    assert!(result["analyzer_version"].is_string());
    for metric in [
        "cyclomatic",
        "cognitive",
        "halstead",
        "maintainability",
        "crap",
    ] {
        assert_eq!(result["metric_specs"][metric], "default");
    }
}

#[test]
fn initialize_and_tools_list() {
    let response: Value = serde_json::from_str(
        &handle_request(&request("initialize", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    assert_eq!(result_of(&response)["serverInfo"]["name"], "leadline");

    // One advertised tool, not twenty: the analyzer surface reaches the model
    // as the declaration block in `execute`'s description.
    let spec = execute_spec();
    assert_eq!(spec["inputSchema"]["required"][0], "code");
    assert_eq!(spec["inputSchema"]["properties"]["code"]["type"], "string");

    // The declaration block must be a real, non-trivial API listing. Which
    // tools it contains is the crate's own invariant, covered by
    // `mcp::tests::declarations_list_every_tool_with_its_arguments`; repeating
    // the twenty names here would only create a second list to keep in sync.
    let declarations = execute_declarations();
    let callable = declarations
        .lines()
        .filter(|line| {
            line.trim_start().starts_with("analyze(")
                || line.trim_start().starts_with("check(")
                || line.trim_start().starts_with("project(")
                || line.trim_start().starts_with("repo_summary(")
        })
        .count();
    assert_eq!(
        callable, 4,
        "declarations must be callable lines: {declarations}"
    );
    assert!(declarations.contains("Promise<object>"), "{declarations}");
}

#[test]
fn direct_analyzer_call_is_rejected_with_a_redirect() {
    for name in ["analyze", "check", "project"] {
        let raw = request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": {} }),
        );
        let response: Value = serde_json::from_str(&handle_request(&raw).unwrap()).unwrap();
        let error = error_of(&response);
        assert_eq!(error["code"], -32602, "{name}: {error}");
        let message = error["message"].as_str().unwrap();
        assert!(message.contains("execute"), "{name}: {message}");
    }
}

#[test]
fn execute_runs_a_script_over_several_tools() {
    let dir = fixture_dir("function branch(x: boolean) { if (x) return 1; return 0; }\n");
    let path = dir.to_str().unwrap();
    // The whole point of the surface: one call, several tools, one result.
    let response = execute_code(&format!(
        "const [summary, risk] = await Promise.all([
           tools.repo_summary({{ path: {path:?} }}),
           tools.risk({{ path: {path:?} }}),
         ]);
         return {{ files: summary.totals.files, top: risk.risks[0].path }};"
    ));
    let result = result_of(&response);
    assert_eq!(result["files"], 1);
    assert_eq!(result["top"], "sample.ts");
}

#[test]
fn execute_reports_tool_failures_with_the_original_message() {
    let response = execute_code("return await tools.explain_metric({ metric: \"vibes\" });");
    let error = error_of(&response);
    assert_eq!(error["code"], -32602);
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("vibes"), "{message}");
    assert!(message.contains("script error"), "{message}");
}

#[test]
fn execute_denies_host_capabilities() {
    // The script holds `tools` and nothing else: no require, no process, no
    // fetch, no timers.
    let response =
        execute_code("return [typeof require, typeof process, typeof fetch, typeof setTimeout];");
    assert_eq!(
        result_of(&response),
        &serde_json::json!(["undefined", "undefined", "undefined", "undefined"])
    );
}

#[test]
fn execute_enforces_the_tool_call_budget() {
    let response = execute_code(
        "for (let i = 0; i < 200; i++) { await tools.explain_metric({ metric: \"cognitive\" }); }
         return \"unreachable\";",
    );
    let message = error_of(&response)["message"].as_str().unwrap().to_owned();
    assert!(message.contains("tool call limit"), "{message}");
}

#[test]
fn execute_requires_code_and_rejects_unknown_fields() {
    for arguments in [serde_json::json!({}), serde_json::json!({ "code": 1 })] {
        let raw = request(
            "tools/call",
            serde_json::json!({ "name": "execute", "arguments": arguments }),
        );
        let response: Value = serde_json::from_str(&handle_request(&raw).unwrap()).unwrap();
        assert_eq!(error_of(&response)["code"], -32602, "{arguments}");
    }
    let raw = request(
        "tools/call",
        serde_json::json!({
            "name": "execute",
            "arguments": { "code": "return 1;", "bogus": 1 }
        }),
    );
    let response: Value = serde_json::from_str(&handle_request(&raw).unwrap()).unwrap();
    assert_eq!(error_of(&response)["code"], -32602);
}

#[test]
fn execute_reports_a_javascript_syntax_error() {
    let response = execute_code("const = ;");
    let message = error_of(&response)["message"].as_str().unwrap();
    assert!(message.contains("variable name expected"), "{message}");
}

#[test]
fn notifications_produce_no_response() {
    assert_eq!(
        handle_request(r#"{"jsonrpc":"2.0","method":"initialized"}"#),
        None
    );
    assert_eq!(
        handle_request(r#"{"jsonrpc":"2.0","method":"notifications/progress","params":{}}"#),
        None
    );
    assert_eq!(handle_request("   "), None);
}

#[test]
fn malformed_json_reports_parse_error_with_null_id() {
    let response: Value = serde_json::from_str(&handle_request("{nope").unwrap()).unwrap();
    assert_eq!(error_of(&response)["code"], -32700);
    assert!(response["id"].is_null());
}

#[test]
fn unknown_method_reports_not_found() {
    let response: Value = serde_json::from_str(
        &handle_request(&request("frobnicate", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    assert_eq!(error_of(&response)["code"], -32601);
}

#[test]
fn invalid_params_reports_invalid_params() {
    // analyze_function requires `function`.
    let response = call_tool(
        "analyze_function",
        serde_json::json!({ "path": "sample.ts" }),
    );
    assert_eq!(error_of(&response)["code"], -32602);

    // Unknown tool name.
    let raw = request(
        "tools/call",
        serde_json::json!({ "name": "rewrite_codebase", "arguments": {} }),
    );
    let response: Value = serde_json::from_str(&handle_request(&raw).unwrap()).unwrap();
    assert_eq!(error_of(&response)["code"], -32602);
}

#[test]
fn batch_requests_return_batch_responses() {
    let batch = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "ping", "params": {} },
        { "jsonrpc": "2.0", "id": 2, "method": "nope", "params": {} },
        { "jsonrpc": "2.0", "method": "notifications/tick", "params": {} },
    ])
    .to_string();
    let response: Value = serde_json::from_str(&handle_request(&batch).unwrap()).unwrap();
    let items = response.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items[0].get("result").is_some());
    assert_eq!(items[1]["error"]["code"], -32601);
}

#[test]
fn tools_call_results_use_call_tool_result_envelope() {
    let dir = fixture_dir("function calc(x: number) { return x; }\n");
    let response = call_tool(
        "analyze",
        serde_json::json!({ "path": dir.to_str().unwrap() }),
    );
    let result = result_of(&response);
    assert!(result["schema_version"].is_number());
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tools/call success must carry MCP text content");
    let from_text: Value = serde_json::from_str(text).unwrap();
    assert_eq!(from_text["schema_version"], result["schema_version"]);
    assert_eq!(from_text["tool"], "analyze");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn analyze_returns_compact_deterministic_envelope() {
    let dir = fixture_dir("function branch(x: boolean) { if (x) return 1; return 0; }\n");
    let path = dir.to_str().unwrap();
    let first = call_tool("analyze", serde_json::json!({ "path": path }));
    let second = call_tool("analyze", serde_json::json!({ "path": path }));
    assert_eq!(first, second);

    let result = result_of(&first);
    envelope_ok(result);
    let functions = result["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 1);
    let row = &functions[0];
    assert_eq!(row["name"], "branch");
    assert_eq!(row["line"], 1);
    assert_eq!(row["cyclomatic"], 2);
    assert!(row["cognitive"].is_number());
    assert_eq!(result["truncated"], false);
    assert_eq!(result["total"], 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn analyze_accepts_coverage_through_a_script() {
    let dir = fixture_dir("function covered(x: boolean) { if (x) return 1; return 0; }\n");
    let file = dir.join("sample.ts");
    let lcov = dir.join("lcov.info");
    std::fs::write(&lcov, "SF:sample.ts\nDA:1,1\nDA:2,0\nend_of_record\n").unwrap();
    let response = call_tool(
        "analyze",
        serde_json::json!({ "path": dir.to_str().unwrap(), "coverage": lcov.to_str().unwrap() }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["functions"].as_array().unwrap().len(), 1);
    assert!(file.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn analyze_function_finds_single_function() {
    let dir = fixture_dir("function alpha() { return 1; }\nfunction beta() { return 2; }\n");
    let path = dir.join("sample.ts").to_str().unwrap().to_owned();
    let response = call_tool(
        "analyze_function",
        serde_json::json!({ "path": path, "function": "beta" }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    let functions = result["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0]["name"], "beta");

    let missing = call_tool(
        "analyze_function",
        serde_json::json!({ "path": dir.join("sample.ts").to_str().unwrap(), "function": "ghost" }),
    );
    assert_eq!(error_of(&missing)["code"], -32602);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_reports_violations_and_pass_state() {
    let dir = fixture_dir("function branch(x: boolean) { if (x) return 1; return 0; }\n");
    let path = dir.to_str().unwrap();
    let failing = call_tool(
        "check",
        serde_json::json!({ "path": path, "thresholds": { "cyclomatic": 1 } }),
    );
    let result = result_of(&failing);
    envelope_ok(result);
    assert_eq!(result["passed"], false);
    assert_eq!(result["violations"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["violations"][0]["reason"],
        serde_json::json!(["cyclomatic"])
    );

    let passing = call_tool(
        "check",
        serde_json::json!({ "path": path, "thresholds": { "cyclomatic": 10 } }),
    );
    assert_eq!(result_of(&passing)["passed"], true);

    let no_thresholds = call_tool("check", serde_json::json!({ "path": path }));
    assert_eq!(error_of(&no_thresholds)["code"], -32602);
    assert!(
        error_of(&no_thresholds)["message"]
            .as_str()
            .unwrap()
            .contains("thresholds"),
        "{no_thresholds}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explain_metric_covers_all_five_and_rejects_unknown() {
    for metric in [
        "cyclomatic",
        "cognitive",
        "halstead",
        "maintainability",
        "crap",
    ] {
        let response = call_tool("explain_metric", serde_json::json!({ "metric": metric }));
        let result = result_of(&response);
        envelope_ok(result);
        assert_eq!(result["metric"], metric);
        assert_eq!(result["spec"], "default");
        assert!(result["definition"].as_str().unwrap().contains("default"));
    }
    let unknown = call_tool("explain_metric", serde_json::json!({ "metric": "vibes" }));
    assert_eq!(error_of(&unknown)["code"], -32602);
}

#[test]
fn analyze_changed_reports_deltas() {
    let root = common::temporary_directory();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(
        root.join("sample.ts"),
        "function calc(x: boolean) { return 1; }\n",
    )
    .unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "base"]);
    std::fs::write(
        root.join("sample.ts"),
        "function calc(x: boolean) { if (x) return 1; return 0; }\n",
    )
    .unwrap();

    let response = call_tool(
        "analyze_changed",
        serde_json::json!({ "base": "HEAD", "path": root.to_str().unwrap() }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    let changes = result["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    let change = &changes[0];
    assert_eq!(change["name"], "calc");
    assert!(change["before"].is_object());
    assert!(change["after"].is_object());
    assert_eq!(change["delta"]["cyclomatic"], 1);
    assert!(change.get("causes").is_none());
    assert_eq!(result["truncated"], false);

    let explained = call_tool(
        "analyze_changed",
        serde_json::json!({
            "base": "HEAD",
            "path": root.to_str().unwrap(),
            "explain": true,
        }),
    );
    let result = result_of(&explained);
    assert_eq!(result["changes"][0]["causes"][0]["line"], 1);
    assert_eq!(result["changes"][0]["causes"][0]["rule"], "if");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analyze_changed_non_worktree_targets_ignore_worktree() {
    let root = common::temporary_directory();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(
        root.join("sample.ts"),
        "function calc(x: boolean) { return 1; }\n",
    )
    .unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "base"]);
    std::fs::write(
        root.join("sample.ts"),
        "function calc(x: boolean) { if (x) return 1; return 0; }\n",
    )
    .unwrap();
    run(&["add", "."]);
    std::fs::write(
        root.join("sample.ts"),
        "function calc(x: boolean) { if (x) { if (!x) return 2; return 1; } return 0; }\n",
    )
    .unwrap();

    let response = call_tool(
        "analyze_changed",
        serde_json::json!({ "base": "HEAD", "path": root.to_str().unwrap(), "target": "index" }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    let changes = result["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["delta"]["cyclomatic"], 1);

    run(&["commit", "-m", "target"]);
    let response = call_tool(
        "analyze_changed",
        serde_json::json!({
            "base": "HEAD~1",
            "path": root.to_str().unwrap(),
            "target": "HEAD"
        }),
    );
    let changes = result_of(&response)["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["delta"]["cyclomatic"], 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analyze_changed_renames_pairs_git_detected_rename() {
    let root = common::temporary_directory();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(
        root.join("sample.ts"),
        "function keep(): number {\n  return 1;\n}\n\nfunction calc(x: boolean) {\n  return 1;\n}\n",
    )
    .unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "base"]);
    run(&["mv", "sample.ts", "sample2.ts"]);
    std::fs::write(
        root.join("sample2.ts"),
        "function keep(): number {\n  return 1;\n}\n\nfunction calc(x: boolean) {\n  if (x) return 1;\n  return 0;\n}\n",
    )
    .unwrap();

    let response = call_tool(
        "analyze_changed",
        serde_json::json!({ "base": "HEAD", "path": root.to_str().unwrap(), "renames": true }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    let changes = result["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "sample2.ts");
    assert_eq!(changes[0]["name"], "calc");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analyze_changed_rejects_option_like_target() {
    let response = call_tool(
        "analyze_changed",
        serde_json::json!({ "base": "HEAD", "target": "--evil" }),
    );
    assert_eq!(error_of(&response)["code"], -32602);
}

#[test]
fn analyze_truncates_large_results_explicitly() {
    let dir = std::env::temp_dir().join(format!(
        "leadline-mcp-trunc-{}-{}",
        std::process::id(),
        AtomicU64::new(0).fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut source = String::new();
    for index in 0..250 {
        source.push_str(&format!("function fn{index}() {{ return {index}; }}\n"));
    }
    std::fs::write(dir.join("big.ts"), source).unwrap();

    let response = call_tool(
        "analyze",
        serde_json::json!({ "path": dir.to_str().unwrap() }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["functions"].as_array().unwrap().len(), 200);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["total"], 250);

    // Order is deterministic across calls.
    let again = call_tool(
        "analyze",
        serde_json::json!({ "path": dir.to_str().unwrap() }),
    );
    assert_eq!(response, again);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_accepts_regression_limits_without_absolute_thresholds() {
    let root = std::env::temp_dir().join(format!(
        "leadline-mcp-regressions-{}-{}",
        std::process::id(),
        AtomicU64::new(0).fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
    };
    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { return x; }\n",
    )
    .unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    let response = call_tool(
        "check",
        serde_json::json!({ "path": root.to_str().unwrap(), "base": "HEAD", "regressions": true }),
    );
    let result = result_of(&response);
    assert_eq!(result["passed"], false);
    assert_eq!(
        result["violations"][0]["reason"],
        serde_json::json!(["regression"])
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_rejects_regressions_without_base() {
    let dir = fixture_dir("function calc(x: number) { return x; }\n");
    let response = call_tool(
        "check",
        serde_json::json!({
            "path": dir.to_str().unwrap(),
            "regressions": true,
        }),
    );
    assert_eq!(error_of(&response)["code"], -32602);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn execute_declarations_carry_every_tool_and_its_arguments() {
    let declarations = execute_declarations();
    for (name, argument) in [
        ("analyze", "sort_by"),
        ("analyze_changed", "min_delta"),
        ("analyze_function", "explain"),
        ("check", "thresholds"),
        ("repo_summary", "coverage"),
        ("test_targets", "coverage"),
        ("impact", "target"),
        ("coupling", "min_cochanges"),
        ("hotspots", "since"),
        ("sql_plan", "max_cost_increase_percent"),
        ("security_findings", "minimum_severity"),
        ("vulnerabilities", "osv"),
        ("sql_risks", "migration_roots"),
        ("project", "test_map"),
    ] {
        assert!(
            declarations.contains(&format!("{name}({{")) && declarations.contains(argument),
            "{name} must be declared with {argument}, got: {declarations}"
        );
    }
    // `explain_metric` takes no options beyond the metric itself.
    assert!(declarations.contains("explain_metric({"), "{declarations}");
}

#[test]
fn test_targets_requires_coverage() {
    let dir = fixture_dir("function branch(x: boolean) { if (x) return 1; return 0; }\n");
    let response = call_tool(
        "test_targets",
        serde_json::json!({ "path": dir.to_str().unwrap() }),
    );
    assert_eq!(error_of(&response)["code"], -32602);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn test_targets_returns_uncovered_rows_and_caps() {
    let dir = fixture_dir("function alpha(x: boolean) { if (x) return 1; return 0; }\n");
    std::fs::write(
        dir.join("second.ts"),
        "function beta(x: boolean) { if (x) return 2; return 0; }\n",
    )
    .unwrap();
    let lcov = dir.join("lcov.info");
    std::fs::write(
        &lcov,
        "TN:\nSF:sample.ts\nDA:1,0\nend_of_record\nTN:\nSF:second.ts\nDA:1,0\nend_of_record\n",
    )
    .unwrap();
    let response = call_tool(
        "test_targets",
        serde_json::json!({ "path": dir.to_str().unwrap(), "coverage": lcov.to_str().unwrap(), "top": 1 }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    let targets = result["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert!(!targets[0]["uncovered"].as_array().unwrap().is_empty());
    assert_eq!(result["truncated"], true);
    assert_eq!(result["total"], 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_gates_baseline_regressions() {
    let dir = fixture_dir("function calc(x: boolean) { return 1; }\n");
    let path = dir.to_str().unwrap().to_owned();
    let report = leadline::analyze_path(std::path::Path::new(&path), None).unwrap();
    let snapshot = dir.join("baseline.json");
    leadline::baseline::Baseline::from_report(&report)
        .write(&snapshot)
        .unwrap();
    std::fs::write(
        dir.join("sample.ts"),
        "function calc(x: boolean) { if (x) { if (!x) { return 2; } return 1; } return 0; }\n",
    )
    .unwrap();

    let failing = call_tool(
        "check",
        serde_json::json!({ "path": path, "baseline": snapshot.to_str().unwrap(), "regressions": true }),
    );
    let result = result_of(&failing);
    envelope_ok(result);
    assert_eq!(result["passed"], false);
    assert_eq!(result["violations"].as_array().unwrap().len(), 1);

    let passing = call_tool(
        "check",
        serde_json::json!({ "path": path, "baseline": snapshot.to_str().unwrap(), "regressions": { "cognitive": 100, "cyclomatic": 100, "crap": 100.0, "max_nesting": 100 } }),
    );
    assert_eq!(result_of(&passing)["passed"], true);

    let conflict = call_tool(
        "check",
        serde_json::json!({ "path": path, "base": "HEAD", "baseline": snapshot.to_str().unwrap(), "regressions": true }),
    );
    assert_eq!(error_of(&conflict)["code"], -32602);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_accepts_coverage_for_crap_gates() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) { return 1; } return 0; }\n");
    let path = dir.to_str().unwrap();
    let lcov = dir.join("lcov.info");
    std::fs::write(&lcov, "SF:sample.ts\nDA:1,1\nend_of_record\n").unwrap();

    let without = call_tool(
        "check",
        serde_json::json!({ "path": path, "thresholds": { "crap": 2.5 } }),
    );
    assert_eq!(
        result_of(&without)["passed"],
        false,
        "no coverage means CRAP is unavailable and must fail a CRAP gate"
    );

    let with = call_tool(
        "check",
        serde_json::json!({
            "path": path,
            "coverage": lcov.to_str().unwrap(),
            "thresholds": { "crap": 2.5 },
        }),
    );
    assert_eq!(result_of(&with)["passed"], true);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_reasons_distinguish_crap_from_unavailable_crap() {
    let dir = fixture_dir(
        "function covered(x: boolean) { if (x) { return 1; } return 0; }\nfunction uncovered(x: boolean) { if (x) { return 1; } return 0; }\n",
    );
    let path = dir.to_str().unwrap();
    let lcov = dir.join("lcov.info");
    std::fs::write(&lcov, "SF:sample.ts\nDA:1,1\nend_of_record\n").unwrap();

    let with = call_tool(
        "check",
        serde_json::json!({
            "path": path,
            "coverage": lcov.to_str().unwrap(),
            "thresholds": { "crap": 1.0 },
        }),
    );
    let result = result_of(&with);
    assert_eq!(result["passed"], false);
    let violations = result["violations"].as_array().unwrap();
    assert_eq!(violations.len(), 2, "{result}");
    let covered = violations
        .iter()
        .find(|row| row["name"] == "covered")
        .expect("covered function must be a violation");
    let uncovered = violations
        .iter()
        .find(|row| row["name"] == "uncovered")
        .expect("uncovered function must be a violation");
    assert_eq!(covered["reason"], serde_json::json!(["crap"]));
    assert_eq!(uncovered["reason"], serde_json::json!(["crap_unavailable"]));

    // Without coverage every function fails closed with the same reason.
    let without = call_tool(
        "check",
        serde_json::json!({ "path": path, "thresholds": { "crap": 1.0 } }),
    );
    let rows = result_of(&without)["violations"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        assert_eq!(row["reason"], serde_json::json!(["crap_unavailable"]));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_reasons_name_every_failed_threshold_in_gate_order() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) { return 1; } return 0; }\n");
    let path = dir.to_str().unwrap();

    let response = call_tool(
        "check",
        serde_json::json!({
            "path": path,
            "thresholds": { "cognitive": 0, "cyclomatic": 1 },
        }),
    );
    let result = result_of(&response);
    assert_eq!(
        result["violations"][0]["reason"],
        serde_json::json!(["cognitive", "cyclomatic"])
    );

    // A metric exactly at its limit is not above it.
    let boundary = call_tool(
        "check",
        serde_json::json!({
            "path": path,
            "thresholds": { "cognitive": 1, "cyclomatic": 2 },
        }),
    );
    assert_eq!(result_of(&boundary)["passed"], true);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_rejects_unknown_arguments_and_gate_keys() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) { return 1; } return 0; }\n");
    let path = dir.to_str().unwrap();

    let unknown_argument = call_tool(
        "check",
        serde_json::json!({
            "path": path,
            "thresholds": { "crap": 1.0 },
            "coverage_file": "target/site/jacoco/jacoco.xml",
        }),
    );
    let message = error_of(&unknown_argument)["message"]
        .as_str()
        .unwrap_or_default();
    assert!(
        message.contains("coverage_file"),
        "unknown argument must be named: {unknown_argument}"
    );

    let unknown_threshold = call_tool(
        "check",
        serde_json::json!({ "path": path, "thresholds": { "maintainability": 20 } }),
    );
    let message = error_of(&unknown_threshold)["message"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("maintainability"), "{message}");
    assert!(message.contains("cognitive"), "{message}");

    let unknown_regression = call_tool(
        "check",
        serde_json::json!({ "path": path, "base": "HEAD", "regressions": { "bogus": 1 } }),
    );
    assert_eq!(error_of(&unknown_regression)["code"], -32602);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn remaining_tools_reject_unknown_arguments() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) { return 1; } return 0; }\n");
    let path = dir.to_str().unwrap();
    let file = dir.join("sample.ts");
    let lcov = dir.join("lcov.info");
    std::fs::write(&lcov, "SF:sample.ts\nDA:1,1\nend_of_record\n").unwrap();

    for (tool, arguments) in [
        ("analyze", serde_json::json!({ "path": path, "bogus": 1 })),
        (
            "analyze_changed",
            serde_json::json!({ "path": path, "bogus": 1 }),
        ),
        (
            "analyze_function",
            serde_json::json!({ "path": file.to_str().unwrap(), "function": "calc", "bogus": 1 }),
        ),
        (
            "explain_metric",
            serde_json::json!({ "metric": "crap", "bogus": 1 }),
        ),
        (
            "repo_summary",
            serde_json::json!({ "path": path, "bogus": 1 }),
        ),
        (
            "test_targets",
            serde_json::json!({ "path": path, "coverage": lcov.to_str().unwrap(), "bogus": 1 }),
        ),
        (
            "dependencies",
            serde_json::json!({ "path": path, "bogus": 1 }),
        ),
        (
            "impact",
            serde_json::json!({ "target": "sample.ts", "bogus": 1 }),
        ),
        (
            "coupling",
            serde_json::json!({ "target": "sample.ts", "bogus": 1 }),
        ),
        ("hotspots", serde_json::json!({ "path": path, "bogus": 1 })),
        (
            "duplication",
            serde_json::json!({ "path": path, "bogus": 1 }),
        ),
        ("policy", serde_json::json!({ "path": path, "bogus": 1 })),
        ("risk", serde_json::json!({ "path": path, "bogus": 1 })),
        ("debt", serde_json::json!({ "path": path, "bogus": 1 })),
        ("project", serde_json::json!({ "path": path, "bogus": 1 })),
    ] {
        let response = call_tool(tool, arguments);
        let message = error_of(&response)["message"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(message.contains("unknown"), "{tool}: {response}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn graph_analytics_tools_are_reachable_and_named_in_instructions() {
    let declarations = execute_declarations();
    for name in ["dependencies", "impact", "coupling"] {
        assert!(declarations.contains(&format!("{name}(")), "{name} missing");
    }
    let instructions = handle_request(&request("initialize", serde_json::json!({}))).unwrap();
    for name in ["dependencies", "impact", "coupling"] {
        assert!(instructions.contains(name), "instructions must name {name}");
    }
}

#[test]
fn history_and_report_tools_are_reachable() {
    let declarations = execute_declarations();
    for name in ["hotspots", "duplication", "policy"] {
        assert!(declarations.contains(&format!("{name}(")), "{name} missing");
    }
}

#[test]
fn dependencies_reports_the_graph() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) return 1; return 0; }\n");
    std::fs::write(
        dir.join("helper.ts"),
        "export function helper() { return 1; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("sample.ts"),
        "import { helper } from './helper';\nfunction calc(x: boolean) { return helper(); }\n",
    )
    .unwrap();
    let response = call_tool(
        "dependencies",
        serde_json::json!({ "path": dir.to_str().unwrap() }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["summary"]["files"], 2);
    assert_eq!(result["summary"]["edges"], 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn impact_reports_dependents_and_rejects_a_missing_target() {
    let dir = fixture_dir("export function helper() { return 1; }\n");
    std::fs::write(
        dir.join("helper.ts"),
        "export function helper() { return 1; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("sample.ts"),
        "import { helper } from './helper';\nfunction calc(x: boolean) { return helper(); }\n",
    )
    .unwrap();
    let response = call_tool(
        "impact",
        serde_json::json!({ "target": "helper.ts", "path": dir.to_str().unwrap() }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    let dependents = result["dependents"].as_array().unwrap();
    assert!(
        dependents.iter().any(|row| row["path"] == "sample.ts"),
        "{response}"
    );

    let missing = call_tool(
        "impact",
        serde_json::json!({ "target": "nope.ts", "path": dir.to_str().unwrap() }),
    );
    assert_eq!(error_of(&missing)["code"], -32602);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn coupling_reports_related_files() {
    // Git fixture: two files committed together twice.
    let root = git_fixture();
    for (message, left, right) in [("one", "a", "b"), ("two", "a2", "b2")] {
        std::fs::write(root.join("a.ts"), format!("export const a = '{left}';\n")).unwrap();
        std::fs::write(root.join("b.ts"), format!("export const b = '{right}';\n")).unwrap();
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-m", message]);
    }
    let response = call_tool(
        "coupling",
        serde_json::json!({ "target": "a.ts", "path": root.to_str().unwrap(), "min_cochanges": 2 }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    let related = result["related"].as_array().unwrap();
    assert!(
        related.iter().any(|row| row["path"] == "b.ts"),
        "{response}"
    );

    let escape = root.parent().unwrap().join(format!(
        "{}-escape.ts",
        root.file_name().unwrap().to_string_lossy()
    ));
    std::fs::write(&escape, "export const escape = 1;\n").unwrap();
    let outside = call_tool(
        "coupling",
        serde_json::json!({ "target": escape.to_str().unwrap(), "path": root.to_str().unwrap() }),
    );
    assert_eq!(error_of(&outside)["code"], -32602);
    std::fs::remove_file(&escape).unwrap();
}

#[test]
fn hotspots_ranks_files_and_rejects_a_bad_window() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) return 1; return 0; }\n");
    let response = call_tool(
        "hotspots",
        serde_json::json!({ "path": dir.to_str().unwrap() }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert!(
        !result["hotspots"].as_array().unwrap().is_empty(),
        "{response}"
    );
    let bad = call_tool(
        "hotspots",
        serde_json::json!({ "path": dir.to_str().unwrap(), "since": "7d" }),
    );
    assert_eq!(error_of(&bad)["code"], -32602);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn duplication_reports_clones() {
    let dir = fixture_dir("function alpha(x: boolean) { if (x) { return 1; } return 0; }\n");
    std::fs::write(
        dir.join("twin.ts"),
        "function beta(x: boolean) { if (x) { return 1; } return 0; }\n",
    )
    .unwrap();
    let response = call_tool(
        "duplication",
        serde_json::json!({ "path": dir.to_str().unwrap() }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["tool"], "duplication");
    assert_eq!(result["mode"], "single");
    assert!(result["groups"].is_array(), "{response}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn policy_reports_violations_without_config() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) return 1; return 0; }\n");
    let response = call_tool(
        "policy",
        serde_json::json!({ "path": dir.to_str().unwrap() }),
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["tool"], "policy");
    assert!(result["violations"].as_array().is_some(), "{response}");
    assert!(result["rules"].is_number(), "{response}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn repo_summary_crap_list_requires_coverage() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) { return 1; } return 0; }\n");
    let path = dir.to_str().unwrap();

    let plain = call_tool("repo_summary", serde_json::json!({ "path": path }));
    let result = result_of(&plain);
    assert!(
        result["top"].get("crap").is_none(),
        "without coverage the crap list is omitted: {plain}"
    );
    assert!(result["top"]["cognitive"].is_array());

    let lcov = dir.join("lcov.info");
    std::fs::write(&lcov, "SF:sample.ts\nDA:1,0\nend_of_record\n").unwrap();
    let covered = call_tool(
        "repo_summary",
        serde_json::json!({ "path": path, "coverage": lcov.to_str().unwrap() }),
    );
    let crap = result_of(&covered)["top"]["crap"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(crap.len(), 1, "{covered}");
    assert_eq!(crap[0]["name"], "calc");
    assert_eq!(crap[0]["crap"], 6.0);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Tool names registered by a TypeScript adapter (`name: "leadline_..."`).
fn registered_tool_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in source.lines() {
        let Some(at) = line.find("name: \"") else {
            continue;
        };
        let rest = &line[at + "name: \"".len()..];
        let Some(end) = rest.find('"') else {
            continue;
        };
        let name = &rest[..end];
        if name.starts_with("leadline_") {
            names.push(name.to_owned());
        }
    }
    names
}

#[test]
fn native_tool_names_do_not_collide_with_mcp_tool_names() {
    use std::collections::BTreeSet;
    let mcp: BTreeSet<String> = result_of(&execute_list_response())["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    // Only `execute` is advertised, so this is the single namespaced MCP name a
    // host can shadow.
    assert_eq!(mcp, BTreeSet::from(["execute".to_owned()]));
    let namespaced: BTreeSet<String> = mcp.iter().map(|name| format!("leadline_{name}")).collect();
    assert!(namespaced.contains("leadline_execute"));
    // Harness MCP clients expose a server tool as `<server>_<tool>`, and the
    // leadline server is keyed `leadline`; a native tool with the same name is
    // silently shadowed, so the names must stay disjoint.
    let namespaced: BTreeSet<String> = mcp.iter().map(|name| format!("leadline_{name}")).collect();
    let expected = BTreeSet::from([
        "leadline_changed".to_owned(),
        "leadline_function".to_owned(),
        "leadline_gate".to_owned(),
        "leadline_secret_check".to_owned(),
    ]);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for plugin in [
        "integrations/opencode/plugin/leadline.ts",
        "integrations/opencode/plugin-v2/index.ts",
        "integrations/agent-adapter-ts/pi/index.ts",
    ] {
        let source = std::fs::read_to_string(root.join(plugin)).unwrap();
        let names: BTreeSet<String> = registered_tool_names(&source).into_iter().collect();
        for name in &names {
            assert!(
                !namespaced.contains(name),
                "{plugin} registers '{name}', which collides with a namespaced MCP tool"
            );
        }
        assert_eq!(
            names, expected,
            "{plugin} must register the same four native tools"
        );
    }
}

#[test]
fn analyze_accepts_budget_params() {
    let dir = fixture_dir(
        "function alpha(x: boolean) { return 1; }\nfunction beta(x: boolean) { if (x) { if (!x) { return 1; } return 2; } return 0; }\n",
    );
    let path = dir.to_str().unwrap();

    let top = call_tool(
        "analyze",
        serde_json::json!({ "path": path, "top": 1, "sort_by": "cognitive" }),
    );
    let result = result_of(&top);
    let functions = result["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0]["name"], "beta");
    assert_eq!(result["total"], 2);
    assert_eq!(result["truncated"], true);

    let lcov = dir.join("lcov.info");
    std::fs::write(&lcov, "SF:sample.ts\nDA:1,0\nDA:2,0\nend_of_record\n").unwrap();
    let filtered = call_tool(
        "analyze",
        serde_json::json!({
            "path": path,
            "coverage": lcov.to_str().unwrap(),
            "min_crap": 10.0,
        }),
    );
    let result = result_of(&filtered);
    let functions = result["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0]["name"], "beta");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn analyze_function_explains_contributions() {
    let dir = fixture_dir(
        "function beta(x: boolean) { if (x) { if (!x) { return 1; } return 2; } return 0; }\n",
    );
    let file = dir.join("sample.ts");

    let plain = call_tool(
        "analyze_function",
        serde_json::json!({ "path": file.to_str().unwrap(), "function": "beta" }),
    );
    assert!(
        result_of(&plain)["functions"][0]
            .get("contributions")
            .is_none()
    );

    let explained = call_tool(
        "analyze_function",
        serde_json::json!({
            "path": file.to_str().unwrap(),
            "function": "beta",
            "explain": true,
        }),
    );
    let contributions = result_of(&explained)["functions"][0]["contributions"]
        .as_array()
        .unwrap();
    assert_eq!(contributions.len(), 2);
    assert_eq!(contributions[0]["rule"], "if");
    assert_eq!(contributions[0]["nesting"], 0);
    assert_eq!(contributions[1]["nesting"], 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn analyze_changed_accepts_budget_params() {
    let root = common::temporary_directory();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(
        root.join("sample.ts"),
        "function simple(x: boolean) { return 1; }\nfunction branchy(x: boolean) { return 1; }\n",
    )
    .unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "base"]);
    std::fs::write(
        root.join("sample.ts"),
        "function simple(x: boolean) { if (x) { return 1; } return 0; }\nfunction branchy(x: boolean) { if (x) { if (!x) { return 2; } return 1; } return 0; }\n",
    )
    .unwrap();

    let filtered = call_tool(
        "analyze_changed",
        serde_json::json!({
            "base": "HEAD",
            "path": root.to_str().unwrap(),
            "min_delta": 2.0,
        }),
    );
    let result = result_of(&filtered);
    let changes = result["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["name"], "branchy");

    let topped = call_tool(
        "analyze_changed",
        serde_json::json!({
            "base": "HEAD",
            "path": root.to_str().unwrap(),
            "top": 1,
            "sort_by": "cognitive",
        }),
    );
    let result = result_of(&topped);
    let changes = result["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["name"], "branchy");
    assert_eq!(result["total"], 2);
    assert_eq!(result["truncated"], true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn repo_summary_returns_totals_and_top_metrics() {
    let dir = fixture_dir(
        "function alpha(x: boolean) { return 1; }\nfunction beta(x: boolean) { if (x) { if (!x) { return 1; } return 2; } return 0; }\n",
    );
    let path = dir.to_str().unwrap();

    let response = call_tool("repo_summary", serde_json::json!({ "path": path }));
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["totals"]["files"], 1);
    assert_eq!(result["totals"]["functions"], 2);
    assert_eq!(result["totals"]["parse_errors"], 0);
    assert_eq!(result["top"]["cognitive"][0]["name"], "beta");
    assert_eq!(result["top"]["cyclomatic"][0]["name"], "beta");
    assert_eq!(result["truncated"], false);

    let capped = call_tool(
        "repo_summary",
        serde_json::json!({ "path": path, "top": 1 }),
    );
    let result = result_of(&capped);
    assert_eq!(result["top"]["cognitive"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn initialize_carries_usage_instructions() {
    let response: Value = serde_json::from_str(
        &handle_request(&request("initialize", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    let instructions = result_of(&response)["instructions"]
        .as_str()
        .expect("initialize must include usage instructions");
    assert!(instructions.contains("analyze_changed"));
    assert!(instructions.contains("evidence"));
}

#[test]
fn tools_list_marks_execute_read_only() {
    let spec = execute_spec();
    let name = spec["name"].as_str().unwrap();
    assert!(
        spec["annotations"]["title"].is_string(),
        "{name} needs a title"
    );
    assert_eq!(
        spec["annotations"]["readOnlyHint"], true,
        "{name} readOnlyHint"
    );
    assert_eq!(
        spec["annotations"]["idempotentHint"], true,
        "{name} idempotentHint"
    );
    assert_eq!(
        spec["annotations"]["openWorldHint"], false,
        "{name} must advertise a closed world"
    );
    // The description must say when to reach for the surface, not just what it
    // is: a model picks the tool from this text alone.
    let description = spec["description"].as_str().unwrap();
    assert!(description.contains("Promise.all"), "{description}");
    assert!(description.contains("await"), "{description}");
}

fn sql_plan_fixture(cost_current: f64, cost_baseline: f64) -> PathBuf {
    const PLAN_TEMPLATE: &str =
        r#"[{"Plan": {"Node Type": "Seq Scan", "Total Cost": COST, "Plan Rows": 10}}]"#;
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    // MCP artifact arguments stay root-relative, so fixtures live under the
    // package root (the test working directory) instead of the temp dir.
    let root = PathBuf::from(format!(
        "target/leadline-mcp-sqlplan-{}-{id}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("current")).unwrap();
    std::fs::create_dir_all(root.join("baseline")).unwrap();
    std::fs::write(
        root.join("current/q.json"),
        PLAN_TEMPLATE.replace("COST", &cost_current.to_string()),
    )
    .unwrap();
    std::fs::write(
        root.join("baseline/q.json"),
        PLAN_TEMPLATE.replace("COST", &cost_baseline.to_string()),
    )
    .unwrap();
    root
}

#[test]
fn sql_plan_is_reachable_and_named_in_instructions() {
    let declarations = execute_declarations();
    for key in [
        "current",
        "baseline",
        "max_cost_increase_percent",
        "max_plan_rows_ratio",
        "max_estimate_error_ratio",
        "top",
    ] {
        assert!(declarations.contains(key), "sql_plan must take {key}");
    }
    let init: Value = serde_json::from_str(
        &handle_request(&request("initialize", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    assert!(
        result_of(&init)["instructions"]
            .as_str()
            .unwrap()
            .contains("sql_plan")
    );
}

#[test]
fn sql_plan_compares_directories_through_a_script() {
    let root = sql_plan_fixture(150.0, 100.0);
    let current = root.join("current").to_str().unwrap().to_owned();
    let baseline = root.join("baseline").to_str().unwrap().to_owned();
    let arguments = serde_json::json!({ "current": current, "baseline": baseline, "max_cost_increase_percent": 25.0 });
    let response = call_tool("sql_plan", arguments);
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["tool"], "sql_plan");
    assert_eq!(result["violations"][0]["kind"], "cost_increase");
    assert_eq!(result["truncated"], false);
    assert!(result.get("queries").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_plan_rejects_bad_arguments() {
    let root = sql_plan_fixture(10.0, 10.0);
    let current = root.join("current").to_str().unwrap().to_owned();
    let baseline = root.join("baseline").to_str().unwrap().to_owned();
    let good = serde_json::json!({ "current": current, "baseline": baseline });
    // unknown keys
    let mut bad = good.clone();
    bad["bogus"] = serde_json::json!(1);
    assert!(call_tool("sql_plan", bad).get("error").is_some());
    // absolute paths stay out of MCP
    let response = call_tool(
        "sql_plan",
        serde_json::json!({ "current": "/tmp/x", "baseline": "target/y" }),
    );
    assert!(error_of(&response)["code"].as_i64() == Some(-32602));
    // negative limits
    let mut bad = good.clone();
    bad["max_cost_increase_percent"] = serde_json::json!(-1.0);
    assert!(call_tool("sql_plan", bad).get("error").is_some());
    // top above the global cap
    let mut bad = good.clone();
    bad["top"] = serde_json::json!(201);
    assert!(call_tool("sql_plan", bad).get("error").is_some());
    // missing baseline
    let response = call_tool(
        "sql_plan",
        serde_json::json!({ "current": good["current"].clone() }),
    );
    assert!(response.get("error").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

fn security_findings_fixture() -> (Fixture, String, String) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    // MCP artifact arguments stay root-relative: the fixture lives under the
    // package root (the test working directory), cleaned up afterwards.
    // Note: not under target/, which discovery excludes via .gitignore.
    let root = Fixture::create(PathBuf::from(format!(
        "leadline-mcp-security-{}-{id}",
        std::process::id()
    )));
    std::fs::create_dir_all(root.join("proj/src")).unwrap();
    std::fs::write(
        root.join("proj/src/auth.ts"),
        "function login(y: number): number {\n  if (y > 0) { return 1; }\n  return 0;\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("findings.sarif"),
        r#"{"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "eslint"}}, "results": [{"ruleId": "js/hardcoded-secret", "level": "error", "message": {"text": "SENTINEL"}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/auth.ts"}, "region": {"startLine": 2}}}]}]}]}"#,
    )
    .unwrap();
    let proj = root.join("proj").to_str().unwrap().to_owned();
    let sarif = root.join("findings.sarif").to_str().unwrap().to_owned();
    (root, proj, sarif)
}

#[test]
fn security_findings_is_reachable_and_named_in_instructions() {
    let declarations = execute_declarations();
    for key in [
        "path",
        "sarif",
        "baseline_sarif",
        "base",
        "staged",
        "target",
        "minimum_severity",
        "new_only",
        "changed_only",
        "top",
    ] {
        assert!(
            declarations.contains(key),
            "security_findings must take {key}"
        );
    }
    let init: Value = serde_json::from_str(
        &handle_request(&request("initialize", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    assert!(
        result_of(&init)["instructions"]
            .as_str()
            .unwrap()
            .contains("security_findings")
    );
}

#[test]
fn security_findings_compares_through_a_script() {
    let (root, proj, sarif) = security_findings_fixture();
    let arguments =
        serde_json::json!({ "path": proj, "sarif": [sarif], "minimum_severity": "low" });
    let response = call_tool("security_findings", arguments);
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["tool"], "security_findings");
    assert_eq!(result["findings"][0]["rule_id"], "js/hardcoded-secret");
    assert!(result["findings"][0]["function_id"].is_string());
    assert_eq!(result["truncated"], false);
    assert!(!result["violations"].as_array().unwrap().is_empty());
    assert!(!serde_json::to_string(&result).unwrap().contains("SENTINEL"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_findings_rejects_bad_arguments() {
    let (root, proj, sarif) = security_findings_fixture();
    let good = serde_json::json!({ "path": proj, "sarif": [sarif] });
    // non-array sarif
    let mut bad = good.clone();
    bad["sarif"] = serde_json::json!("findings.sarif");
    assert!(call_tool("security_findings", bad).get("error").is_some());
    // unknown fields
    let mut bad = good.clone();
    bad["bogus"] = serde_json::json!(1);
    assert!(call_tool("security_findings", bad).get("error").is_some());
    // absolute report paths stay out of MCP
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "sarif": ["/tmp/x.sarif"] }),
    );
    assert!(error_of(&response)["code"].as_i64() == Some(-32602));
    // over-limit top
    let mut bad = good.clone();
    bad["top"] = serde_json::json!(201);
    assert!(call_tool("security_findings", bad).get("error").is_some());
    // missing sarif
    let response = call_tool("security_findings", serde_json::json!({ "path": "." }));
    assert!(response.get("error").is_some());
    // bad severity
    let mut bad = good.clone();
    bad["minimum_severity"] = serde_json::json!("bogus");
    assert!(call_tool("security_findings", bad).get("error").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_findings_rejects_changed_only_without_comparison() {
    let (root, proj, sarif) = security_findings_fixture();
    // Without base/staged/target every finding stays unchanged, so the gate
    // would silently pass; reject the combination instead.
    let response = call_tool(
        "security_findings",
        serde_json::json!({
            "path": proj,
            "sarif": [sarif],
            "minimum_severity": "low",
            "changed_only": true,
        }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_findings_rejects_empty_sarif() {
    let (root, proj, _) = security_findings_fixture();
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "path": proj, "sarif": [] }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    assert!(
        error_of(&response)["message"]
            .as_str()
            .unwrap()
            .contains("sarif"),
        "{response}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn vulnerabilities_fixture() -> (PathBuf, String, String) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    // Root-relative artifacts under the package root, cleaned up afterwards.
    // Note: not under target/, which discovery excludes via .gitignore.
    let root = PathBuf::from(format!("leadline-mcp-vuln-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("proj/src")).unwrap();
    std::fs::write(root.join("proj/src/app.ts"), "import _ from 'lodash';\n").unwrap();
    std::fs::write(root.join("osv.json"), r#"{"results": [{"source": {"path": "package-lock.json"}, "packages": [{"package": {"name": "lodash", "version": "4.17.20", "ecosystem": "npm"}, "vulnerabilities": [{"id": "GHSA-xxxx-yyyy-zzzz", "details": "SENTINEL", "database_specific": {"severity": "HIGH"}, "affected": [{"ranges": [{"type": "SEMVER", "events": [{"fixed": "4.17.21"}]}]}]}]}]}]}"#).unwrap();
    let proj = root.join("proj").to_str().unwrap().to_owned();
    let osv = root.join("osv.json").to_str().unwrap().to_owned();
    (root, proj, osv)
}

#[test]
fn vulnerabilities_is_reachable_and_named_in_instructions() {
    let declarations = execute_declarations();
    for key in [
        "path",
        "osv",
        "trivy",
        "baseline_osv",
        "baseline_trivy",
        "base",
        "staged",
        "target",
        "minimum_severity",
        "top",
    ] {
        assert!(
            declarations.contains(key),
            "vulnerabilities must take {key}"
        );
    }
    let init: Value = serde_json::from_str(
        &handle_request(&request("initialize", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    assert!(
        result_of(&init)["instructions"]
            .as_str()
            .unwrap()
            .contains("vulnerabilities")
    );
}

#[test]
fn vulnerabilities_reports_import_evidence_through_a_script() {
    let (root, proj, osv) = vulnerabilities_fixture();
    let arguments = serde_json::json!({ "path": proj, "osv": [osv], "minimum_severity": "low" });
    let response = call_tool("vulnerabilities", arguments);
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["tool"], "vulnerabilities");
    assert_eq!(result["findings"][0]["advisory_id"], "GHSA-xxxx-yyyy-zzzz");
    assert_eq!(result["reachability_model"], "changed-direct-imports");
    assert_eq!(result["truncated"], false);
    assert!(!result["violations"].as_array().unwrap().is_empty());
    assert!(!serde_json::to_string(&result).unwrap().contains("SENTINEL"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vulnerabilities_uses_configured_minimum_severity() {
    let (root, proj, osv) = vulnerabilities_fixture();
    // `[vulnerabilities] minimum_severity` is the CLI's default gate and must
    // apply over MCP too, without repeating it in every call.
    std::fs::write(
        PathBuf::from(&proj).join("leadline.toml"),
        "[vulnerabilities]\nminimum_severity = \"high\"\n",
    )
    .unwrap();
    let response = call_tool(
        "vulnerabilities",
        serde_json::json!({ "path": proj, "osv": [osv] }),
    );
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    let result = result_of(&response);
    assert!(
        !result["violations"].as_array().unwrap().is_empty(),
        "{result}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vulnerabilities_rejects_bad_arguments() {
    let (root, proj, osv) = vulnerabilities_fixture();
    let good = serde_json::json!({ "path": proj, "osv": [osv] });
    let mut bad = good.clone();
    bad["osv"] = serde_json::json!("osv.json");
    assert!(call_tool("vulnerabilities", bad).get("error").is_some());
    let mut bad = good.clone();
    bad["bogus"] = serde_json::json!(1);
    assert!(call_tool("vulnerabilities", bad).get("error").is_some());
    let response = call_tool(
        "vulnerabilities",
        serde_json::json!({ "osv": ["/tmp/x.json"] }),
    );
    assert!(error_of(&response)["code"].as_i64() == Some(-32602));
    let many: Vec<serde_json::Value> = (0..33)
        .map(|i| serde_json::json!(format!("f{i}.json")))
        .collect();
    assert!(
        call_tool("vulnerabilities", serde_json::json!({ "osv": many }))
            .get("error")
            .is_some()
    );
    let response = call_tool("vulnerabilities", serde_json::json!({ "path": "." }));
    assert!(response.get("error").is_some());
    let mut bad = good.clone();
    bad["minimum_severity"] = serde_json::json!("bogus");
    assert!(call_tool("vulnerabilities", bad).get("error").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

fn sql_risks_fixture() -> (Fixture, String) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    // Root-relative artifacts under the package root, cleaned up afterwards.
    // Note: not under target/, which discovery excludes via .gitignore.
    let root = Fixture::create(PathBuf::from(format!(
        "leadline-mcp-sql-{}-{id}",
        std::process::id()
    )));
    std::fs::write(root.join("risky.sql"), "UPDATE users SET active = false;\n").unwrap();
    let rel = root.to_str().unwrap().to_owned();
    (root, rel)
}

#[test]
fn sql_risks_is_reachable_and_named_in_instructions() {
    let declarations = execute_declarations();
    for key in [
        "path",
        "large_offset",
        "migration_roots",
        "minimum_severity",
        "top",
    ] {
        assert!(declarations.contains(key), "sql_risks must take {key}");
    }
    let init: Value = serde_json::from_str(
        &handle_request(&request("initialize", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    assert!(
        result_of(&init)["instructions"]
            .as_str()
            .unwrap()
            .contains("sql_risks")
    );
}

#[test]
fn sql_risks_reports_static_findings() {
    let (root, rel) = sql_risks_fixture();
    let arguments = serde_json::json!({ "path": rel, "minimum_severity": "low" });
    let response = call_tool("sql_risks", arguments);
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    let result = result_of(&response);
    envelope_ok(result);
    assert_eq!(result["tool"], "sql_risks");
    assert_eq!(result["dialect"], "postgresql");
    assert_eq!(
        result["findings"][0]["rule_id"],
        "sql/update-delete-without-where"
    );
    assert_eq!(result["truncated"], false);
    assert!(!result["violations"].as_array().unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_risks_honors_config_excludes_and_defaults() {
    let (root, rel) = sql_risks_fixture();
    // `[analysis].exclude` must skip files on MCP exactly like the CLI, and
    // `[sql]` supplies large_offset/migration_roots without arguments.
    std::fs::write(
        root.join("leadline.toml"),
        "[analysis]\nexclude = [\"risky.sql\"]\n\n[sql]\nlarge_offset = 2\n",
    )
    .unwrap();
    let response = call_tool("sql_risks", serde_json::json!({ "path": rel }));
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    let result = result_of(&response);
    assert!(
        result["findings"].as_array().unwrap().is_empty(),
        "{result}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_risks_accepts_relative_migration_roots() {
    let (root, rel) = sql_risks_fixture();
    std::fs::create_dir_all(root.join("migrations")).unwrap();
    std::fs::write(
        root.join("migrations/001.sql"),
        "CREATE TABLE users (id int);\n",
    )
    .unwrap();
    // Migration roots are analysis-root-relative, like the CLI flag.
    let response = call_tool(
        "sql_risks",
        serde_json::json!({
            "path": rel,
            "migration_roots": ["migrations"],
            "minimum_severity": "low",
        }),
    );
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    let result = result_of(&response);
    assert_eq!(result["schema_evidence_available"], true);
    assert!(
        !result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["rule_id"] == "sql/unknown-table"),
        "{result}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sql_risks_rejects_bad_arguments() {
    let (root, rel) = sql_risks_fixture();
    let good = serde_json::json!({ "path": rel });
    let mut bad = good.clone();
    bad["bogus"] = serde_json::json!(1);
    assert!(call_tool("sql_risks", bad).get("error").is_some());
    let mut bad = good.clone();
    bad["migration_roots"] = serde_json::json!(["/abs"]);
    assert!(call_tool("sql_risks", bad).get("error").is_some());
    let mut bad = good.clone();
    bad["top"] = serde_json::json!(201);
    assert!(call_tool("sql_risks", bad).get("error").is_some());
    let mut bad = good.clone();
    bad["large_offset"] = serde_json::json!(0);
    assert!(call_tool("sql_risks", bad).get("error").is_some());
    let mut bad = good.clone();
    bad["minimum_severity"] = serde_json::json!("bogus");
    assert!(call_tool("sql_risks", bad).get("error").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

fn mcp_args(args: &[&str]) -> Result<Option<leadline::mcp::HttpOptions>, String> {
    leadline::mcp::parse_mcp_args(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
}

#[test]
fn mcp_args_default_to_stdio() {
    assert_eq!(mcp_args(&[]), Ok(None));
}

#[test]
fn mcp_args_parse_port_and_host() {
    assert_eq!(
        mcp_args(&["--port"]),
        Ok(Some(leadline::mcp::HttpOptions {
            host: "127.0.0.1".to_owned(),
            port: leadline::http::DEFAULT_PORT,
        }))
    );
    assert_eq!(mcp_args(&["--port", "0"]).unwrap().unwrap().port, 0);
    let options = mcp_args(&["--port", "8080", "--host", "0.0.0.0"])
        .unwrap()
        .unwrap();
    assert_eq!(options.port, 8080);
    assert_eq!(options.host, "0.0.0.0");
    assert!(mcp_args(&["--host"]).is_err());
    // A host configures only the HTTP transport, so it needs a port.
    assert!(mcp_args(&["--host", "0.0.0.0"]).is_err());
    assert!(mcp_args(&["--port", "abc"]).is_err());
    assert!(mcp_args(&["--bogus"]).is_err());
}

#[test]
fn http_bind_falls_back_to_free_port_when_taken() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = guard.local_addr().unwrap().port();
    let (_listener, actual, fell_back) = leadline::http::bind("127.0.0.1", taken).unwrap();
    assert!(fell_back);
    assert_ne!(actual, taken);
}

/// A spawned HTTP server process, killed when the guard drops.
///
/// The server runs as its own process with the ambient local-metrics store
/// removed, so a test run cannot write into the developer's store.
struct HttpServer {
    child: std::process::Child,
    port: u16,
}

impl HttpServer {
    fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the real server on an OS-assigned port and reads its banner.
fn spawn_http_server() -> HttpServer {
    use std::io::BufRead as _;
    let mut child = common::leadline()
        .args(["mcp", "--port", "0"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut banner = String::new();
    reader.read_line(&mut banner).unwrap();
    let port = banner
        .rsplit(':')
        .next()
        .and_then(|tail| tail.split('/').next())
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or_else(|| panic!("no listening port in {banner:?}"));
    // Keep draining stderr so a chatty server never blocks on a full pipe.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    });
    HttpServer { child, port }
}

fn http_exchange(port: u16, head: &str, body: &str) -> String {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn http_post_serves_tools_list() {
    let server = spawn_http_server();
    let port = server.port();
    let body = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})
        .to_string();
    let response = http_exchange(port, "POST /mcp HTTP/1.1\r\nHost: localhost", &body);
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("analyze_changed"));
}

#[test]
fn http_health_reports_status() {
    let server = spawn_http_server();
    let port = server.port();
    let response = http_exchange(port, "GET /health HTTP/1.1\r\nHost: localhost", "");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("\"status\":\"ok\""));
}

#[test]
fn http_rejects_wrong_method_and_path() {
    let server = spawn_http_server();
    let port = server.port();
    let response = http_exchange(port, "GET /mcp HTTP/1.1\r\nHost: localhost", "");
    assert!(response.starts_with("HTTP/1.1 405"), "{response}");
    let response = http_exchange(port, "GET /nope HTTP/1.1\r\nHost: localhost", "");
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");
}

#[test]
fn http_validates_origin_for_browser_requests() {
    let server = spawn_http_server();
    let port = server.port();
    let body = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})
        .to_string();
    // Non-loopback, scheme-less, and non-http(s) origins are rejected
    // (DNS-rebinding defense).
    for origin in [
        "http://evil.example",
        "https://evil.example:8443",
        "null",
        "localhost",
        "evil://localhost",
    ] {
        let head = format!("POST /mcp HTTP/1.1\r\nHost: localhost\r\nOrigin: {origin}");
        let response = http_exchange(port, &head, &body);
        assert!(response.starts_with("HTTP/1.1 403"), "{origin}: {response}");
    }
    // Loopback origins and Origin-less clients (stdio bridges, curl) pass.
    for head in [
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost:5173",
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nOrigin: http://127.0.0.1",
        "POST /mcp HTTP/1.1\r\nHost: localhost",
    ] {
        let response = http_exchange(port, head, &body);
        assert!(response.starts_with("HTTP/1.1 200"), "{head}: {response}");
    }
}

#[test]
fn http_rejects_oversized_header_lines() {
    let server = spawn_http_server();
    let port = server.port();
    let long = "x".repeat(9000);
    // Send the whole request, oversized line included, then wait before
    // reading: the early rejection must survive a close with request bytes
    // still queued, not be reset away.
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET /health HTTP/1.1\r\nHost: localhost\r\nX-Long: {long}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("the rejection must survive a delayed read");
    assert!(response.starts_with("HTTP/1.1 431"), "{response}");
}

/// Send one raw request without adding framing headers.
fn http_raw(port: u16, request: &str) -> String {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn http_requires_exactly_one_valid_host_and_content_length() {
    let server = spawn_http_server();
    let port = server.port();
    let body =
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}}).to_string();
    let request = |head: &str| {
        format!(
            "{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    // HTTP/1.1 requires a Host, and a loopback listener only trusts loopback.
    let missing = http_raw(port, &request("POST /mcp HTTP/1.1"));
    assert!(missing.starts_with("HTTP/1.1 400"), "{missing}");
    let rebound = http_raw(port, &request("POST /mcp HTTP/1.1\r\nHost: evil.example"));
    assert!(rebound.starts_with("HTTP/1.1 400"), "{rebound}");
    let duplicate = http_raw(
        port,
        &request("POST /mcp HTTP/1.1\r\nHost: localhost\r\nHost: evil.example"),
    );
    assert!(duplicate.starts_with("HTTP/1.1 400"), "{duplicate}");
    // Duplicate framing headers must not let a proxy and the server disagree.
    let double_length = http_raw(
        port,
        &format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    );
    assert!(double_length.starts_with("HTTP/1.1 400"), "{double_length}");
    // An empty port is a malformed authority, not a loopback host.
    let empty_port = http_raw(port, &request("POST /mcp HTTP/1.1\r\nHost: localhost:"));
    assert!(empty_port.starts_with("HTTP/1.1 400"), "{empty_port}");
    // Loopback authorities with ports keep working, including split bodies.
    let ok = http_raw(port, &request("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:1"));
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    let (first, second) = body.split_at(body.len() / 2);
    write!(stream, "{first}").unwrap();
    stream.flush().unwrap();
    write!(stream, "{second}").unwrap();
    let mut split_response = String::new();
    stream.read_to_string(&mut split_response).unwrap();
    assert!(
        split_response.starts_with("HTTP/1.1 200"),
        "{split_response}"
    );
}

#[test]
fn http_rejects_malformed_framing_and_empty_bodies() {
    let server = spawn_http_server();
    let port = server.port();
    let body =
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}}).to_string();
    // Only HTTP/1.1 is spoken.
    let response = http_raw(
        port,
        "GET /health HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 505"), "{response}");
    // Exactly three request-line tokens.
    let response = http_raw(
        port,
        "GET /health HTTP/1.1 extra\r\nHost: localhost\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    // No whitespace between a header name and its colon.
    let response = http_raw(port, "GET /health HTTP/1.1\r\nHost : localhost\r\n\r\n");
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    // Header lines without a colon are malformed.
    let response = http_raw(
        port,
        "GET /health HTTP/1.1\r\nHost: localhost\r\nbroken\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    // Transfer codings are not implemented, so TE plus Content-Length cannot
    // be interpreted differently from a front proxy.
    let response = http_raw(
        port,
        &format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    );
    assert!(response.starts_with("HTTP/1.1 501"), "{response}");
    // An empty POST is not a JSON-RPC notification.
    let response = http_raw(
        port,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    // Framing numbers are digits only.
    let response = http_raw(
        port,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: +10\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    // A half-closed connection without the terminating blank line is
    // truncated, not an empty header line.
    use std::io::{Read as _, Write as _};
    use std::net::Shutdown;
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET /health HTTP/1.1\r\nHost: localhost\r\n").unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut truncated = String::new();
    stream.read_to_string(&mut truncated).unwrap();
    assert!(truncated.starts_with("HTTP/1.1 400"), "{truncated}");
}

#[test]
fn jsonrpc_rejects_invalid_requests_and_oversized_batches() {
    // Empty batch, missing method, and structured ids are invalid requests.
    for raw in [
        "[]",
        "{}",
        r#"[{"jsonrpc": "2.0"}]"#,
        r#"{"jsonrpc": "2.0", "id": true, "method": "ping"}"#,
        r#"{"jsonrpc": "2.0", "id": [1], "method": "ping"}"#,
    ] {
        let mut response: Value = serde_json::from_str(&handle_request(raw).unwrap()).unwrap();
        if let Value::Array(items) = response {
            response = items.into_iter().next().unwrap();
        }
        assert_eq!(response["error"]["code"], -32600, "{raw}");
    }
    // A batch of notifications alone stays silent.
    let silent = format!(
        "[{}]",
        r#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#
    );
    assert!(handle_request(&silent).is_none());
    // Tool notifications are dispatched for their side effects but never
    // answered.
    assert!(
        handle_request(
            r#"{"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "explain_metric", "arguments": {"metric": "crap"}}}"#
        )
        .is_none()
    );
    // Params, when present, must be structured.
    let mut response: Value = serde_json::from_str(
        &handle_request(r#"{"jsonrpc": "2.0", "id": 1, "method": "ping", "params": "x"}"#).unwrap(),
    )
    .unwrap();
    if let Value::Array(items) = response {
        response = items.into_iter().next().unwrap();
    }
    assert_eq!(response["error"]["code"], -32602);
    // Batches are capped before they can amplify into a huge response; the
    // over-limit reply keeps the array shape so batch clients can parse it.
    let batch: Vec<Value> = (0..65)
        .map(|_| serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
        .collect();
    let mut response: Value =
        serde_json::from_str(&handle_request(&serde_json::to_string(&batch).unwrap()).unwrap())
            .unwrap();
    let Value::Array(items) = &response else {
        panic!("over-limit batch must answer with an array: {response}");
    };
    response = items.first().unwrap().clone();
    assert_eq!(response["error"]["code"], -32600);
    // An oversized batch of notifications still draws no response.
    let notifications: Vec<Value> = (0..65)
        .map(|_| serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .collect();
    assert!(handle_request(&serde_json::to_string(&notifications).unwrap()).is_none());
}

#[test]
fn mis_typed_string_arguments_are_rejected() {
    // A non-string gate argument must not silently disable the gate.
    let response = call_tool(
        "security_findings",
        serde_json::json!({
            "sarif": ["tests/fixtures/security/basic.sarif"],
            "minimum_severity": 123,
        }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    // Same for the analysis path: a number is not a path.
    let response = call_tool("analyze", serde_json::json!({ "path": 1 }));
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
}

#[test]
fn mis_typed_boolean_arguments_are_rejected() {
    // A non-boolean flag must not silently read as false.
    let dir = fixture_dir("function calc(x: boolean) { if (x) return 1; return 0; }\n");
    let path = dir.to_str().unwrap();
    let file = dir.join("sample.ts");
    for (tool, arguments) in [
        (
            "analyze_changed",
            serde_json::json!({ "path": path, "explain": "yes" }),
        ),
        (
            "analyze_changed",
            serde_json::json!({ "path": path, "renames": 1 }),
        ),
        (
            "analyze_function",
            serde_json::json!({ "path": file.to_str().unwrap(), "function": "calc", "explain": "yes" }),
        ),
    ] {
        let response = call_tool(tool, arguments);
        assert_eq!(
            error_of(&response)["code"].as_i64(),
            Some(-32602),
            "{tool}: expected an invalid-params rejection, got {response}"
        );
        let message = error_of(&response)["message"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(
            message.contains("must be a boolean"),
            "{tool}: expected a boolean rejection, got {response}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn list_tools_reject_over_limit_top() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) return 1; return 0; }\n");
    let response = call_tool(
        "analyze",
        serde_json::json!({ "path": dir.to_str().unwrap(), "top": 201 }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    let response = call_tool(
        "repo_summary",
        serde_json::json!({ "path": dir.to_str().unwrap(), "top": 51 }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    // The cap is inclusive: top=50 stays valid.
    let response = call_tool(
        "repo_summary",
        serde_json::json!({ "path": dir.to_str().unwrap(), "top": 50 }),
    );
    assert!(response.get("error").is_none(), "{response}");
    let response = call_tool(
        "test_targets",
        serde_json::json!({
            "path": dir.to_str().unwrap(),
            "coverage": dir.join("missing.info").to_str().unwrap(),
            "top": 201,
        }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn artifact_paths_accept_native_separators_and_reject_escapes() {
    let (root, proj, sarif) = security_findings_fixture();
    // Parent traversal out of the working directory.
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "path": proj, "sarif": ["../outside.sarif"] }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    // Interior `..` that stays inside the working directory is normalized.
    let interior = format!("leadline-mcp-missing/../{sarif}");
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "path": proj, "sarif": [interior], "minimum_severity": "low" }),
    );
    assert!(response.get("error").is_none(), "{response}");
    // JSON clients on Windows naturally send root-relative paths with `\`.
    let native = sarif.replace('/', "\\");
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "path": proj, "sarif": [native], "minimum_severity": "low" }),
    );
    assert!(response.get("error").is_none(), "{response}");
    // Normalizing separators must not turn drive-qualified input into a
    // root-relative path.
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "path": proj, "sarif": ["C:\\outside.sarif"] }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "path": proj, "sarif": [".\\C:outside.sarif"] }),
    );
    assert!(
        error_of(&response)["message"]
            .as_str()
            .unwrap()
            .contains("root-relative"),
        "{response}"
    );
    // sql_plan directories are validated the same way.
    let response = call_tool(
        "sql_plan",
        serde_json::json!({ "current": "../plans", "baseline": "target/plans" }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    let response = call_tool(
        "sql_plan",
        serde_json::json!({ "current": "target/plans", "baseline": "../../plans" }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn artifact_paths_reject_symlinks_that_leave_the_working_directory() {
    let (root, proj, _sarif) = security_findings_fixture();
    let link = root.join("escape.sarif");
    std::os::unix::fs::symlink("/etc/hosts", &link).unwrap();
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "path": proj, "sarif": [link.to_str().unwrap()] }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    // A missing artifact must not hide behind a symlinked parent directory.
    let directory = root.join("escape_dir");
    std::os::unix::fs::symlink("/etc", &directory).unwrap();
    let missing = directory.join("missing.sarif");
    let response = call_tool(
        "security_findings",
        serde_json::json!({ "path": proj, "sarif": [missing.to_str().unwrap()] }),
    );
    assert_eq!(error_of(&response)["code"].as_i64(), Some(-32602));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_uses_config_thresholds_and_regressions() {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("leadline-mcp-config-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("leadline.toml"),
        "[thresholds.function]\ncyclomatic = 1\n\n[regressions]\ncognitive = 2\ncyclomatic = 1\nmax_nesting = 1\ncrap = 1.0\n",
    )
    .unwrap();
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) return x; return 0; }\n",
    )
    .unwrap();
    // The request names only cognitive; cyclomatic must come from the config.
    let response = call_tool(
        "check",
        serde_json::json!({
            "path": root.to_str().unwrap(),
            "thresholds": { "cognitive": 100 },
        }),
    );
    let result = result_of(&response);
    assert_eq!(result["passed"], false, "{result}");
    assert_eq!(result["thresholds"]["cyclomatic"], 1);

    // A configured +1 cyclomatic regression limit must reach the gate.
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["add", "."]);
    run(&["commit", "-m", "base", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 0) { if (x > 1) return 2; } return 0; }\n",
    )
    .unwrap();
    let response = call_tool(
        "check",
        serde_json::json!({
            "path": root.to_str().unwrap(),
            "base": "HEAD",
            "regressions": true,
            "thresholds": { "cyclomatic": 100 },
        }),
    );
    let result = result_of(&response);
    assert_eq!(result["regressions"]["cognitive"], 2);
    assert_eq!(result["regressions"]["cyclomatic"], 1);
    // Deltas equal to the configured limits pass; zero-tolerance would fail.
    assert_eq!(result["passed"], true, "{result}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn security_findings_caps_violations_at_top() {
    let (root, proj, sarif) = security_findings_fixture();
    // Three gated findings in one report; top=1 must bound the response.
    let report = std::fs::read_to_string(&sarif).unwrap();
    let mut value: Value = serde_json::from_str(&report).unwrap();
    let mut results = Vec::new();
    for index in 0..3 {
        let mut result = value["runs"][0]["results"][0].clone();
        result["ruleId"] = serde_json::json!(format!("js/hardcoded-secret-{index}"));
        results.push(result);
    }
    value["runs"][0]["results"] = Value::Array(results);
    std::fs::write(&sarif, value.to_string()).unwrap();
    let response = call_tool(
        "security_findings",
        serde_json::json!({
            "path": proj,
            "sarif": [sarif],
            "minimum_severity": "low",
            "top": 1,
        }),
    );
    let result = result_of(&response);
    assert_eq!(result["violations"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analyze_tool_reuses_an_index_without_writing_one() {
    let dir = fixture_dir("function alpha(a: number) { return a + 1; }\n");
    let path = dir.to_str().unwrap();
    // Build the index outside MCP so this test only exercises reads.
    let built = common::leadline().arg("index").arg(&dir).output().unwrap();
    assert!(built.status.success());

    let index_file = dir.join(".leadline").join("index.json");
    let index_arg = dir.join(".leadline");
    let before = std::fs::read(&index_file).unwrap();

    let response = call_tool(
        "analyze",
        serde_json::json!({ "path": path, "index": index_arg.to_str().unwrap() }),
    );
    let result = result_of(&response);
    assert_eq!(result["index"]["analyzed"], 0);
    assert!(result["index"]["reused"].as_u64().unwrap() > 0);

    let after = std::fs::read(&index_file).unwrap();
    assert_eq!(before, after, "MCP must never write the index");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn analyze_tool_reuses_a_repository_index_for_a_file_target() {
    let dir = fixture_dir("function alpha(a: number) { return a + 1; }\n");
    let file = dir.join("sample.ts");
    let index = dir.join(".leadline");
    // Documented workflow: build the repository index once with
    // `leadline index`, then reuse it for a single-file analysis.
    let built = common::leadline().arg("index").arg(&dir).output().unwrap();
    assert!(built.status.success());

    let response = call_tool(
        "analyze",
        serde_json::json!({ "path": file.to_str().unwrap(), "index": index.to_str().unwrap() }),
    );
    let result = result_of(&response);
    assert_eq!(result["index"]["analyzed"], 0);
    assert!(result["index"]["reused"].as_u64().unwrap() > 0);

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn non_string_index_argument_is_rejected() {
    let response = call_tool("analyze", serde_json::json!({ "path": ".", "index": 7 }));
    assert_eq!(error_of(&response)["code"], -32602);
}

#[test]
fn risk_reports_components() {
    let dir = fixture_dir("function calc(x: boolean) { if (x) { return 1; } return 0; }\n");
    let response = call_tool("risk", serde_json::json!({ "path": dir.to_str().unwrap() }));
    let result = result_of(&response);
    envelope_ok(result);
    assert!(
        !result["risks"].as_array().unwrap().is_empty(),
        "{response}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn debt_and_project_compare_a_git_repo() {
    let root = git_fixture();
    std::fs::write(
        root.join("sample.ts"),
        "function calc(x: boolean) { return 1; }\n",
    )
    .unwrap();
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "base"]);
    // Leave a more complex uncommitted state for the comparison.
    std::fs::write(
        root.join("sample.ts"),
        "function calc(x: boolean) { if (x) { return 1; } return 0; }\n",
    )
    .unwrap();

    let debt = call_tool(
        "debt",
        serde_json::json!({ "path": root.to_str().unwrap(), "base": "HEAD" }),
    );
    let result = result_of(&debt);
    envelope_ok(result);
    assert_eq!(result["tool"], "debt");
    assert!(result["summary"].is_object(), "{debt}");

    let project = call_tool(
        "project",
        serde_json::json!({ "path": root.to_str().unwrap() }),
    );
    let result = result_of(&project);
    envelope_ok(result);
    assert_eq!(result["tool"], "project");
    assert!(
        result["summary"]["files"].as_u64().unwrap() >= 1,
        "{project}"
    );

    let missing = call_tool(
        "project",
        serde_json::json!({ "path": root.to_str().unwrap(), "pit": ["missing-pit.json"] }),
    );
    assert_eq!(error_of(&missing)["code"], -32602);
}

// ---------------------------------------------------------------------------
// Telemetry over the HTTP transport
// ---------------------------------------------------------------------------

/// Metrics directory for one test, following this file's fixture naming.
fn metrics_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("leadline-mcp-metrics-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Posts one JSON-RPC body to the HTTP transport and returns the raw response.
fn post_json(port: u16, body: &str) -> String {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read");
    response
}

#[test]
fn mcp_records_sessions_methods_and_payloads() {
    let metrics = metrics_directory();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);

    let mut child = common::leadline()
        .args(["mcp", "--port", &port.to_string()])
        .env("LEADLINE_METRICS_DIR", &metrics)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("server starts");

    let mut ready = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(ready, "server never accepted a connection");

    let listed = post_json(
        port,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
    );
    assert!(listed.contains("tools"), "{listed}");
    let fixture = fixture_dir("export function alpha(x: number) { return x > 1 ? x : 1; }\n");
    let called = post_json(
        port,
        &request(
            "tools/call",
            serde_json::json!({
                "name": "execute",
                "arguments": {
                    "code": format!(
                        "const s = await tools.repo_summary({{ path: {:?} }}); return s.totals;",
                        fixture.to_str().unwrap()
                    )
                }
            }),
        ),
    );
    assert!(called.contains("functions"), "{called}");
    let _ = post_json(
        port,
        r#"{"jsonrpc":"2.0","id":2,"method":"nope","params":{}}"#,
    );
    let _ = post_json(
        port,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#,
    );
    let _ = post_json(port, "not json");

    child.kill().ok();
    child.wait().ok();

    let text = std::fs::read_to_string(metrics.join("leadline.prom")).expect("metrics rendered");
    assert!(
        text.contains(r#"leadline_mcp_requests_total{method="tools_list"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"leadline_mcp_requests_total{method="unknown"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"leadline_mcp_requests_total{method="tools_call"} 2"#),
        "{text}"
    );
    assert!(
        text.contains(r#"leadline_mcp_errors_total{reason="method_not_found",transport="http"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"leadline_mcp_errors_total{reason="parse_error",transport="http"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"leadline_mcp_errors_total{reason="tool_error",transport="http"} 1"#),
        "{text}"
    );
    assert!(text.contains("leadline_mcp_request_bytes_count"), "{text}");
    assert!(text.contains("leadline_mcp_response_bytes_count"), "{text}");
    assert!(
        text.contains(r#"leadline_mcp_inflight_calls{transport="http"} 0"#),
        "{text}"
    );
}
