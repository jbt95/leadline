// leadline native plugin for OpenCode V2.
//
// EXPERIMENTAL: the V2 plugin API is unstable (written against
// `@opencode/plugin` 2.0.2 and opencode2 v0.0.0-beta-18269). Pin the
// dependency in package.json and expect breakage across V2 betas.
//
// Thin wrapper, same contract as the V1 plugin in ../plugin/leadline.ts:
// shells out to the `leadline` binary and returns compact JSON. Never
// reimplements metrics. Effective tool names are stable:
// leadline_changed, leadline_function, leadline_check.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { Plugin } from "@opencode/plugin";

const runFile = promisify(execFile);

// Truncation bound for tool output, matching the V1 plugin.
const OUTPUT_LIMIT = 4000;

interface LeadlineFailure {
  stdout?: string;
  message: string;
}

function runLeadline(args: string[]): Promise<string> {
  return runFile("leadline", args, { timeout: 60_000, maxBuffer: 4 * 1024 * 1024 }).then(
    ({ stdout }) => stdout.slice(0, OUTPUT_LIMIT),
    (error: LeadlineFailure) => (error.stdout ?? `leadline failed: ${error.message}`).slice(0, OUTPUT_LIMIT),
  );
}

interface ChangedInput {
  base?: string;
}

interface FunctionInput {
  path?: string;
  name?: string;
}

export default Plugin.define({
  id: "leadline",
  async setup(ctx) {
    await ctx.tool.transform((editor) => {
      // No editor.namespace() call: it does not exist on all V2 betas.
      // Names carry the `leadline_` prefix directly instead.
      editor.add({
        name: "leadline_changed",
        description: "Analyze changed functions vs git base for complexity regressions.",
        input: {
          type: "object",
          properties: {
            base: { type: "string", description: "git base revision, default HEAD~1" },
          },
          additionalProperties: false,
        },
        execute: async (input) => {
          const { base = "HEAD~1" } = input as ChangedInput;
          return { content: await runLeadline(["changed", "--base", base, "--format", "agent-json"]) };
        },
      });
      editor.add({
        name: "leadline_function",
        description: "Show metrics for one function.",
        input: {
          type: "object",
          properties: {
            path: { type: "string", description: "source file" },
            name: { type: "string", description: "function name" },
          },
          additionalProperties: false,
        },
        execute: async (input) => {
          const { path = "", name = "" } = input as FunctionInput;
          return { content: await runLeadline(["function", path, name, "--json"]) };
        },
      });
      editor.add({
        name: "leadline_check",
        description: "Quality-gate check over changed code (warn mode, never blocks).",
        input: {
          type: "object",
          properties: {},
          additionalProperties: false,
        },
        execute: async () => {
          return { content: await runLeadline(["check", ".", "--format", "agent-json"]) };
        },
      });
    });
  },
});
