// Shared leadline adapter core.
//
// The `leadline` Rust binary owns every metric. This module only discovers
// the binary, invokes it with fixed argument lists, decodes its agent-json
// output into the small domain shapes the harness shims format, and runs
// warn-mode post-edit feedback. No harness API is imported here.

import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { isJsonObject } from "./json.js";
import type { JsonObject, JsonValue } from "./json.js";

const execFileAsync = promisify(execFile);

export const BINARY_NAME = "leadline";
export const DEFAULT_BASE = "HEAD~1";
export const MAX_FUNCTION_LINES = 50;
export const NO_FUNCTIONS_MESSAGE = "No functions reported.";
/** Thresholds used only when neither CLI flags nor leadline.toml define any. */
export const CHECK_THRESHOLDS = ["--cognitive", "15", "--cyclomatic", "10", "--max-nesting", "4"] as const;

const AGENT_FORMAT = "agent-json";
const MAX_BUFFER_BYTES = 16 * 1024 * 1024;
const WELL_KNOWN_LOCATIONS: string[] = [
  join(homedir(), ".cargo/bin", BINARY_NAME),
  "/usr/local/bin/leadline",
  "/opt/homebrew/bin/leadline",
];

// --- Domain types (decoded analyzer output, never raw JSON) ---

export interface FunctionMetrics {
  cognitive: number;
  cyclomatic: number;
  crap: number | null;
  coverage: number | null;
}

export interface AnalysisFunction extends FunctionMetrics {
  name: string;
  line: number;
}

/** One syntax-error span the analyzer could not parse. */
export interface ParseDiagnostic {
  line: number;
  kind: string;
}

export interface AnalysisFile {
  path: string;
  functions: AnalysisFunction[];
  parseErrors: ParseDiagnostic[];
}

export interface AnalysisReport {
  files: AnalysisFile[];
  truncated: boolean;
}

export interface ChangedEntry {
  path: string;
  function: string;
  line: number;
  before: FunctionMetrics;
  after: FunctionMetrics;
}

/** One syntax-error span in the before or after revision of one file. */
export interface ChangedParseError extends ParseDiagnostic {
  path: string;
  phase: "before" | "after";
}

export interface ChangedReport {
  base: string;
  summary: { changedFunctions: number; regressions: number; improvements: number };
  regressions: ChangedEntry[];
  improvements: ChangedEntry[];
  parseErrors: ChangedParseError[];
  truncated: boolean;
}

// --- Tool inputs ---

export interface ChangedInput {
  base?: string;
  path?: string;
}

export interface FunctionInput {
  file: string;
  name: string;
}

export interface CheckInput {
  path?: string;
}

export type PostEditMode = "advisory" | "warn" | "gate";

export interface PostEditEvent {
  path?: string;
  base?: string;
}

// --- Binary discovery ---

export function discoverBinary(): string {
  const directories = (process.env.PATH ?? "").split(delimiter);
  for (const directory of directories) {
    if (directory.length === 0) {
      continue;
    }
    const candidate = join(directory, BINARY_NAME);
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  for (const location of WELL_KNOWN_LOCATIONS) {
    if (existsSync(location)) {
      return location;
    }
  }
  throw new Error(
    `leadline binary not found. Searched PATH and ${WELL_KNOWN_LOCATIONS.join(", ")}. ` +
      "Build it from the leadline repository with `cargo build --release` and put it on your PATH.",
  );
}

// --- Process execution (fixed argument lists; JSON stdout) ---

async function runLeadline(args: string[], cwd?: string): Promise<string> {
  const binary = discoverBinary();
  try {
    const { stdout } = await execFileAsync(binary, args, { cwd, maxBuffer: MAX_BUFFER_BYTES });
    return stdout;
  } catch (error) {
    const failure = error as { stdout?: string; stderr?: string; message?: string };
    // `leadline check` exits 1 when a threshold is violated and still writes
    // the violating functions to stdout; keep that output instead of failing.
    if (typeof failure.stdout === "string" && failure.stdout.trim().length > 0) {
      return failure.stdout;
    }
    const detail = (failure.stderr ?? "").trim();
    throw new Error(
      `leadline ${args[0] ?? "command"} failed: ${detail.length > 0 ? detail : (failure.message ?? "unknown error")}`,
    );
  }
}

// --- Boundary decoders ---

function expectNumber(raw: JsonObject, field: string): number {
  const value = raw[field];
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`leadline: expected numeric field '${field}' in analyzer output`);
  }
  return value;
}

function expectNullableNumber(raw: JsonObject, field: string): number | null {
  const value = raw[field];
  if (value === null || value === undefined) {
    return null;
  }
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`leadline: expected numeric or null field '${field}' in analyzer output`);
  }
  return value;
}

