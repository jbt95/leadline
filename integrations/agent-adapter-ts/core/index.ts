// Shared leadline adapter core.
//
// The `leadline` Rust binary owns every metric. This module only discovers
// the binary, invokes it with fixed argument lists, decodes its JSON stdout
// into named domain types, and formats compact agent-facing text.

import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { delimiter, join } from "node:path";
import { homedir } from "node:os";
import { promisify } from "node:util";
import { isJsonObject } from "./json.js";
import type { JsonObject, JsonValue } from "./json.js";

const execFileAsync = promisify(execFile);

export const BINARY_NAME = "leadline";
export const MAX_FUNCTION_LINES = 50;
export const DEFAULT_BASE = "HEAD~1";
export const NO_FUNCTIONS_MESSAGE = "No functions reported.";

const AGENT_FORMAT = "agent-json";

const WELL_KNOWN_LOCATIONS: string[] = [
  join(homedir(), ".cargo/bin", BINARY_NAME),
  "/usr/local/bin/leadline",
  "/opt/homebrew/bin/leadline",
];

// --- Domain types (decoded analyzer output; never raw JSON) ---

export interface MetricSnapshot {
  cognitive: number;
  cyclomatic: number;
  crap: number | null;
  coverage: number | null;
}

export interface FunctionSnapshot {
  name: string;
  startLine: number;
  endLine: number;
  metrics: MetricSnapshot;
}

export interface AnalysisFile {
  path: string;
  functions: FunctionSnapshot[];
  parseErrorCount: number;
}

export interface AnalysisReport {
  kind: "analysis";
  files: AnalysisFile[];
}

export interface FunctionChange {
  path: string;
  name: string;
  before: FunctionSnapshot | null;
  after: FunctionSnapshot | null;
}

export interface ChangedReport {
  kind: "changed";
  base: string;
  changes: FunctionChange[];
  parseErrorCount: number;
}

export type AgentReport = AnalysisReport | ChangedReport;

// --- Tool inputs (validated at each tool boundary) ---

export interface ChangedInput {
  base: string;
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

export interface ExtensionHost {
  registerTools?: (tools: LeadlineTools) => void;
  onPostEdit?: (listener: (event: PostEditEvent) => Promise<string | null>) => void;
}

export interface LeadlineTool<TInput> {
  name: string;
  description: string;
  run: (input: TInput) => Promise<string>;
}

export interface LeadlineTools {
  changed: LeadlineTool<ChangedInput>;
  function: LeadlineTool<FunctionInput>;
  check: LeadlineTool<CheckInput>;
}

// --- Binary discovery ---

function findOnPath(): string | null {
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
  return null;
}

export function discoverBinary(): string {
  const onPath = findOnPath();
  if (onPath !== null) {
    return onPath;
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

async function runLeadline(args: string[]): Promise<string> {
  const binary = discoverBinary();
  let stdout: string;
  try {
    const result = await execFileAsync(binary, args, { maxBuffer: 16 * 1024 * 1024 });
    stdout = result.stdout;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`leadline ${args[0]} failed: ${message}`);
  }
  return stdout;
}

function requireNonEmpty(value: string | undefined, label: string): string {
  if (value === undefined || value.length === 0) {
    throw new Error(`leadline: ${label} is required`);
  }
  return value;
}

// --- Boundary decoders (raw JSON in, named domain types out) ---

function decodeFiniteNumber(raw: JsonObject, field: string): number {
  const value = raw[field];
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`leadline: expected numeric field '${field}' in analyzer output`);
  }
  return value;
}

function decodeNullableNumber(raw: JsonObject, field: string): number | null {
  const value = raw[field];
  if (value === null || value === undefined) {
    return null;
  }
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`leadline: expected numeric or null field '${field}' in analyzer output`);
  }
  return value;
}

function decodeSnapshot(raw: JsonValue): FunctionSnapshot {
  if (!isJsonObject(raw)) {
    throw new Error("leadline: expected a function object in analyzer output");
  }
  const name = raw.name;
  if (typeof name !== "string" || name.length === 0) {
    throw new Error("leadline: function entry is missing its name");
  }
  const metricsRaw = raw.metrics;
  if (!isJsonObject(metricsRaw)) {
    throw new Error(`leadline: function '${name}' is missing its metrics`);
  }
  return {
    name,
    startLine: decodeFiniteNumber(raw, "start_line"),
    endLine: decodeFiniteNumber(raw, "end_line"),
    metrics: {
      cognitive: decodeFiniteNumber(metricsRaw, "cognitive"),
      cyclomatic: decodeFiniteNumber(metricsRaw, "cyclomatic"),
      crap: decodeNullableNumber(metricsRaw, "crap"),
      coverage: decodeNullableNumber(metricsRaw, "coverage"),
    },
  };
}

