#!/usr/bin/env node
//! leadline-triage: run one triage feature over a leadline JSON report and
//! print a ranked worklist composed from TypeSafe judgments.
//!
//! This is the companion binary's entry point; leadline itself stays offline
//! and this tool never runs unless invoked. Advisory only: it never gates, and
//! it never suppresses a finding.

import { readFile } from "node:fs/promises";
import { configFromEnv } from "../src/client.ts";
import { planRoute, routeRequest, type RoutedRequest } from "../src/features/route.ts";
import { isString } from "../src/json.ts";
import { DEFAULT_BATCH_SIZE, MAX_BATCH_SIZE, type RunResult } from "../src/pipeline.ts";
import {
  featureSpecs,
  isReportFeatureId,
  planReportFeature,
  runReportFeature,
  type RankedItem,
} from "../src/registry.ts";
import { readReport } from "../src/report.ts";
import type { Judgments } from "../src/types.ts";

class UsageError extends Error {}

const USAGE = `leadline-triage — rank leadline reports with TypeSafe judgments

Usage:
  leadline-triage <feature> [--input FILE] [--json] [--dry-run] [--batch-size N]
  leadline-triage route "request text" [--json] [--dry-run]

Features:
  changed      Changed/regression triage   (leadline changed --base REV --json)
  check        Repo-wide check triage      (leadline check ... --json)
  security     Security/vulnerability triage
               (leadline security --sarif FILE --json, or vulnerabilities --osv/--trivy FILE --json)
  debt         Debt acceptance review      (leadline debt --base REV --json)
  duplication  Duplication intent triage   (leadline duplication --json)
  route        Workflow routing for a free-form request

The report is read from --input FILE, or stdin when the flag is omitted.
Requires TYPESAFE_API_KEY. TYPESAFE_BASE_URL points at a mirror,
TYPESAFE_DEFAULT_MODEL picks another model, TYPESAFE_TIMEOUT_MS changes the
per-request timeout. --dry-run prints the exact requests without calling
TypeSafe.

Examples:
  leadline changed --base origin/main --json | leadline-triage changed
  leadline-triage security --input security.json --json
  leadline-triage route "did my last edit make things worse?"`;

type CliOptions = {
  input: string | null;
  json: boolean;
  dryRun: boolean;
  batchSize: number;
  help: boolean;
  positionals: string[];
};

async function main(argv: string[]): Promise<number> {
  const [command, ...rest] = argv;

  if (command === undefined || command === "help" || command === "--help" || command === "-h") {
    console.log(USAGE);

    return 0;
  }

  if (command === "list" || command === "--list") {
    console.log(JSON.stringify(featureSpecs(), null, 2));

    return 0;
  }

  const options = parseOptions(rest);

  if (options.help) {
    console.log(USAGE);

    return 0;
  }

  if (command === "route") {
    let text = options.positionals.join(" ").trim();

    if (text === "" && options.input !== null) {
      text = await readText(options.input);
    }

    if (text === "" && !process.stdin.isTTY) {
      text = await readText(null);
    }

    if (text === "") {
      throw new UsageError("route needs the request text as an argument or via --input FILE");
    }

    if (options.dryRun) {
      console.log(JSON.stringify({ dryRun: true, ...planRoute({ request: text }) }, null, 2));

      return 0;
    }

    const result = await routeRequest({ request: text, config: configFromEnv(process.env) });
    console.log(
      options.json ? JSON.stringify({ dryRun: false, ...result }, null, 2) : routeLine(result),
    );

    return 0;
  }

  if (!isReportFeatureId(command)) {
    console.error(`leadline-triage: unknown feature '${command}'\n`);
    console.error(USAGE);

    return 2;
  }

  const report = await readReport({ input: options.input });

  if (options.dryRun) {
    console.log(planReportFeature(command, report, options.batchSize));

    return 0;
  }

  const result = await runReportFeature(command, report, {
    config: configFromEnv(process.env),
    batchSize: options.batchSize,
  });

  const spec = featureSpecs().find((candidate) => candidate.id === command);
  console.log(
    options.json
      ? JSON.stringify({ dryRun: false, ...result }, null, 2)
      : reportLines(spec?.title ?? command, result),
  );

  return 0;
}

function parseOptions(args: string[]): CliOptions {
  const options: CliOptions = {
    input: null,
    json: false,
    dryRun: false,
    batchSize: DEFAULT_BATCH_SIZE,
    help: false,
    positionals: [],
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];

    if (arg === "--input") {
      const value = args[index + 1];

      if (value === undefined) {
        throw new UsageError("--input requires a file path");
      }

      options.input = value;
      index += 1;
    } else if (arg === "--json") {
      options.json = true;
    } else if (arg === "--dry-run") {
      options.dryRun = true;
    } else if (arg === "--batch-size") {
      const value = Number(args[index + 1]);

      if (!Number.isInteger(value) || value < 1 || value > MAX_BATCH_SIZE) {
        throw new UsageError(`--batch-size must be an integer from 1 to ${MAX_BATCH_SIZE}`);
      }

      options.batchSize = value;
      index += 1;
    } else if (arg === "--help" || arg === "-h") {
      options.help = true;
    } else if (arg.startsWith("-")) {
      throw new UsageError(`unknown option '${arg}'`);
    } else {
      options.positionals.push(arg);
    }
  }

  return options;
}

