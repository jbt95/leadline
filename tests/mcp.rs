use leadline::mcp::handle_request;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn request(method: &str, params: Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string()
}

fn call_tool(name: &str, arguments: Value) -> Value {
    let raw = request(
        "tools/call",
        serde_json::json!({ "name": name, "arguments": arguments }),
    );
    let response = handle_request(&raw).expect("tools/call must respond");
    serde_json::from_str(&response).unwrap()
}

fn result_of(response: &Value) -> &Value {
    response
        .get("result")
        .unwrap_or_else(|| panic!("expected result, got {response}"))
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

fn envelope_ok(result: &Value) {
    assert_eq!(result["schema_version"], 1);
    assert!(result["analyzer_version"].is_string());
    for metric in [
        "cyclomatic",
        "cognitive",
        "halstead",
        "maintainability",
        "crap",
    ] {
        assert_eq!(result["metric_specs"][metric], "default-v1");
    }
}

#[test]
fn initialize_and_tools_list() {
    let response: Value = serde_json::from_str(
        &handle_request(&request("initialize", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    assert_eq!(result_of(&response)["serverInfo"]["name"], "leadline");

    let response: Value = serde_json::from_str(
        &handle_request(&request("tools/list", serde_json::json!({}))).unwrap(),
    )
    .unwrap();
    let tools = result_of(&response)["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "analyze",
            "analyze_changed",
            "analyze_function",
            "check",
            "explain_metric"
        ]
    );
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
fn analyze_accepts_direct_method_and_coverage() {
    let dir = fixture_dir("function covered(x: boolean) { if (x) return 1; return 0; }\n");
    let file = dir.join("sample.ts");
    let lcov = dir.join("lcov.info");
    std::fs::write(&lcov, "SF:sample.ts\nDA:1,1\nDA:2,0\nend_of_record\n").unwrap();
    let raw = request(
        "analyze",
        serde_json::json!({ "path": dir.to_str().unwrap(), "coverage": lcov.to_str().unwrap() }),
    );
    let response: Value = serde_json::from_str(&handle_request(&raw).unwrap()).unwrap();
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

    let passing = call_tool(
        "check",
        serde_json::json!({ "path": path, "thresholds": { "cyclomatic": 10 } }),
    );
    assert_eq!(result_of(&passing)["passed"], true);

    let no_thresholds = call_tool("check", serde_json::json!({ "path": path }));
    assert_eq!(error_of(&no_thresholds)["code"], -32602);
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
        assert_eq!(result["spec"], "default-v1");
        assert!(
            result["definition"]
                .as_str()
                .unwrap()
                .contains("default-v1")
        );
    }
    let unknown = call_tool("explain_metric", serde_json::json!({ "metric": "vibes" }));
    assert_eq!(error_of(&unknown)["code"], -32602);
}

#[test]
fn analyze_changed_reports_deltas() {
    let root = std::env::temp_dir().join(format!(
        "leadline-mcp-changed-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
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
    assert_eq!(result["truncated"], false);
    std::fs::remove_dir_all(root).unwrap();
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