function decodeMetrics(raw: JsonObject): FunctionMetrics {
  return {
    cognitive: expectNumber(raw, "cognitive"),
    cyclomatic: expectNumber(raw, "cyclomatic"),
    crap: expectNullableNumber(raw, "crap"),
    coverage: expectNullableNumber(raw, "coverage"),
  };
}

function decodeDiagnostics(raw: JsonValue | undefined, what: string): ParseDiagnostic[] {
  if (!Array.isArray(raw)) {
    throw new Error(`leadline: ${what} is missing its parse_errors array`);
  }
  const diagnostics: ParseDiagnostic[] = [];
  for (const entry of raw) {
    if (!isJsonObject(entry)) {
      throw new Error(`leadline: ${what} parse_errors entries must be objects`);
    }
    const line = entry.start_line;
    const kind = entry.kind;
    diagnostics.push({
      line: typeof line === "number" && Number.isFinite(line) ? line : 0,
      kind: typeof kind === "string" && kind.length > 0 ? kind : "parse-error",
    });
  }
  return diagnostics;
}

function parseObject(stdout: string, what: string): JsonObject {
  let parsed: JsonValue;
  try {
    parsed = JSON.parse(stdout) as JsonValue;
  } catch {
    throw new Error(`leadline: ${what} output is not valid JSON`);
  }
  if (!isJsonObject(parsed)) {
    throw new Error(`leadline: expected a JSON object in ${what} output`);
  }
  return parsed;
}

export function decodeAnalysis(stdout: string): AnalysisReport {
  const parsed = parseObject(stdout, "analyzer");
  const filesRaw = parsed.files;
  if (!Array.isArray(filesRaw)) {
    throw new Error("leadline: analyzer output is missing its files array");
  }
  const files: AnalysisFile[] = [];
  for (const fileRaw of filesRaw) {
    if (!isJsonObject(fileRaw)) {
      throw new Error("leadline: expected file objects in analyzer output");
    }
    const path = fileRaw.path;
    if (typeof path !== "string" || path.length === 0) {
      throw new Error("leadline: file entry is missing its path");
    }
    const functionsRaw = fileRaw.functions;
    if (!Array.isArray(functionsRaw)) {
      throw new Error(`leadline: file '${path}' is missing its functions array`);
    }
    const functions: AnalysisFunction[] = [];
    for (const functionRaw of functionsRaw) {
      if (!isJsonObject(functionRaw)) {
        throw new Error(`leadline: expected function objects for '${path}'`);
      }
      const name = functionRaw.name;
      if (typeof name !== "string" || name.length === 0) {
        throw new Error(`leadline: function in '${path}' is missing its name`);
      }
      functions.push({ name, line: expectNumber(functionRaw, "line"), ...decodeMetrics(functionRaw) });
    }
    files.push({
      path,
      functions,
      parseErrors: decodeDiagnostics(fileRaw.parse_errors, `file '${path}'`),
    });
  }
  return { files, truncated: parsed.truncated === true };
}

function decodeChangedEntries(raw: JsonValue | undefined, field: string): ChangedEntry[] {
  if (!Array.isArray(raw)) {
    throw new Error(`leadline: changed output is missing its ${field} array`);
  }
  const entries: ChangedEntry[] = [];
  for (const entryRaw of raw) {
    if (!isJsonObject(entryRaw)) {
      throw new Error(`leadline: expected entries in changed ${field}`);
    }
    const path = entryRaw.path;
    const name = entryRaw.function;
    if (typeof path !== "string" || path.length === 0 || typeof name !== "string" || name.length === 0) {
      throw new Error(`leadline: changed ${field} entry is missing its path or function name`);
    }
    const beforeRaw = entryRaw.before;
    const afterRaw = entryRaw.after;
    if (!isJsonObject(beforeRaw) || !isJsonObject(afterRaw)) {
      throw new Error(`leadline: changed ${field} entry for '${path}:${name}' is missing before/after metrics`);
    }
    entries.push({
      path,
      function: name,
      line: expectNumber(entryRaw, "line"),
      before: decodeMetrics(beforeRaw),
      after: decodeMetrics(afterRaw),
    });
  }
  return entries;
}

function decodeChangedParseErrors(raw: JsonValue | undefined): ChangedParseError[] {
  if (!Array.isArray(raw)) {
    throw new Error("leadline: changed output is missing its parse_errors array");
  }
  const errors: ChangedParseError[] = [];
  for (const entryRaw of raw) {
    if (!isJsonObject(entryRaw)) {
      throw new Error("leadline: expected entries in changed parse_errors");
    }
    const path = entryRaw.path;
    if (typeof path !== "string" || path.length === 0) {
      throw new Error("leadline: changed parse_errors entry is missing its path");
    }
    for (const phase of ["before", "after"] as const) {
      for (const diagnostic of decodeDiagnostics(
        entryRaw[phase],
        `changed ${phase} for '${path}'`,
      )) {
        errors.push({ path, phase, ...diagnostic });
      }
    }
  }
  return errors;
}

