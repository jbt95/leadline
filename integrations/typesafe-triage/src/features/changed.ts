//! Changed/regression triage: rank the functions whose metrics regressed.
//!
//! Report: `leadline changed --base REV --json`. Only paired functions with a
//! positive delta and newly added functions enter the worklist; improved and
//! unchanged functions are not triage material.

import { bucketFor, normalize, round } from "../compose.ts";
import {
  asArray,
  asObject,
  optionalNumber,
  optionalString,
  requireObjectValue,
  requireString,
  type JsonObject,
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

export type ChangedDeltas = {
  cognitive: number;
  cyclomatic: number;
  max_nesting: number;
  crap: number;
};

export type ChangedItem = {
  path: string;
  name: string | null;
  line: number | null;
  added: boolean;
  deltas: ChangedDeltas;
  metrics: MetricSet;
  deltaTotal: number;
  weight: number;
};

export type ChangedContext = { base: string | null; analyzerVersion: string | null };

export type ChangedState = {
  report: { kind: "changed"; base: string | null; analyzer: string | null };
  items: {
    path: string;
    name: string | null;
    line: number | null;
    added: boolean;
    deltas: ChangedDeltas;
    metrics: MetricSet;
  }[];
};

export type RankedChangedItem = {
  kind: "changed";
  path: string;
  name: string | null;
  line: number | null;
  added: boolean;
  deltas: ChangedDeltas;
  metrics: MetricSet;
  priority: number;
  bucket: Bucket;
  judgments: Judgments;
};

export const changedFeature: Feature<
  ChangedItem,
  RankedChangedItem,
  ChangedContext,
  ChangedState
> = {
  id: "changed",
  title: "Changed/regression triage",
  summary:
    "Ranks changed functions whose metrics regressed, weighing the delta against TypeSafe judgments of role, inherent complexity, and attention.",

  buildItems(report: JsonValue) {
    const root = requireObjectValue(report, "changed report");
    const items: ChangedItem[] = [];

    for (const value of asArray(root.functions)) {
      const entry = asObject(value);

      if (entry === null) {
        continue;
      }

      const after = asObject(entry.after);
      const afterMetrics = after === null ? null : asObject(after.metrics);

      if (after === null || afterMetrics === null) {
        continue;
      }

      const before = asObject(entry.before);
      const beforeMetrics = before === null ? null : asObject(before.metrics);

      const deltas: ChangedDeltas = {
        cognitive: metricDelta(afterMetrics, beforeMetrics, "cognitive"),
        cyclomatic: metricDelta(afterMetrics, beforeMetrics, "cyclomatic"),
        max_nesting: metricDelta(afterMetrics, beforeMetrics, "max_nesting"),
        crap: metricDelta(afterMetrics, beforeMetrics, "crap"),
      };

      // Only improvements cancel nothing: any positive dimension keeps the row,
      // and the weight sums the positive parts (including CRAP) so a large
      // regression in one dimension cannot be erased by another's improvement.
      const regressed =
        deltas.cognitive > 0 ||
        deltas.cyclomatic > 0 ||
        deltas.max_nesting > 0 ||
        deltas.crap > 0;

      const deltaTotal = round(
        Math.max(deltas.cognitive, 0) +
          Math.max(deltas.cyclomatic, 0) +
          Math.max(deltas.max_nesting, 0) +
          Math.max(deltas.crap, 0),
        2,
      );

      const added = before === null;

      if (!added && !regressed) {
        continue;
      }

      items.push({
        path: requireString(entry, "path", "changed function"),
        name: optionalString(entry, "name"),
        line: optionalNumber(after, "start_line"),
        added,
        deltas,
        metrics: metricSet(afterMetrics),
        deltaTotal,
        weight: 0,
      });
    }

    const weights = normalize(items.map((item) => item.deltaTotal));

    const weighted = items.map((item, index) => ({
      ...item,
      weight: round(weights[index]),
    }));

    const notes: string[] = [];

    if (weighted.length === 0) {
      notes.push("no changed function regressed: nothing to triage");
    }

    return {
      items: weighted,
      notes,
      context: {
        base: optionalString(root, "base"),
        analyzerVersion: optionalString(root, "analyzer_version"),
      },
    };
  },

  buildState(items: ChangedItem[], context: ChangedContext): ChangedState {
    return {
      report: {
        kind: "changed",
        base: context.base,
        analyzer: context.analyzerVersion,
      },
      items: items.map((item) => ({
        path: item.path,
        name: item.name,
        line: item.line,
        added: item.added,
        deltas: item.deltas,
        metrics: item.metrics,
      })),
    };
  },

  buildQuestions(items: ChangedItem[]): Questions {
    return mergeQuestions(items.map((_, index) => codeItemQuestions(index)));
  },

  rankItem(item: ChangedItem, answers: Answers, index: number): RankedChangedItem {
    const judged = parseTrio(answers, index);

    const priority = trioPriority({
      weight: item.weight,
      role: judged.role,
      inherent: judged.inherent,
      attention: judged.attention,
    });

    return {
      kind: "changed",
      path: item.path,
      name: item.name,
      line: item.line,
      added: item.added,
      deltas: item.deltas,
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

function metricDelta(after: JsonObject, before: JsonObject | null, key: string): number {
  const afterValue = optionalNumber(after, key) ?? 0;
  const beforeValue = before === null ? 0 : (optionalNumber(before, key) ?? 0);

  return round(afterValue - beforeValue, 2);
}
