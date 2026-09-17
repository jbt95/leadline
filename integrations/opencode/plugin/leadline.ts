// leadline native plugin for OpenCode V1. Thin wrapper over the shared
// adapter core: binary discovery, argument building, decoding, and
// formatting all live in ../../agent-adapter-ts/core/index.js, so the V1 and
// V2 tools behave identically. The `leadline` binary owns every metric.
// Tool names are stable: leadline_changed, leadline_function, leadline_gate,
// leadline_secret_check. `leadline_gate` (not `leadline_check`) because MCP
// clients expose the `leadline` server's `check` tool as `leadline_check`,
// and that namespaced tool silently shadows a native tool with the same name.
import {
  runChanged,
  runCheck,
  runFunction,
  runSecretGate,
  secretGateMessage,
} from "../../agent-adapter-ts/core/index.js";

interface LeadlineArgs {
  base?: string;
  path?: string;
  name?: string;
  mode?: string;
}

/// Failures surface as text so the model sees the analyzer error instead of a
/// rejected tool call.
function text(run: () => Promise<string>): Promise<string> {
  return run().catch((error: unknown) =>
    error instanceof Error ? error.message : String(error),
  );
}

export const tools = [
  {
    name: "leadline_changed",
    description: "Call after editing Java/JS/TS/TSX, fixing bugs, refactoring, or before committing to spot complexity regressions vs a git base.",
    parameters: {
      base: "git base revision, default HEAD~1",
      path: "limit analysis to this path",
    },
    execute({ base, path }: LeadlineArgs) {
      return text(() => runChanged({ base, path }));
    },
  },
  {
    name: "leadline_function",
    description: "Call when a flagged function needs inspection; shows cognitive, cyclomatic, CRAP, and coverage for one function.",
    parameters: { path: "source file", name: "function name" },
    execute({ path, name }: LeadlineArgs) {
      return text(() => runFunction({ file: path ?? "", name: name ?? "" }));
    },
  },
  {
    name: "leadline_gate",
    description: "Call before committing or in CI to gate changed code against quality thresholds (warn mode, never blocks).",
    parameters: {},
    execute() {
      return text(() => runCheck({}));
    },
  },
  {
    name: "leadline_secret_check",
    description: "Call only when asked to scan for secrets; checks worktree or staged files via the shared gate (warn mode, never blocks).",
    parameters: { mode: "worktree|staged, default worktree" },
    execute({ mode = "worktree" }: LeadlineArgs) {
      return text(() =>
        runSecretGate(".", mode === "staged" ? "staged" : "worktree").then(secretGateMessage),
      );
    },
  },
];

export default { tools };
