// leadline native plugin for OpenCode. Thin wrapper: shells out to the
// `leadline` binary and returns compact JSON. Never reimplements metrics.
// Tool names are stable: leadline_changed, leadline_function, leadline_check.
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const runFile = promisify(execFile);

interface LeadlineArgs {
  base?: string;
  path?: string;
  name?: string;
}

interface LeadlineFailure {
  stdout?: string;
  message: string;
}

function runLeadline(args: string[], limit = 4000): Promise<string> {
  return runFile("leadline", args, { timeout: 60_000, maxBuffer: 4 * 1024 * 1024 }).then(
    ({ stdout }) => stdout.slice(0, limit),
    (error: LeadlineFailure) => (error.stdout ?? `leadline failed: ${error.message}`).slice(0, limit),
  );
}

export const tools = [
  {
    name: "leadline_changed",
    description: "Analyze changed functions vs git base for complexity regressions.",
    parameters: { base: "git base revision, default HEAD~1" },
    execute({ base = "HEAD~1" }: LeadlineArgs) {
      return runLeadline(["changed", "--base", base, "--format", "agent-json"]);
    },
  },
  {
    name: "leadline_function",
    description: "Show metrics for one function.",
    parameters: { path: "source file", name: "function name" },
    execute({ path, name }: LeadlineArgs) {
      return runLeadline(["function", path ?? "", name ?? "", "--json"]);
    },
  },
  {
    name: "leadline_check",
    description: "Quality-gate check over changed code (warn mode, never blocks).",
    parameters: {},
    execute() {
      return runLeadline(["check", ".", "--format", "agent-json"]);
    },
  },
];

export default { tools };
