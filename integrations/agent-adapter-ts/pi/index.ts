// Pi/OMP registration shim: four leadline tools and warn-mode post-edit
// feedback. Tool behavior, binary discovery, and formatting live in
// ../core/index.js; the core never imports harness APIs.

import type { ExtensionAPI, ToolResultEvent } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { DEFAULT_BASE, postEditFeedback, runChanged, runCheck, runFunction } from "../core/index.js";

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
    description: "Call after editing C/C++/Go/Java/JS/TS/TSX/Zig/Rust/Python, fixing bugs, refactoring, or before committing to spot complexity regressions vs a git base.",
    parameters: Type.Object({
      base: Type.Optional(Type.String({ description: "Git revision to compare against (default HEAD~1)" })),
      path: Type.Optional(Type.String({ description: "Only analyze this path" })),
    }),
    execute: async (_toolCallId, params) => textTool(() => runChanged(params)),
  });

  pi.registerTool({
    name: "leadline_function",
    label: "Leadline Function",
    description: "Call when a flagged function needs inspection; shows cognitive, cyclomatic, CRAP, and coverage for one function.",
    parameters: Type.Object({
      file: Type.String({ description: "Source file path" }),
      name: Type.String({ description: "Function name" }),
    }),
    execute: async (_toolCallId, params) => textTool(() => runFunction(params)),
  });

  pi.registerTool({
    name: "leadline_gate",
    label: "Leadline Gate",
    description: "Call before committing or in CI to gate changed code against quality thresholds (warn mode, never blocks).",
    parameters: Type.Object({
      path: Type.Optional(Type.String({ description: "Path to check (default .)" })),
    }),
    execute: async (_toolCallId, params) => textTool(() => runCheck(params)),
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
    if (content.length === event.content.length) {
      return;
    }
    return { content };
  });
}