export function decodeChanged(stdout: string): ChangedReport {
  const parsed = parseObject(stdout, "changed");
  const base = parsed.base;
  if (typeof base !== "string" || base.length === 0) {
    throw new Error("leadline: changed output is missing its base revision");
  }
  const summaryRaw = parsed.summary;
  if (!isJsonObject(summaryRaw)) {
    throw new Error("leadline: changed output is missing its summary");
  }
  return {
    base,
    summary: {
      changedFunctions: expectNumber(summaryRaw, "changed_functions"),
      regressions: expectNumber(summaryRaw, "regressions"),
      improvements: expectNumber(summaryRaw, "improvements"),
    },
    regressions: decodeChangedEntries(parsed.regressions, "regressions"),
    improvements: decodeChangedEntries(parsed.improvements, "improvements"),
    parseErrors: decodeChangedParseErrors(parsed.parse_errors),
    truncated: parsed.truncated === true,
  };
}

// --- Compact formatting (projection only; no metric computation) ---

export function formatScore(value: number | null): string {
  return value === null ? "n/a" : value.toFixed(1);
}

function capLines(lines: string[]): string[] {
  if (lines.length <= MAX_FUNCTION_LINES) {
    return lines;
  }
  return [
    ...lines.slice(0, MAX_FUNCTION_LINES),
    `... and ${lines.length - MAX_FUNCTION_LINES} more not shown (capped at ${MAX_FUNCTION_LINES})`,
  ];
}

function analysisLine(file: string, fn: AnalysisFunction): string {
  return (
    `${file}:${fn.name} line ${fn.line} cognitive ${fn.cognitive}, cyclomatic ${fn.cyclomatic}, ` +
    `crap ${formatScore(fn.crap)}, coverage ${formatScore(fn.coverage)}`
  );
}

function changeLine(kind: "regression" | "improvement", entry: ChangedEntry): string {
  const before = entry.before;
  const after = entry.after;
  const parts: string[] = [];
  if (after.cognitive !== before.cognitive) {
    parts.push(`cognitive ${before.cognitive}->${after.cognitive}`);
  }
  if (after.cyclomatic !== before.cyclomatic) {
    parts.push(`cyclomatic ${before.cyclomatic}->${after.cyclomatic}`);
  }
  if (after.crap !== before.crap) {
    parts.push(`crap ${formatScore(before.crap)}->${formatScore(after.crap)}`);
  }
  const detail = parts.length > 0 ? ` ${parts.join(", ")}` : "";
  return `${entry.path}:${entry.function} line ${entry.line} ${kind}${detail}`;
}

export function formatAnalysis(report: AnalysisReport): string {
  const lines: string[] = [];
  for (const file of report.files) {
    for (const fn of file.functions) {
      lines.push(analysisLine(file.path, fn));
    }
    for (const error of file.parseErrors) {
      lines.push(`${file.path} parse error at line ${error.line} (${error.kind})`);
    }
  }
  if (report.truncated) {
    lines.push("... truncated: analyzer output was capped");
  }
  if (lines.length === 0) {
    return NO_FUNCTIONS_MESSAGE;
  }
  return capLines(lines).join("\n");
}

export function formatChanged(report: ChangedReport): string {
  const summary = report.summary;
  const lines = [
    `${summary.changedFunctions} changed function(s): ${summary.regressions} regression(s), ${summary.improvements} improvement(s)`,
  ];
  for (const entry of report.regressions) {
    lines.push(changeLine("regression", entry));
  }
  for (const entry of report.improvements) {
    lines.push(changeLine("improvement", entry));
  }
  for (const error of report.parseErrors) {
    lines.push(`${error.path} parse error in ${error.phase} at line ${error.line} (${error.kind})`);
  }
  if (
    report.regressions.length === 0 &&
    report.improvements.length === 0 &&
    report.parseErrors.length === 0
  ) {
    lines.push("No complexity regressions or improvements.");
  }
  if (report.truncated) {
    lines.push("... truncated: analyzer output was capped");
  }
  return capLines(lines).join("\n");
}

// --- Commands exposed to harnesses ---

function changedArgs(input: ChangedInput): string[] {
  const args = ["changed", "--base", input.base ?? DEFAULT_BASE, "--format", AGENT_FORMAT];
  if (input.path !== undefined && input.path.length > 0) {
    args.push("--path", input.path);
  }
  return args;
}

export async function runChanged(input: ChangedInput, cwd?: string): Promise<string> {
  return formatChanged(decodeChanged(await runLeadline(changedArgs(input), cwd)));
}

