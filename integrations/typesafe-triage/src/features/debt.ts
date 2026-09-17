//! Debt review: triage new threshold transitions and increased risk changes.
//!
//! Report: `leadline debt --base REV --json`. Resolved and decreased rows are
//! good news and are summarized in the notes instead of judged.

import { choice, noul } from "../client.ts";
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
  ROLE_CRITERIA,
  TRIO_POLICY,
  codeItemQuestions,
  parseTrio,
  trioPriority,
} from "../judgments.ts";
import type { Feature } from "../pipeline.ts";
import type { Answers, Bucket, Judgments, Questions } from "../types.ts";

export type DebtGroup = "function" | "risk";

export type DebtComponent = { component: string; delta: number };

export type DebtItem = {
  group: DebtGroup;
  path: string;
  name: string | null;
  dimension: string | null;
  threshold: number | null;
  before: number | null;
  after: number | null;
  delta: number | null;
  components: DebtComponent[];
  severity: number;
  weight: number;
};

export type DebtContext = { base: string | null };

export type DebtState = {
  report: { kind: "debt"; base: string | null };
  items: {
    group: DebtGroup;
    path: string;
    name: string | null;
    dimension: string | null;
    threshold: number | null;
    before: number | null;
    after: number | null;
    delta: number | null;
    components: DebtComponent[];
  }[];
};

export type RankedDebtItem = {
  kind: "debt";
  group: DebtGroup;
  path: string;
  name: string | null;
  dimension: string | null;
  threshold: number | null;
  before: number | null;
  after: number | null;
  components: DebtComponent[];
  priority: number;
  bucket: Bucket;
  judgments: Judgments;
};

export const debtFeature: Feature<DebtItem, RankedDebtItem, DebtContext, DebtState> = {
  id: "debt",
  title: "Debt acceptance review",
  summary:
    "Triages new function debt and increased risk against a base revision; resolved and decreased rows are reported as wins.",

  buildItems(report: JsonValue) {
    const root = requireObjectValue(report, "debt report");
    const items: DebtItem[] = [];

    for (const value of asArray(root.findings)) {
      const finding = asObject(value);

      if (finding === null || optionalString(finding, "status") !== "new") {
        continue;
      }

      const after = optionalNumber(finding, "after");
      const threshold = optionalNumber(finding, "threshold");
      items.push({
        group: "function",
        path: requireString(finding, "path", "debt finding"),
        name: optionalString(finding, "name"),
        dimension: optionalString(finding, "dimension"),
        threshold,
        before: optionalNumber(finding, "before"),
        after,
        delta: null,
        components: [],
        severity: Math.max((after ?? 0) - (threshold ?? 0), 0),
        weight: 0,
      });
    }

    for (const value of asArray(root.risk_changes)) {
      const change = asObject(value);

      if (change === null || optionalString(change, "status") !== "increased") {
        continue;
      }

      items.push({
        group: "risk",
        path: requireString(change, "path", "risk change"),
        name: null,
        dimension: null,
        threshold: null,
        before: optionalNumber(change, "before_score"),
        after: optionalNumber(change, "after_score"),
        delta: optionalNumber(change, "delta"),
        components: componentsOf(change),
        severity: Math.max(optionalNumber(change, "delta") ?? 0, 0),
        weight: 0,
      });
    }

    const weights = normalize(items.map((item) => item.severity));

    const weighted = items.map((item, index) => ({
      ...item,
      weight: round(weights[index]),
    }));

    const summary = asObject(root.summary) ?? {};

    const notes = [
      `debt flow: ${optionalNumber(summary, "new") ?? 0} new, ` +
        `${optionalNumber(summary, "resolved") ?? 0} resolved, ` +
        `${optionalNumber(summary, "risk_increased") ?? 0} risk increased, ` +
        `${optionalNumber(summary, "risk_decreased") ?? 0} risk decreased`,
    ];

    if (weighted.length === 0) {
      notes.push("nothing new or increased to review");
    }

    return {
      items: weighted,
      notes,
      context: { base: optionalString(root, "base") },
    };
  },

  buildState(items: DebtItem[], context: DebtContext): DebtState {
    return {
      report: { kind: "debt", base: context.base },
      items: items.map((item) => ({
        group: item.group,
        path: item.path,
        name: item.name,
        dimension: item.dimension,
        threshold: item.threshold,
        before: item.before,
        after: item.after,
        delta: item.delta,
        components: item.components,
      })),
    };
  },

  buildQuestions(items: DebtItem[]): Questions {
    const questions: Questions = {};

    for (let index = 0; index < items.length; index += 1) {
      const item = items[index];

      if (item.group === "function") {
        for (const [id, question] of Object.entries(codeItemQuestions(index))) {
          questions[id] = question;
        }

        continue;
      }

      questions[`f${index}__role`] = choice(
        `What role does items[${index}] most likely play in this project? Judge from its path.`,
        ROLE_CRITERIA,
      );
      questions[`f${index}__attention`] = noul(
        `Should items[${index}] be investigated now rather than accepted as standing risk?`,
        {
          true: "Investigate now",
          false: "Accept as standing risk for now",
        },
      );
    }

    return questions;
  },

  rankItem(item: DebtItem, answers: Answers, index: number): RankedDebtItem {
    const judged = parseTrio(answers, index);

    const priority = trioPriority({
      weight: item.weight,
      role: judged.role,
      inherent: judged.inherent,
      attention: judged.attention,
    });

    return {
      kind: "debt",
      group: item.group,
      path: item.path,
      name: item.name,
      dimension: item.dimension,
      threshold: item.threshold,
      before: item.before,
      after: item.after,
      components: item.components,
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

function componentsOf(change: JsonObject): DebtComponent[] {
  const components: DebtComponent[] = [];

  for (const value of asArray(change.components)) {
    const component = asObject(value);

    if (component === null) {
      continue;
    }

    const delta = optionalNumber(component, "delta");

    if (delta === null || delta === 0) {
      continue;
    }

    components.push({
      component: optionalString(component, "component") ?? "(unknown)",
      delta: round(delta, 2),
    });
  }

  return components;
}
