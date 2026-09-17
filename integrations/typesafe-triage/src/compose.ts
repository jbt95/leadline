//! Deterministic composition shared by every triage feature.
//!
//! The model supplies judgments; this module owns every weight, normalization,
//! and confidence threshold. Change policy here, not in the questions, and the
//! same recorded answers re-rank without another inference call.

import type { Answer, Bucket, JudgmentEntry } from "./types.ts";

export const choiceOf = (answer: Answer | undefined): string | null =>
  answer?.type === "choice" ? answer.choice : null;

export const scoreOf = (answer: Answer | undefined): number | null =>
  answer?.type === "score" ? answer.score : null;

export const noulOf = (answer: Answer | undefined): number | null =>
  answer?.type === "noul" ? answer.noul : null;

export const confidenceOf = (answer: Answer | undefined): number | null =>
  answer === undefined || answer.type === "noul" ? null : answer.confidence;

export const probabilityOf = (answer: Answer | undefined, option: string): number =>
  answer === undefined || answer.type === "noul" ? 0 : (answer.probabilities[option] ?? 0);

/** The weakest confidence among the answers that carry one; null when none do
 * (a Noul-only judgment has no separate confidence). */
export function minConfidence(...answers: (Answer | undefined)[]): number | null {
  const values = answers
    .map(confidenceOf)
    .filter((value): value is number => value !== null);

  return values.length > 0 ? Math.min(...values) : null;
}

export function judgmentEntry(
  answer: Answer | undefined,
  value: string | number | null,
): JudgmentEntry {
  return { value, confidence: confidenceOf(answer) };
}

/** Confidence policy in one place: below `acceptBelow` the finding is treated
 * as standing debt, below `actAt` it goes to human review, otherwise it can be
 * acted on directly. */
export type ConfidencePolicy = { acceptBelow: number; actAt: number };

export const DEFAULT_POLICY: ConfidencePolicy = { acceptBelow: 0.4, actAt: 0.6 };

export function bucketFor(options: {
  attention: number | null;
  confidence: number | null;
  policy?: ConfidencePolicy;
}): Bucket {
  const policy = options.policy ?? DEFAULT_POLICY;

  if (options.attention !== null && options.attention < policy.acceptBelow) {
    return "accept";
  }

  if (options.confidence !== null && options.confidence < policy.actAt) {
    return "review";
  }

  return "act";
}

/** Min-max normalization over one run's values, so priorities are comparable
 * within a worklist. Equal values land at 0.5 (no signal either way). */
export function normalize(values: number[]): number[] {
  const finite = values.filter((value) => Number.isFinite(value));

  if (finite.length === 0) {
    return values.map(() => 0.5);
  }

  const min = Math.min(...finite);
  const max = Math.max(...finite);

  if (max === min) {
    return values.map(() => 0.5);
  }

  return values.map((value) => (Number.isFinite(value) ? (value - min) / (max - min) : 0.5));
}

/** Weight a Choice's option through a table. Unknown options fall back to the
 * feature's fallback so a new or misspelled option cannot inflate a priority. */
export function weightOf(
  weights: { [option: string]: number },
  option: string | null,
  fallback = 0,
): number {
  if (option === null) {
    return fallback;
  }

  return Object.hasOwn(weights, option) ? weights[option] : fallback;
}

export function round(value: number, digits = 3): number {
  return Number(value.toFixed(digits));
}

/** Sorts descending by priority; ties keep the report's original order. */
export function sortRanked<T extends { priority: number }>(items: T[]): T[] {
  return items
    .map((item, index) => ({ item, index }))
    .sort((left, right) => right.item.priority - left.item.priority || left.index - right.index)
    .map((entry) => entry.item);
}