export async function runFunction(input: FunctionInput, cwd?: string): Promise<string> {
  if (input.file.length === 0) {
    throw new Error("leadline: file is required");
  }
  if (input.name.length === 0) {
    throw new Error("leadline: function name is required");
  }
  const args = ["function", input.file, input.name, "--format", AGENT_FORMAT];
  return formatAnalysis(decodeAnalysis(await runLeadline(args, cwd)));
}

async function runCheckWithThresholds(args: string[], cwd?: string): Promise<string> {
  try {
    return await runLeadline(args, cwd);
  } catch (error) {
    if (!(error instanceof Error) || !error.message.includes("requires at least one metric threshold")) {
      throw error;
    }
    // No flag and no leadline.toml threshold: fall back to the documented defaults.
    return runLeadline([...args, ...CHECK_THRESHOLDS], cwd);
  }
}

export async function runCheck(input: CheckInput, cwd?: string): Promise<string> {
  const path = input.path !== undefined && input.path.length > 0 ? input.path : ".";
  const args = ["check", path, "--format", AGENT_FORMAT];
  return formatAnalysis(decodeAnalysis(await runCheckWithThresholds(args, cwd)));
}

// --- Shared secret gate (scanner via the versioned shell runner) ---
//
// The shell runner owns scanner invocation and redacted report handoff;
// this module executes it with a fixed empty argument list and maps its
// exit codes. No detection patterns live here.

export type SecretGateMode = "worktree" | "staged";

export type SecretGateStatus = "clean" | "findings" | "unavailable";

export interface AdapterResult {
  status: SecretGateStatus;
  detail: string;
}

const SECRET_RUNNER_RELATIVE = ["..", "..", "common", "leadline-secret-check.sh"];

function secretRunnerPath(): string {
  const override = process.env.LEADLINE_SECRET_RUNNER;
  if (override !== undefined && override.length > 0) {
    return override;
  }
  return join(fileURLToPath(new URL(".", import.meta.url)), ...SECRET_RUNNER_RELATIVE);
}

function capDetail(text: string): string {
  return capLines(text.split("\n")).join("\n");
}

export async function runSecretGate(root: string, mode: SecretGateMode): Promise<AdapterResult> {
  const runner = secretRunnerPath();
  try {
    await execFileAsync(runner, [], {
      cwd: root,
      env: { ...process.env, LEADLINE_SECRET_MODE: mode },
      maxBuffer: MAX_BUFFER_BYTES,
    });
    return { status: "clean", detail: "" };
  } catch (error) {
    const failure = error as { code?: string | number; stdout?: string; stderr?: string };
    if (failure.code === "ENOENT") {
      return { status: "unavailable", detail: "secret gate runner not found" };
    }
    const code = typeof failure.code === "number" ? failure.code : -1;
    const stdout = typeof failure.stdout === "string" ? failure.stdout : "";
    const stderr = typeof failure.stderr === "string" ? failure.stderr : "";
    if (code === 1) {
      return { status: "findings", detail: capDetail(stdout) };
    }
    if (code === 127) {
      const hint = stderr.trim().length > 0 ? stderr.trim() : "secret scanner unavailable";
      return { status: "unavailable", detail: capDetail(hint) };
    }
    // Exit 3 is the runner's "no git comparison target": the worktree gate
    // skipped before scanning, so report it like any other environment miss.
    if (code === 3) {
      const hint = stderr.trim().length > 0 ? stderr.trim() : "no git comparison target";
      return { status: "unavailable", detail: capDetail(hint) };
    }
    const detail = stderr.trim().length > 0 ? stderr.trim() : `secret gate failed with exit ${code}`;
    throw new Error(capDetail(detail));
  }
}

/// One-line status for an explicit secret-check tool call.
///
/// `unavailable` must never read as clean: a missing runner or scanner means
/// the repository was not scanned, so the caller has to say so.
export function secretGateMessage(result: AdapterResult): string {
  if (result.status === "findings") {
    return `leadline secret gate: possible secrets detected\n${result.detail}`;
  }
  if (result.status === "unavailable") {
    return `leadline secret gate unavailable: ${result.detail}`;
  }
  return "leadline secret gate: clean";
}

// Post-edit feedback runs in warn mode only: it returns null (stays silent)
// when disabled, when nothing regressed, or when the analyzer fails, so it can
// never gate or break the edit flow.
export async function postEditFeedback(event: PostEditEvent, mode: PostEditMode, cwd?: string): Promise<string | null> {
  if (mode !== "warn") {
    return null;
  }
  let report: ChangedReport;
  try {
    report = decodeChanged(await runLeadline(changedArgs(event), cwd));
  } catch {
    return null;
  }
  if (report.regressions.length === 0) {
    return null;
  }
  return capLines(report.regressions.map((entry) => changeLine("regression", entry))).join("\n");
}
