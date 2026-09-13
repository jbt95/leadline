// leadline native plugin for OpenCode V2.
//
// EXPERIMENTAL: the V2 plugin API is unstable (written against
// `@opencode/plugin` 2.0.2 and opencode2 v0.0.0-beta-18269). Pin the
// dependency in package.json and expect breakage across V2 betas.
//
// Thin wrapper over the shared adapter core: the `leadline` binary owns all
// metrics and formatting. Tool names stay stable:
// leadline_changed, leadline_function, leadline_check.
import { Plugin } from "@opencode/plugin";
import { runChanged, runCheck, runFunction } from "../../agent-adapter-ts/core/index.js";

interface ChangedInput {
  base?: string;
  path?: string;
}

interface FunctionInput {
  path?: string;
  name?: string;
}

export default Plugin.define({
  id: "leadline",
  async setup(ctx) {
    await ctx.tool.transform((editor) => {
      editor.add({
        name: "leadline_changed",
        description: "Analyze changed functions vs a git base revision for complexity regressions.",
        input: {
          type: "object",
          properties: {
            base: { type: "string", description: "git base revision, default HEAD~1" },
            path: { type: "string", description: "limit analysis to this path" },
          },
          additionalProperties: false,
        },
        execute: async (input) => {
          const { base, path } = input as ChangedInput;
          return { content: await runChanged({ base, path }) };
        },
      });
      editor.add({
        name: "leadline_function",
        description: "Show complexity metrics for one function (cognitive, cyclomatic, CRAP, coverage).",
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
          return { content: await runFunction({ file: path, name }) };
        },
      });
      editor.add({
        name: "leadline_check",
        description: "Run the leadline quality gate over the project and list functions above the thresholds.",
        input: { type: "object", properties: {}, additionalProperties: false },
        execute: async () => ({ content: await runCheck({}) }),
      });
    });
  },
});
