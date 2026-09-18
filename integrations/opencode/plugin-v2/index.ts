// leadline native plugin for OpenCode V2.
//
// EXPERIMENTAL: the V2 plugin API is unstable (written against
// `@opencode/plugin` 2.0.2 and opencode2 v0.0.0-beta-18269). Pin the
// dependency in package.json and expect breakage across V2 betas.
//
// Thin wrapper over the shared adapter core: the `leadline` binary owns all
// metrics and formatting. Tool names stay stable:
// leadline_changed, leadline_function, leadline_gate, leadline_secret_check.
// `leadline_gate` (not `leadline_check`) because MCP clients expose the
// `leadline` server's `check` tool as `leadline_check`, and a native tool
// with the same name is silently shadowed by that namespaced tool.
import { Plugin } from "@opencode/plugin";
import { DEFAULT_BASE, postEditFeedback, runChanged, runCheck, runFunction, runSecretGate, secretGateMessage } from "../../agent-adapter-ts/core/index.js";

interface ChangedInput {
  base?: string;
  path?: string;
}

interface FunctionInput {
  path?: string;
  name?: string;
}

// V2 code mode reads `output`; `content` carries the display text. Failures are
// returned as text so the model sees the analyzer error instead of a bare
// "no output" result.
async function textResult(run: () => Promise<string>) {
  try {
    const text = await run();
    return { output: text, content: [{ type: "text" as const, text }] };
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error);
    return { output: text, content: [{ type: "text" as const, text }] };
  }
}

export default Plugin.define({
  id: "leadline",
  async setup(ctx) {
    // The V2 tool context carries only the session ID; resolve the project
    // directory per call so relative paths match the session's project.
    const directoryFor = async (sessionID: string): Promise<string | undefined> => {
      try {
        const info = await ctx.session.get({ sessionID: sessionID as Parameters<typeof ctx.session.get>[0]["sessionID"] });
        return info.location.directory;
      } catch {
        return undefined;
      }
    };

    await ctx.tool.transform((editor) => {
      editor.add({
        name: "leadline_changed",
        description: "Call after editing Java/JS/TS/TSX, fixing bugs, refactoring, or before committing to spot complexity regressions vs a git base.",
        input: {
          type: "object",
          properties: {
            base: { type: "string", description: "git base revision, default HEAD~1" },
            path: { type: "string", description: "limit analysis to this path" },
          },
          additionalProperties: false,
        },
        output: { type: "string" },
        execute: async (input, context) => {
          const { base, path } = input as ChangedInput;
          const directory = await directoryFor(context.sessionID);
          return textResult(() => runChanged({ base, path }, directory));
        },
      });
      editor.add({
        name: "leadline_function",
        description: "Call when a flagged function needs inspection; shows cognitive, cyclomatic, CRAP, and coverage for one function.",
        input: {
          type: "object",
          properties: {
            path: { type: "string", description: "source file" },
            name: { type: "string", description: "function name" },
          },
          additionalProperties: false,
        },
        output: { type: "string" },
        execute: async (input, context) => {
          const { path = "", name = "" } = input as FunctionInput;
          const directory = await directoryFor(context.sessionID);
          return textResult(() => runFunction({ file: path, name }, directory));
        },
      });
      editor.add({
        name: "leadline_gate",
        description: "Call before committing or in CI to gate changed code against quality thresholds (warn mode, never blocks).",
        input: { type: "object", properties: {}, additionalProperties: false },
        output: { type: "string" },
        execute: async (_input, context) => {
          const directory = await directoryFor(context.sessionID);
          return textResult(() => runCheck({}, directory));
        },
      });
      editor.add({
        name: "leadline_secret_check",
        description: "Call only when asked to scan for secrets; checks worktree or staged files via the shared gate (warn mode, never blocks).",
        input: {
          type: "object",
          properties: {
            mode: { type: "string", description: "scan scope: worktree or staged, default worktree" },
          },
          additionalProperties: false,
        },
        output: { type: "string" },
        execute: async (input, context) => {
          const { mode } = input as { mode?: string };
          const directory = await directoryFor(context.sessionID);
          return textResult(async () => {
            const result = await runSecretGate(
              directory ?? ".",
              mode === "staged" ? "staged" : "worktree",
            );
            return secretGateMessage(result);
          });
        },
      });
    });
    await ctx.tool.hook("execute.after", async (event) => {
      if (event.status !== "completed") {
        return;
      }
      if (event.tool !== "edit" && event.tool !== "write") {
        return;
      }
      const directory = await directoryFor(event.sessionID);
      const note = await postEditFeedback({ base: DEFAULT_BASE }, "warn", directory);
      if (note === null) {
        return;
      }
      const prior = event.result.content;
      const text = `leadline:\n${note}`;
      if (typeof prior === "string") {
        event.result.content = `${prior}\n${text}`;
        return;
      }
      if (Array.isArray(prior)) {
        event.result.content = [...prior, { type: "text" as const, text }];
      }
    });
  },
});
