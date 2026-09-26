//! Embedded JavaScript sandbox behind the MCP `execute` tool.
//!
//! A script runs in a QuickJS context with no filesystem, network, module, or
//! timer access. The only capability it holds is `tools`, one function per
//! analyzer tool. The bridge between JavaScript and Rust is a single
//! JSON-in/JSON-out native function, so no `rquickjs` value conversion leaks
//! into the tool layer and every tool keeps its own JSON contract.
//!
//! Scripts are async function bodies, compiled as one by the runner. The
//! returned promise is settled with [`Promise::finish`], which drives the
//! QuickJS job queue, so `await` and `Promise.all` settle without an async
//! runtime and without a `futures` dependency.

use rquickjs::{Array, Context, Ctx, Exception, Function, Promise, Runtime};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Memory ceiling for one script.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// Wall-clock limit for one script.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Tool calls one script may make.
const MAX_TOOL_CALLS: usize = 100;

/// Reported when a script outruns [`TIMEOUT`].
const TIMEOUT_MESSAGE: &str = "script exceeded the 30s time limit";

/// A tool handler: JSON arguments in, JSON result or `(code, message)` error out.
pub type ToolHandler = Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value, (i64, String)>>;

/// The callable tool surface, keyed by tool name.
pub type ToolTable = BTreeMap<&'static str, ToolHandler>;

/// Installs `tools` and the result serializer on a fresh context.
const BOOTSTRAP: &str = r"
(() => {
  const dispatch = (name, args) => {
    const payload = JSON.parse(__leadline(name, JSON.stringify(args === undefined ? {} : args)));
    if (!payload.ok) throw new Error(payload.error);
    return payload.value;
  };
  const tools = {};
  for (const name of __tool_names) tools[name] = (args) => dispatch(name, args);
  globalThis.tools = tools;
  // The body is an async function body, so it is compiled as one. Failures
  // are returned as data instead of thrown, which keeps the JavaScript
  // message intact instead of collapsing it into a QuickJS exception.
  globalThis.__leadline_run = async () => {
    try {
      const value = await (new Function('return (async () => {' + __leadline_body + '\n})()'))();
      return JSON.stringify({ ok: true, value: value === undefined ? null : value });
    } catch (e) {
      return JSON.stringify({ ok: false, error: String((e && e.message) || e) });
    }
  };
})();
";

/// One script execution, with its own QuickJS runtime and context.
pub struct Sandbox {
    runtime: Runtime,
    context: Context,
}

impl Sandbox {
    /// Create a sandbox with the memory ceiling applied.
    pub fn new() -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        Ok(Self { runtime, context })
    }

    /// Run `code` as an async function body and return its JSON result.
    ///
    /// The script sees only `tools`. A script that outruns [`TIMEOUT`] is
    /// reported as a timeout even when QuickJS also raised an interrupt
    /// exception, because the deadline is the authority.
    pub fn run(&self, code: &str, tools: &ToolTable) -> Result<serde_json::Value, String> {
        let deadline = Instant::now() + TIMEOUT;
        self.runtime
            .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
        let outcome = self.context.with(|ctx| run_script(&ctx, code, tools));
        self.runtime.set_interrupt_handler(None);
        if Instant::now() >= deadline {
            return Err(TIMEOUT_MESSAGE.to_string());
        }
        outcome
    }
}

/// Register the bridge, evaluate the script, and decode its result.
fn run_script(ctx: &Ctx, code: &str, tools: &ToolTable) -> Result<serde_json::Value, String> {
    // The closure owns its captures so it satisfies `IntoJsFunc`'s `'js`
    // bound; the table is twenty `Arc` entries, so the clone is trivial.
    let owned = tools.clone();
    let calls = AtomicUsize::new(0);
    let bridge = Function::new(ctx.clone(), move |tool: String, args: String| {
        invoke(&owned, &calls, &tool, &args)
    })
    .map_err(|e| e.to_string())?;
    ctx.globals()
        .set("__leadline", bridge)
        .map_err(|e| e.to_string())?;

    let names = Array::new(ctx.clone()).map_err(|e| e.to_string())?;
    for (index, name) in tools.keys().enumerate() {
        names.set(index, *name).map_err(|e| e.to_string())?;
    }
    ctx.globals()
        .set("__tool_names", names)
        .map_err(|e| e.to_string())?;

    ctx.eval::<(), _>(BOOTSTRAP).map_err(|e| describe(ctx, e))?;

    // The body is set as a global, never concatenated into Rust-side source,
    // so a body containing quotes or newlines needs no escaping.
    ctx.globals()
        .set("__leadline_body", code)
        .map_err(|e| describe(ctx, e))?;
    // Plain `eval` returns the runner's promise itself; `eval_promise` would
    // wrap it a second time. `finish` pumps the job queue, so awaited tool
    // calls and `Promise.all` settle before it returns.
    let promise: Promise = ctx.eval("__leadline_run()").map_err(|e| describe(ctx, e))?;
    let report: String = promise.finish().map_err(|e| describe(ctx, e))?;
    let report: serde_json::Value = serde_json::from_str(&report)
        .map_err(|e| format!("script returned a non-JSON result: {e}"))?;
    match report.get("ok") {
        Some(serde_json::Value::Bool(true)) => Ok(report
            .get("value")
            .cloned()
            .unwrap_or(serde_json::Value::Null)),
        _ => Err(format!(
            "script error: {}",
            report
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown failure")
        )),
    }
}

