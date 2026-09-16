// Pi/OMP registration shim: four leadline tools and warn-mode post-edit
// feedback. Tool behavior, binary discovery, and formatting live in
// ../core/index.js; the core never imports harness APIs.

import type { ExtensionAPI, ToolResultEvent } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { DEFAULT_BASE, postEditFeedback, runChanged, runCheck, runFunction, runSecretGate, secretGateMessage } from "../core/index.js";

function toolResult(text: string) {
  return { content: [{ type: "text" as const, text }], details: {} };
}

// Analyzer failures surface as tool text, matching the OpenCode wrappers: the
// model still sees the error, and no harness-level rejection is raised.
async function textTool(run: () => Promise<string>) {
  try {
    return toolResult(await run());
  } catch (error) {
    return toolResult(error instanceof Error ? error.message : String(error));
  }
}

export function registerLeadlineExtension(pi: ExtensionAPI): void {
  pi.registerTool({
    name: "leadline_changed",
    label: "Leadline Changed",
    description: "Analyze functions changed relative to a git base revision and report complexity regressions.",
    parameters: Type.Object({
      base: Type.Optional(Type.String({ description: "Git revision to compare against (default HEAD~1)" })),
      path: Type.Optional(Type.String({ description: "Only analyze this path" })),
    }),
    execute: async (_toolCallId, params) => textTool(() => runChanged(params)),
  });

  pi.registerTool({
    name: "leadline_function",
    label: "Leadline Function",
    description: "Analyze one function's complexity metrics (cognitive, cyclomatic, CRAP, coverage).",
    parameters: Type.Object({
      file: Type.String({ description: "Source file path" }),
      name: Type.String({ description: "Function name" }),
    }),
    execute: async (_toolCallId, params) => textTool(() => runFunction(params)),
  });

  pi.registerTool({
    name: "leadline_gate",
    label: "Leadline Gate",
    description: "Run the leadline quality gate over a path and list functions above the thresholds.",
    parameters: Type.Object({
      path: Type.Optional(Type.String({ description: "Path to check (default .)" })),
    }),
    execute: async (_toolCallId, params) => textTool(() => runCheck(params)),
  });

  pi.registerTool({
    name: "leadline_secret_check",
    label: "Leadline Secret Check",
    description: "Scan worktree or staged files for secrets via the shared gate. Fails the call on findings or an unavailable scanner.",
    parameters: Type.Object({
      root: Type.Optional(Type.String({ description: "Project root to scan (default .)" })),
      mode: Type.Optional(Type.Union([Type.Literal("worktree"), Type.Literal("staged")], { description: "Scan scope (default worktree)" })),
    }),
    execute: async (_toolCallId, params) => {
      const result = await runSecretGate(
        typeof params.root === "string" && params.root.length > 0 ? params.root : ".",
        params.mode === "staged" ? "staged" : "worktree",
      );
      // The explicit gate is the blocking surface: findings and an
      // unavailable scanner reject the call instead of reading as success.
      if (result.status !== "clean") {
        throw new Error(secretGateMessage(result));
      }
      return toolResult(secretGateMessage(result));
    },
  });

  pi.on("tool_result", async (event: ToolResultEvent, ctx) => {
    if (event.isError || (event.toolName !== "edit" && event.toolName !== "write")) {
      return;
    }
    const content = [...event.content];
    const note = await postEditFeedback({ base: DEFAULT_BASE }, "warn", ctx.cwd);
    if (note !== null) {
      content.push({ type: "text" as const, text: `leadline:\n${note}` });
    }
    // The edit event cannot veto an already-completed write, so secret
    // findings surface as a warning here; blocking lives in the
    // `leadline_secret_check` tool and the pre-commit hook instead.
    try {
      const gate = await runSecretGate(ctx.cwd, "worktree");
      if (gate.status === "findings") {
        content.push({
          type: "text" as const,
          text: `leadline secret gate: possible secrets detected\n${gate.detail}`,
        });
      }
    } catch {
      // Unavailable scanner or misconfiguration stays silent post-edit.
    }
    if (content.length === event.content.length) {
      return;
    }
    return { content };
  });
}