function decodeNullableSnapshot(raw: JsonValue | undefined): FunctionSnapshot | null {
  if (raw === null || raw === undefined) {
    return null;
  }
  return decodeSnapshot(raw);
}

function countParseErrors(raw: JsonValue | undefined): number {
  if (!Array.isArray(raw)) {
    throw new Error("leadline: expected a parse_errors array in analyzer output");
  }
  return raw.length;
}

function decodeAnalysisReport(raw: JsonObject): AnalysisReport {
  const filesRaw = raw.files;
  if (!Array.isArray(filesRaw)) {
    throw new Error("leadline: expected a files array in analyzer output");
  }
  const files: AnalysisFile[] = [];
  for (const entry of filesRaw) {
    if (!isJsonObject(entry)) {
      throw new Error("leadline: expected file objects in analyzer output");
    }
    const path = entry.path;
    if (typeof path !== "string" || path.length === 0) {
      throw new Error("leadline: file entry is missing its path");
    }
    const functionsRaw = entry.functions;
    if (!Array.isArray(functionsRaw)) {
      throw new Error(`leadline: file '${path}' is missing its functions array`);
    }
    const functions: FunctionSnapshot[] = [];
    for (const item of functionsRaw) {
      functions.push(decodeSnapshot(item));
    }
    files.push({ path, functions, parseErrorCount: countParseErrors(entry.parse_errors) });
  }
  return { kind: "analysis", files };
}

function decodeChangedReport(raw: JsonObject): ChangedReport {
  const base = raw.base;
  if (typeof base !== "string" || base.length === 0) {
    throw new Error("leadline: changed output is missing its base revision");
  }
  const functionsRaw = raw.functions;
  if (!Array.isArray(functionsRaw)) {
    throw new Error("leadline: expected a functions array in changed output");
  }
  const changes: FunctionChange[] = [];
  for (const entry of functionsRaw) {
    if (!isJsonObject(entry)) {
      throw new Error("leadline: expected function-change objects in changed output");
    }
    const path = entry.path;
    const name = entry.name;
    if (typeof path !== "string" || path.length === 0 || typeof name !== "string" || name.length === 0) {
      throw new Error("leadline: function-change entry is missing its path or name");
    }
    changes.push({
      path,
      name,
      before: decodeNullableSnapshot(entry.before),
      after: decodeNullableSnapshot(entry.after),
    });
  }
  return { kind: "changed", base, changes, parseErrorCount: countParseErrors(raw.parse_errors) };
}

export function decodeAgentReport(stdout: string): AgentReport {
  const parsed: JsonValue = JSON.parse(stdout);
  if (!isJsonObject(parsed)) {
    throw new Error("leadline: expected a JSON object at the top level of analyzer output");
  }
  if ("files" in parsed) {
    return decodeAnalysisReport(parsed);
  }
  return decodeChangedReport(parsed);
}

// --- Compact formatting (projection only; no metric computation) ---

// Single stable float format for every score shown to agents.
export function formatScore(value: number): string {
  if (!Number.isFinite(value)) {
    return "n/a";
  }
  return value.toFixed(1);
}

function snapshotLine(snapshot: FunctionSnapshot): string {
  const metrics = snapshot.metrics;
  const crap = metrics.crap === null ? "n/a" : formatScore(metrics.crap);
  const coverage = metrics.coverage === null ? "n/a" : formatScore(metrics.coverage);
  return (
    `lines ${snapshot.startLine}-${snapshot.endLine} ` +
    `cognitive ${metrics.cognitive}, cyclomatic ${metrics.cyclomatic}, ` +
    `crap ${crap}, coverage ${coverage}`
  );
}

function changeLine(change: FunctionChange): string {
  if (change.after === null) {
    return `${change.path}:${change.name} removed`;
  }
  if (change.before === null) {
    return `${change.path}:${change.name} added (${snapshotLine(change.after)})`;
  }
  const lines: string[] = [];
  const before = change.before.metrics;
  const after = change.after.metrics;
  if (after.cognitive !== before.cognitive) {
    lines.push(`cognitive ${before.cognitive}->${after.cognitive}`);
  }
  if (after.cyclomatic !== before.cyclomatic) {
    lines.push(`cyclomatic ${before.cyclomatic}->${after.cyclomatic}`);
  }
  if (after.crap !== before.crap) {
    const beforeText = before.crap === null ? "n/a" : formatScore(before.crap);
    const afterText = after.crap === null ? "n/a" : formatScore(after.crap);
    lines.push(`crap ${beforeText}->${afterText}`);
  }
  if (lines.length === 0) {
    return `${change.path}:${change.name} unchanged`;
  }
  return `${change.path}:${change.name} ${lines.join(", ")}`;
}

