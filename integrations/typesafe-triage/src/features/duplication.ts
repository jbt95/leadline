//! Duplication triage: separate accidental clones from intentional shapes.
//!
//! Report: `leadline duplication [PATH] --json`. Each clone group is judged as
//! intentional (boilerplate, tests, generated) or an extraction candidate;
//! groups whose extraction probability is low land in the `accept` bucket, and
//! the deterministic report still lists every group.

import { choice } from "../client.ts";
import {
  bucketFor,
  choiceOf,
  confidenceOf,
  judgmentEntry,
  normalize,
  probabilityOf,
  round,
} from "../compose.ts";
import {
  asArray,
  asObject,
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

/** Intentional shapes are the common reason a token clone is not actionable;
 * `unclear` exists so the model never has to force a call. */
const INTENT_CRITERIA = {
  extract_candidate: "Accidental duplication: the occurrences should share one implementation",
  intentional_boilerplate:
    "Deliberate repetition that is clearer left duplicated (protocol shapes, records)",
  test_or_fixture: "Test data, fixtures, or examples where duplication is expected",
  generated_or_vendored: "Generated, vendored, or copied third-party code",
  unclear: "The available evidence does not identify the intent",
} satisfies { [intent: string]: string };

/** Duplication-specific policy: extraction is worth acting on only when the
 * model is reasonably sure, and low extraction probability is an explicit
 * accept (the report still lists the group). */
const DUPLICATION_POLICY = { acceptBelow: 0.25, actAt: 0.6 };

export type DuplicationOccurrence = {
  path: string;
  start_line: number | null;
  end_line: number | null;
  duplicated_lines: number | null;
};

export type DuplicationItem = {
  id: string | null;
  language: string | null;
  tokens: number;
  lines: number;
  occurrences: DuplicationOccurrence[];
  weight: number;
};

export type DuplicationContext = { profile: string | null };

export type DuplicationState = {
  report: { kind: "duplication" };
  items: {
    language: string | null;
    tokens: number;
    lines: number;
    occurrences: DuplicationOccurrence[];
  }[];
};

export type RankedDuplicationItem = {
  kind: "duplication";
  id: string | null;
  language: string | null;
  tokens: number;
  lines: number;
  occurrences: DuplicationOccurrence[];
  priority: number;
  bucket: Bucket;
  judgments: Judgments;
};

export const duplicationFeature: Feature<
  DuplicationItem,
  RankedDuplicationItem,
  DuplicationContext,
  DuplicationState
> = {
  id: "duplication",
  title: "Duplication intent triage",
  summary:
    "Classifies clone groups as intentional shapes or extraction candidates; low-extraction groups are accepted but never hidden from the report.",

  buildItems(report: JsonValue) {
    const root = requireObjectValue(report, "duplication report");
    const items: DuplicationItem[] = [];

    for (const value of asArray(root.groups)) {
      const group = asObject(value);

      if (group === null) {
        continue;
      }

      const occurrences = occurrencesOf(group);
      items.push({
        id: optionalString(group, "id"),
        language: optionalString(group, "language"),
        tokens: optionalNumber(group, "token_count") ?? 0,
        lines: occurrences.reduce(
          (max, occurrence) => Math.max(max, occurrence.duplicated_lines ?? 0),
          0,
        ),
        occurrences,
        weight: 0,
      });
    }

    const weights = normalize(items.map((item) => item.tokens));

    const weighted = items.map((item, index) => ({
      ...item,
      weight: round(weights[index]),
    }));

    const notes: string[] = [];

    if (optionalBoolean(root, "complete") === false) {
      notes.push(
        "duplication analysis was incomplete (ceiling reached); groups may be missing",
      );
    }

    if (weighted.length === 0) {
      notes.push("no duplicate groups in the report");
    }

    return {
      items: weighted,
      notes,
      context: { profile: optionalString(root, "profile") },
    };
  },

  buildState(items: DuplicationItem[]): DuplicationState {
    return {
      report: { kind: "duplication" },
      items: items.map((item) => ({
        language: item.language,
        tokens: item.tokens,
        lines: item.lines,
        occurrences: item.occurrences,
      })),
    };
  },

  buildQuestions(items: DuplicationItem[]): Questions {
    return mergeQuestions(
      items.map((_, index) => ({
        [`f${index}__intent`]: choice(
          `Are the occurrences of items[${index}] intentionally shared shapes (boilerplate, generated, tests) or an accidental clone worth extracting?`,
          INTENT_CRITERIA,
        ),
      })),
    );
  },

  rankItem(item: DuplicationItem, answers: Answers, index: number): RankedDuplicationItem {
    const intentAnswer = answers[`f${index}__intent`];
    const extraction = probabilityOf(intentAnswer, "extract_candidate");
    const confidence = confidenceOf(intentAnswer);

    return {
      kind: "duplication",
      id: item.id,
      language: item.language,
      tokens: item.tokens,
      lines: item.lines,
      occurrences: item.occurrences,
      priority: round(item.weight * extraction),
      bucket: bucketFor({
        attention: extraction,
        confidence,
        policy: DUPLICATION_POLICY,
      }),
      judgments: {
        intent: judgmentEntry(intentAnswer, choiceOf(intentAnswer)),
      },
    };
  },
};

function occurrencesOf(group: JsonObject): DuplicationOccurrence[] {
  const occurrences: DuplicationOccurrence[] = [];

  for (const value of asArray(group.occurrences)) {
    const occurrence = asObject(value);

    if (occurrence === null) {
      continue;
    }

    occurrences.push({
      path: optionalString(occurrence, "path") ?? "(unknown path)",
      start_line: optionalNumber(occurrence, "start_line"),
      end_line: optionalNumber(occurrence, "end_line"),
      duplicated_lines: optionalNumber(occurrence, "duplicated_lines"),
    });
  }

  return occurrences;
}
