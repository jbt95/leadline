// Pi/OMP registration shim: three leadline tools and warn-mode post-edit
// feedback. Tool behavior, binary discovery, and formatting live in
// ../core/index.js; the core never imports harness APIs.

import type { ExtensionAPI, ToolResultEvent } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { DEFAULT_BASE, postEditFeedback, runChanged, runCheck, runFunction } from "../core/index.js";

function toolResult(text: string) {
  return { content: [{ type: "text" as const, text }], details: {} };
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
    execute: async (_toolCallId, params) => toolResult(await runChanged(params)),
  });

  pi.registerTool({
    name: "leadline_function",
    label: "Leadline Function",
    description: "Analyze one function's complexity metrics (cognitive, cyclomatic, CRAP, coverage).",
    parameters: Type.Object({
      file: Type.String({ description: "Source file path" }),
      name: Type.String({ description: "Function name" }),
    }),
    execute: async (_toolCallId, params) => toolResult(await runFunction(params)),
  });

  pi.registerTool({
    name: "leadline_check",
    label: "Leadline Check",
    description: "Run the leadline quality gate over a path and list functions above the thresholds.",
    parameters: Type.Object({
      path: Type.Optional(Type.String({ description: "Path to check (default .)" })),
    }),
    execute: async (_toolCallId, params) => toolResult(await runCheck(params)),
  });

  pi.on("tool_result", async (event: ToolResultEvent, ctx) => {
    if (event.isError || (event.toolName !== "edit" && event.toolName !== "write")) {
      return;
    }
    const note = await postEditFeedback({ base: DEFAULT_BASE }, "warn", ctx.cwd);
    if (note === null) {
      return;
    }
    return { content: [...event.content, { type: "text" as const, text: `leadline:\n${note}` }] };
  });
}