/// The JavaScript message behind a QuickJS exception.
///
/// `Error::Exception` carries nothing, so the pending exception is read back
/// off the context; without this every script failure would read
/// "Exception generated by QuickJS" and the model would learn nothing.
fn describe(ctx: &Ctx, error: rquickjs::Error) -> String {
    if !matches!(error, rquickjs::Error::Exception) {
        return error.to_string();
    }
    let message = ctx
        .catch()
        .as_object()
        .and_then(|object| Exception::from_object(object.clone()))
        .and_then(|exception| exception.message())
        .unwrap_or_else(|| error.to_string());
    format!("script error: {message}")
}

/// One bridge call: enforce the call budget, then run the tool.
fn invoke(tools: &ToolTable, calls: &AtomicUsize, tool: &str, args: &str) -> String {
    if calls.fetch_add(1, Ordering::Relaxed) >= MAX_TOOL_CALLS {
        return failure(&format!("tool call limit of {MAX_TOOL_CALLS} reached"));
    }
    let Some(handler) = tools.get(tool) else {
        return failure(&format!("unknown tool '{tool}'"));
    };
    let Ok(args) = serde_json::from_str(args) else {
        return failure("tool arguments are not valid JSON");
    };
    match handler(args) {
        Ok(value) => serde_json::json!({ "ok": true, "value": value }).to_string(),
        Err((_, message)) => failure(&message),
    }
}

/// A rejected call, as the bridge payload the script turns into a `throw`.
fn failure(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One tool that echoes its arguments, so a test can prove the bridge
    /// carries real JSON in both directions.
    fn echo_table() -> ToolTable {
        [("echo", Arc::new(Ok) as ToolHandler)]
            .into_iter()
            .collect()
    }

    fn run(code: &str, tools: &ToolTable) -> Result<serde_json::Value, String> {
        Sandbox::new().unwrap().run(code, tools)
    }

    #[test]
    fn a_script_composes_tools_and_returns_one_value() {
        let table = echo_table();
        let result = run(
            r#"const a = await tools.echo({ n: 1 });
               const b = await tools.echo({ n: 2 });
               return { total: a.n + b.n, both: [a, b] };"#,
            &table,
        )
        .unwrap();
        assert_eq!(result["total"], 3);
        assert_eq!(result["both"][1]["n"], 2);
    }

    #[test]
    fn parallel_calls_settle_before_the_script_returns() {
        // `Promise.all` needs the job queue pumped, which is what `finish` does.
        let table = echo_table();
        let result = run(
            r#"const out = await Promise.all([1, 2, 3].map((n) => tools.echo({ n })));
               return out.map((row) => row.n);"#,
            &table,
        )
        .unwrap();
        assert_eq!(result, serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn a_body_returning_nothing_yields_null() {
        assert_eq!(
            run("const x = 1;", &echo_table()).unwrap(),
            serde_json::Value::Null
        );
    }

    #[test]
    fn a_tool_failure_becomes_a_script_error_with_the_original_message() {
        let table: ToolTable = [(
            "boom",
            Arc::new(|_| Err((-32602i64, "metric 'vibes' is unknown".to_owned()))) as ToolHandler,
        )]
        .into_iter()
        .collect();
        let error = run("return await tools.boom({});", &table).unwrap_err();
        assert_eq!(error, "script error: metric 'vibes' is unknown");
    }

    #[test]
    fn a_tool_outside_the_table_cannot_be_called() {
        let table = echo_table();
        // Through the documented namespace the name is simply absent.
        let error = run("return await tools.nope({});", &table).unwrap_err();
        assert!(error.contains("not a function"), "{error}");
        // Through the bridge global the name is still refused explicitly.
        let result = run(
            r#"return JSON.parse(__leadline("nope", "{}")).error;"#,
            &table,
        )
        .unwrap();
        assert_eq!(result, "unknown tool 'nope'");
    }

    #[test]
    fn the_call_budget_stops_a_runaway_loop() {
        let table = echo_table();
        let error = run(
            "for (let i = 0; i < 500; i++) { await tools.echo({ i }); } return 1;",
            &table,
        )
        .unwrap_err();
        assert!(error.contains("tool call limit"), "{error}");
    }

    #[test]
    fn the_script_has_no_host_capabilities() {
        let table = echo_table();
        let result = run(
            "return [typeof require, typeof process, typeof fetch, typeof setTimeout];",
            &table,
        )
        .unwrap();
        assert_eq!(
            result,
            serde_json::json!(["undefined", "undefined", "undefined", "undefined"])
        );
    }

    #[test]
    fn a_syntax_error_reports_the_javascript_message() {
        let error = run("const = ;", &echo_table()).unwrap_err();
        assert!(error.contains("variable name expected"), "{error}");
    }

    #[test]
    fn a_thrown_error_reports_its_message() {
        let error = run("throw new Error(\"deliberate\");", &echo_table()).unwrap_err();
        assert_eq!(error, "script error: deliberate");
    }

    #[test]
    fn a_body_containing_quotes_and_newlines_needs_no_escaping() {
        // The body travels as a global value, so it is never spliced into
        // Rust-side source.
        let result = run("const s = \"a\\\"b\";\nreturn s + \"\\n\";", &echo_table()).unwrap();
        assert_eq!(result, "a\"b\n");
    }

    #[test]
    fn a_runaway_loop_is_interrupted() {
        // The deadline is the authority: an infinite loop must not hang the
        // server. Kept short by trusting the interrupt handler, not a sleep.
        let error = run("while (true) {}", &echo_table()).unwrap_err();
        assert!(error.contains("30s"), "{error}");
    }
}