function reportLines(featureTitle: string, result: RunResult<RankedItem>): string {
  const lines = [
    `${featureTitle} — ${result.items.length} item(s), model ${result.model ?? "n/a"}, ${result.batches} request(s)`,
  ];

  result.items.forEach((item, index) => {
    lines.push(
      `${String(index + 1).padStart(2)}. ${labelOf(item)}  priority ${item.priority}  [${item.bucket}]`,
    );
    const judgments = judgmentLine(item.judgments);

    if (judgments !== "") {
      lines.push(`    ${judgments}`);
    }

    const detail = detailLine(item);

    if (detail !== "") {
      lines.push(`    ${detail}`);
    }
  });

  for (const note of result.notes) {
    lines.push(`note: ${note}`);
  }

  lines.push(
    `usage: ${result.usage.inputTokens} input / ${result.usage.outputTokens} output tokens`,
  );

  return lines.join("\n");
}

function routeLine(result: RoutedRequest): string {
  const lines = [
    `workflow: ${result.workflow ?? "unknown"}${confidenceSuffix(result.confidence)}  action: ${result.action}`,
  ];

  if (result.command !== null) {
    lines.push(`command: ${result.command}`);
  }

  lines.push(
    `alternatives: ${result.alternatives
      .map((entry) => `${entry.workflow} ${entry.probability}`)
      .join(" · ")}`,
  );
  lines.push(
    `usage: ${result.usage.inputTokens} input / ${result.usage.outputTokens} output tokens`,
  );

  return lines.join("\n");
}

function labelOf(item: RankedItem): string {
  switch (item.kind) {
    case "duplication": {
      const id = String(item.id ?? "").slice(0, 8);

      return `group ${id} (${item.occurrences.map((entry) => entry.path).join(" ~ ")})`;
    }

    case "security":
      return `${item.label} (${item.path ?? "unknown path"})`;
    case "changed":
    case "check":
    case "debt":
      return [item.path, item.name].filter((part) => part !== null).join("::");
  }
}

function judgmentLine(judgments: Judgments): string {
  const parts: string[] = [];

  if (judgments.role !== undefined) {
    parts.push(`role ${judgments.role.value ?? "?"}${confidenceSuffix(judgments.role.confidence)}`);
  }

  if (judgments.inherent !== undefined) {
    parts.push(
      `inherent ${judgments.inherent.value ?? "?"}${confidenceSuffix(judgments.inherent.confidence)}`,
    );
  }

  if (judgments.attention !== undefined) {
    parts.push(`attention ${round3(judgments.attention)}`);
  }

  if (judgments.exposure !== undefined) {
    parts.push(
      `exposure ${judgments.exposure.value ?? "?"}${confidenceSuffix(judgments.exposure.confidence)}`,
    );
  }

  if (judgments.reachable !== undefined) {
    parts.push(`reachable ${round3(judgments.reachable)}`);
  }

  if (judgments.intent !== undefined) {
    parts.push(`intent ${judgments.intent.value ?? "?"}${confidenceSuffix(judgments.intent.confidence)}`);
  }

  return parts.join(" · ");
}

function detailLine(item: RankedItem): string {
  switch (item.kind) {
    case "changed":
      return `Δ cognitive ${signed(item.deltas.cognitive)} → cognitive ${item.metrics.cognitive ?? "?"}, cyclomatic ${item.metrics.cyclomatic ?? "?"}, nesting ${item.metrics.max_nesting ?? "?"}${item.added ? " (new function)" : ""}`;
    case "check":
      return `metrics cognitive ${item.metrics.cognitive ?? "?"}, cyclomatic ${item.metrics.cyclomatic ?? "?"}, nesting ${item.metrics.max_nesting ?? "?"}`;
    case "security": {
      const reason = item.detail.reason;
      const fixed = item.detail.fixed_versions;
      const reasonText = isString(reason) ? ` · ${reason}` : "";

      const fixedText =
        Array.isArray(fixed) && fixed.length > 0 ? ` · fixed in ${fixed.join(", ")}` : "";

      return `severity ${item.severity}${reasonText}${fixedText}`;
    }

    case "debt":
      return item.group === "risk"
        ? `risk ${item.before ?? "?"} → ${item.after ?? "?"} (Δ ${item.components.length} component(s))`
        : `${item.dimension ?? "unknown"} ${item.before ?? "?"} → ${item.after ?? "?"} (limit ${item.threshold ?? "?"})`;
    case "duplication":
      return `${item.tokens} tokens · ${item.lines} lines · ${item.occurrences.length} occurrences`;
  }
}

const round3 = (value: number): number => Number(value.toFixed(3));

const signed = (value: number): string => (value > 0 ? `+${value}` : `${value}`);

const confidenceSuffix = (confidence: number | null): string =>
  confidence === null ? "" : ` ${round3(confidence)}`;

async function readText(input: string | null): Promise<string> {
  if (input !== null && input !== "-") {
    return (await readFile(input, "utf8")).trim();
  }

  const chunks: Uint8Array[] = [];

  for await (const chunk of process.stdin) {
    chunks.push(Buffer.from(chunk));
  }

  return Buffer.concat(chunks).toString("utf8").trim();
}

main(process.argv.slice(2))
  .then((code) => {
    process.exitCode = code;
  })
  .catch((error: Error) => {
    console.error(`leadline-triage: ${error.message}`);
    process.exitCode = error instanceof UsageError ? 2 : 3;
  });
