//! Repo-wide `check` backlog triage: rank every function above the gate
//! thresholds so a backlog can be worked in priority order.
//!
//! Report: `leadline check [PATH] --<thresholds> --json`. Parse errors are
//! objective failures and are surfaced as must-fix notes instead of questions.

import { bucketFor, normalize, round } from "../compose.ts";
import {
  asArray,
  asObject,
  optionalNumber,
  optionalString,
  requireObjectValue,
  type JsonValue,
} from "../json.ts";
import {
  TRIO_POLICY,
  codeItemQuestions,
  mergeQuestions,
  parseTrio,
  trioPriority,
} from "../judgments.ts";
import { metricSet, type MetricSet } from "../metrics.ts";
import type { Feature } from "../pipeline.ts";
import type { Answers, Bucket, Judgments, Questions } from "../types.ts";

/** Reference severity scale: cognitive and cyclomatic count once, nesting
 * twice. Deliberately simple and stable; tune this one line to re-rank. */
const NESTING_WEIGHT = 2;

export type CheckItem = {
  path: string;
  name: string | null;
  line: number | null;
  metrics: MetricSet;
  severity: number;
  weight: number;
};

export type CheckContext = { analyzerVersion: string | null };

export type CheckState = {
  report: { kind: "check"; analyzer: string | null };
  items: { path: string; name: string | null; line: number | null; metrics: MetricSet }[];
};

export type RankedCheckItem = {
  kind: "check";
  path: string;
  name: string | null;
  line: number | null;
  metrics: MetricSet;
  priority: number;
  bucket: Bucket;
  judgments: Judgments;
};

export const checkFeature: Feature<CheckItem, RankedCheckItem, CheckContext, CheckState> = {
  id: "check",
  title: "Repo-wide check triage",
  summary:
    "Ranks every function above the gate thresholds against role, inherent complexity, and attention; parse errors are listed as must-fix.",

  buildItems(report: JsonValue) {
    const root = requireObjectValue(report, "check report");
    const items: CheckItem[] = [];
    const parseErrorPaths = new Set<string>();
    let parseErrorCount = 0;

    for (const fileValue of asArray(root.files)) {
      const file = asObject(fileValue);

      if (file === null) {
        continue;
      }

      const path = optionalString(file, "path") ?? "(unknown file)";
      const parseErrors = asArray(file.parse_errors);
      parseErrorCount += parseErrors.length;

      if (parseErrors.length > 0) {
        parseErrorPaths.add(path);
      }

      for (const functionValue of asArray(file.functions)) {
        const functions = asObject(functionValue);

        if (functions === null) {
          continue;
        }

        const metrics = metricSet(asObject(functions.metrics));
        items.push({
          path,
          name: optionalString(functions, "name"),
          line: optionalNumber(functions, "start_line"),
          metrics,
          severity: severityOf(metrics),
          weight: 0,
        });
      }
    }

    const weights = normalize(items.map((item) => item.severity));

    const weighted = items.map((item, index) => ({
      ...item,
      weight: round(weights[index]),
    }));

    const notes: string[] = [];

    if (parseErrorCount > 0) {
      const shown = [...parseErrorPaths].slice(0, 5).join(", ");
      const suffix = parseErrorPaths.size > 5 ? ", …" : "";
      notes.push(`must fix first: ${parseErrorCount} parse error(s) in ${shown}${suffix}`);
    }

    if (weighted.length === 0 && parseErrorCount === 0) {
      notes.push("no findings above the configured thresholds");
    }

    return {
      items: weighted,
      notes,
      context: { analyzerVersion: optionalString(root, "analyzer_version") },
    };
  },

  buildState(items: CheckItem[], context: CheckContext): CheckState {
    return {
      report: { kind: "check", analyzer: context.analyzerVersion },
      items: items.map((item) => ({
        path: item.path,
        name: item.name,
        line: item.line,
        metrics: item.metrics,
      })),
    };
  },

  buildQuestions(items: CheckItem[]): Questions {
    return mergeQuestions(items.map((_, index) => codeItemQuestions(index)));
  },

  rankItem(item: CheckItem, answers: Answers, index: number): RankedCheckItem {
    const judged = parseTrio(answers, index);

    const priority = trioPriority({
      weight: item.weight,
      role: judged.role,
      inherent: judged.inherent,
      attention: judged.attention,
    });

    return {
      kind: "check",
      path: item.path,
      name: item.name,
      line: item.line,
      metrics: item.metrics,
      priority: round(priority),
      bucket: bucketFor({
        attention: judged.attention,
        confidence: judged.confidence,
        policy: TRIO_POLICY,
      }),
      judgments: judged.judgments,
    };
  },
};

function severityOf(metrics: MetricSet): number {
  return (
    (metrics.cognitive ?? 0) +
    (metrics.cyclomatic ?? 0) +
    NESTING_WEIGHT * (metrics.max_nesting ?? 0)
  );
}