function truncateLines(lines: string[]): string[] {
  if (lines.length <= MAX_FUNCTION_LINES) {
    return lines;
  }
  const kept = lines.slice(0, MAX_FUNCTION_LINES);
  kept.push(`... and ${lines.length - MAX_FUNCTION_LINES} more not shown (capped at ${MAX_FUNCTION_LINES})`);
  return kept;
}

export function formatCompact(report: AgentReport): string {
  const lines: string[] = [];
  let uncovered = 0;
  if (report.kind === "changed") {
    for (const change of report.changes) {
      const text = changeLine(change);
      if (text.indexOf("unchanged") === -1) {
        lines.push(text);
      }
      const snapshot = change.after ?? change.before;
      if (snapshot !== null && snapshot.metrics.coverage === null) {
        uncovered += 1;
      }
    }
    if (report.parseErrorCount > 0) {
      lines.push(`parse errors: ${report.parseErrorCount}`);
    }
  } else {
    for (const file of report.files) {
      for (const snapshot of file.functions) {
        lines.push(`${file.path}:${snapshot.name} ${snapshotLine(snapshot)}`);
        if (snapshot.metrics.coverage === null) {
          uncovered += 1;
        }
      }
    }
    let parseErrors = 0;
    for (const file of report.files) {
      parseErrors += file.parseErrorCount;
    }
    if (parseErrors > 0) {
      lines.push(`parse errors: ${parseErrors}`);
    }
  }
  if (uncovered > 0) {
    lines.push(`Note: coverage unavailable for ${uncovered} function(s); CRAP may be missing.`);
  }
  if (lines.length === 0) {
    return NO_FUNCTIONS_MESSAGE;
  }
  return truncateLines(lines).join("\n");
}

// --- Regression check for warn-mode post-edit feedback ---

function isRegression(before: MetricSnapshot, after: MetricSnapshot): boolean {
  if (after.cognitive > before.cognitive || after.cyclomatic > before.cyclomatic) {
    return true;
  }
  return before.crap !== null && after.crap !== null && after.crap > before.crap;
}

// --- Tools (each validates its named input, then runs a fixed arg list) ---

export function createTools(): LeadlineTools {
  return {
    changed: {
      name: "leadline_changed",
      description: "Analyze functions changed relative to a git base revision with the leadline binary.",
      run: async (input: ChangedInput): Promise<string> => {
        const base = requireNonEmpty(input.base, "base revision");
        const args = ["changed", "--base", base, "--format", AGENT_FORMAT];
        if (input.path !== undefined && input.path.length > 0) {
          args.push("--path", input.path);
        }
        return formatCompact(decodeAgentReport(await runLeadline(args)));
      },
    },
    function: {
      name: "leadline_function",
      description: "Analyze one named function in a file with the leadline binary.",
      run: async (input: FunctionInput): Promise<string> => {
        const file = requireNonEmpty(input.file, "file");
        const name = requireNonEmpty(input.name, "function name");
        const args = ["function", file, name, "--format", AGENT_FORMAT];
        return formatCompact(decodeAgentReport(await runLeadline(args)));
      },
    },
    check: {
      name: "leadline_check",
      description: "List functions exceeding quality thresholds with the leadline binary.",
      run: async (input: CheckInput): Promise<string> => {
        const path = input.path === undefined || input.path.length === 0 ? "." : input.path;
        const args = ["check", path, "--format", AGENT_FORMAT];
        return formatCompact(decodeAgentReport(await runLeadline(args)));
      },
    },
  };
}

// Post-edit feedback runs in warn mode only: it returns null (stays silent)
// when disabled, when nothing regressed, or when the analyzer fails, so it
// can never gate or break the edit flow.
export async function postEditFeedback(input: ChangedInput, mode: PostEditMode): Promise<string | null> {
  if (mode !== "warn") {
    return null;
  }
  const base = requireNonEmpty(input.base, "base revision");
  const args = ["changed", "--base", base, "--format", AGENT_FORMAT];
  if (input.path !== undefined && input.path.length > 0) {
    args.push("--path", input.path);
  }
  let report: AgentReport;
  try {
    report = decodeAgentReport(await runLeadline(args));
  } catch {
    return null;
  }
  if (report.kind !== "changed") {
    return null;
  }
  const lines: string[] = [];
  for (const change of report.changes) {
    if (change.after === null) {
      continue;
    }
    if (change.before === null || isRegression(change.before.metrics, change.after.metrics)) {
      lines.push(changeLine(change));
    }
  }
  if (lines.length === 0) {
    return null;
  }
  return truncateLines(lines).join("\n");
}
