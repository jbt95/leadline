//! Security/vulnerability triage: rank scanner findings by how exposed and
//! reachable they look in this codebase.
//!
//! Reports: `leadline security --sarif FILE --json` (security findings) or
//! `leadline vulnerabilities --osv/--trivy FILE --json` (dependency findings).
//! Both are normalized to one shape. This feature ranks; it never suppresses a
//! finding and never changes a scanner gate.

import { choice, noul } from "../client.ts";
import {
  bucketFor,
  choiceOf,
  confidenceOf,
  judgmentEntry,
  noulOf,
  round,
  weightOf,
} from "../compose.ts";
import {
  asArray,
  asObject,
  isString,
  optionalBoolean,
  optionalNumber,
  optionalString,
  requireObjectValue,
  type JsonObject,
  type JsonValue,
} from "../json.ts";
import { mergeQuestions } from "../judgments.ts";
import type { Feature } from "../pipeline.ts";
import type { Answers, Bucket, Judgments, Questions } from "../types.ts";

const EXPOSURE_WEIGHTS = {
  direct_runtime: 1,
  transitive_runtime: 0.7,
  dev_or_build: 0.35,
  test_or_example: 0.2,
  unclear: 0.5,
} satisfies { [exposure: string]: number };

const EXPOSURE_CRITERIA = {
  direct_runtime: "A direct dependency or code path used at runtime",
  transitive_runtime: "Reached through another runtime dependency",
  dev_or_build: "Development, build, or CI tooling only",
  test_or_example: "Tests, examples, fixtures, or documentation",
  unclear: "The available evidence does not identify the exposure",
} satisfies { [exposure: string]: string };

/** Absolute severity bands, not run-relative: a critical finding must not
 * outrank a low one by virtue of being the only item in its run. */
const SEVERITY_WEIGHTS = {
  critical: 1,
  high: 0.8,
  medium: 0.5,
  low: 0.25,
  unknown: 0.4,
} satisfies { [severity: string]: number };

const CHANGED_WEIGHT = 1;

const UNCHANGED_WEIGHT = 0.7;

export type SecurityScanner = "security" | "vulnerabilities";

export type SecurityItem = {
  scanner: SecurityScanner;
  path: string | null;
  label: string;
  line: number | null;
  severity: string;
  changed: boolean;
  detail: { [key: string]: string | number | boolean | string[] | null };
  weight: number;
};

export type SecurityContext = null;

export type SecurityState = {
  report: { kind: "scanner" };
  items: {
    scanner: SecurityScanner;
    path: string | null;
    label: string;
    line: number | null;
    severity: string;
    changed: boolean;
    detail: { [key: string]: string | number | boolean | string[] | null };
  }[];
};

export type RankedSecurityItem = {
  kind: "security";
  scanner: SecurityScanner;
  path: string | null;
  label: string;
  line: number | null;
  severity: string;
  priority: number;
  bucket: Bucket;
  judgments: Judgments;
  detail: { [key: string]: string | number | boolean | string[] | null };
};

export const securityFeature: Feature<
  SecurityItem,
  RankedSecurityItem,
  SecurityContext,
  SecurityState
> = {
  id: "security",
  title: "Security/vulnerability triage",
  summary:
    "Ranks scanner findings by severity, exposure, and how reachable they look; advisory only, findings are never suppressed.",

  buildItems(report: JsonValue) {
    const root = requireObjectValue(report, "scanner report");
    const findings = asArray(root.findings);
    const items: SecurityItem[] = [];

    for (const value of findings) {
      const item = normalizeFinding(asObject(value));

      items.push({
        ...item,
        weight: round(
          weightOf(SEVERITY_WEIGHTS, item.severity, SEVERITY_WEIGHTS.unknown) *
            (item.changed ? CHANGED_WEIGHT : UNCHANGED_WEIGHT),
        ),
      });
    }

    const notes: string[] = [];

    if (items.length === 0) {
      notes.push("no scanner findings in the report");
    }

    return { items, notes, context: null };
  },

  buildState(items: SecurityItem[]): SecurityState {
    return {
      report: { kind: "scanner" },
      items: items.map((item) => ({
        scanner: item.scanner,
        path: item.path,
        label: item.label,
        line: item.line,
        severity: item.severity,
        changed: item.changed,
        detail: item.detail,
      })),
    };
  },

  buildQuestions(items: SecurityItem[]): Questions {
    return mergeQuestions(
      items.map((_, index) => securityQuestions(index)),
    );
  },

  rankItem(item: SecurityItem, answers: Answers, index: number): RankedSecurityItem {
    const exposureAnswer = answers[`f${index}__exposure`];
    const reachableAnswer = answers[`f${index}__reachable`];
    const exposure = choiceOf(exposureAnswer);
    const reachable = noulOf(reachableAnswer) ?? 0.5;
    const exposureWeight = weightOf(EXPOSURE_WEIGHTS, exposure, 0.5);
    const priority = item.weight * exposureWeight * (0.2 + 0.8 * reachable);

    return {
      kind: "security",
      scanner: item.scanner,
      path: item.path,
      label: item.label,
      line: item.line,
      severity: item.severity,
      priority: round(priority),
      bucket: bucketFor({
        attention: null,
        confidence: confidenceOf(exposureAnswer),
      }),
      judgments: {
        exposure: judgmentEntry(exposureAnswer, exposure),
        reachable,
      },
      detail: item.detail,
    };
  },
};

function securityQuestions(index: number): Questions {
  const reference = `items[${index}]`;
  const questions: Questions = {};
  questions[`f${index}__exposure`] = choice(
    `How is ${reference} exposed in this codebase? Judge from its path, scanner, severity, and reachability evidence.`,
    EXPOSURE_CRITERIA,
  );
  questions[`f${index}__reachable`] = noul(
    `Does the evidence suggest ${reference} is realistically reachable or exercised, rather than merely declared?`,
    {
      true: "Reachable or exercised on a path this project runs",
      false: "Declared or listed, but no evidence it is reached",
    },
  );

  return questions;
}

type NormalizedFinding = Omit<SecurityItem, "weight">;

function normalizeFinding(finding: JsonObject | null): NormalizedFinding {
  if (finding === null) {
    return unknownFinding();
  }

  const ruleId = optionalString(finding, "rule_id");

  if (ruleId !== null) {
    return {
      scanner: "security",
      path: optionalString(finding, "path"),
      label: ruleId,
      line: optionalNumber(finding, "start_line"),
      severity: (optionalString(finding, "severity") ?? "unknown").toLowerCase(),
      changed: optionalBoolean(finding, "changed") === true,
      detail: {
        tool: optionalString(finding, "tool"),
        reason: optionalString(finding, "reason"),
        risk_score: optionalNumber(finding, "risk_score"),
        fan_in: optionalNumber(finding, "fan_in"),
        blast_radius: optionalNumber(finding, "blast_radius"),
      },
    };
  }

  const advisoryId = optionalString(finding, "advisory_id");

  if (advisoryId !== null) {
    return {
      scanner: "vulnerabilities",
      path: optionalString(finding, "manifest_path"),
      label: `${optionalString(finding, "package") ?? "package"} ${advisoryId}`,
      line: null,
      severity: (optionalString(finding, "severity") ?? "unknown").toLowerCase(),
      changed: optionalBoolean(finding, "reachable_from_changed") === true,
      detail: {
        package: optionalString(finding, "package"),
        installed_version: optionalString(finding, "installed_version"),
        fixed_versions: stringEntries(asArray(finding.fixed_versions)),
        reachable_from_changed: optionalBoolean(finding, "reachable_from_changed") === true,
        changed_imports: changedImportPaths(asArray(finding.changed_imports)),
      },
    };
  }

  return unknownFinding();
}

/** Rows of an unrecognized shape are preserved at unknown severity instead of
 * dropped: triage ranks every finding and never suppresses one. */
function unknownFinding(): NormalizedFinding {
  return {
    scanner: "security",
    path: null,
    label: "(unrecognized finding)",
    line: null,
    severity: "unknown",
    changed: false,
    detail: {},
  };
}

/** Leadline serializes changed imports as `{path, package}` objects; the
 * package is already in `detail`, so the evidence kept here is the path. */
function changedImportPaths(values: JsonValue[]): string[] {
  const paths: string[] = [];

  for (const value of values) {
    const entry = asObject(value);
    const path = entry === null ? null : optionalString(entry, "path");

    if (path !== null) {
      paths.push(path);
    }
  }

  return paths;
}

function stringEntries(values: JsonValue[]): string[] {
  const entries: string[] = [];

  for (const value of values) {
    if (isString(value)) {
      entries.push(value);
    }
  }

  return entries;
}
