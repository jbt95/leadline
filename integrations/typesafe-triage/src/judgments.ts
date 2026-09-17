//! The shared three-question judgment used on code items: what role the item
//! plays, how inherent its complexity is, and whether it deserves attention.
//!
//! Question ids are `f<index>__<judgment>` and indexes are batch-local. The
//! instructions reference `items[<index>]`, so each question is self-contained
//! even though several share one state.

import { TypeSafeError, choice, noul, score } from "./client.ts";
import {
  choiceOf,
  judgmentEntry,
  minConfidence,
  noulOf,
  scoreOf,
  weightOf,
} from "./compose.ts";
import type { Answers, Judgments, Questions } from "./types.ts";

/** Deterministic role weights: a regression on a critical path outweighs the
 * same delta in tooling. Recognize but do not invent a role: `unclear` sits in
 * the middle. */
export const ROLE_WEIGHTS = {
  critical_path: 1,
  core_domain: 0.9,
  adapter_or_integration: 0.6,
  tooling_or_script: 0.4,
  unclear: 0.5,
} satisfies { [role: string]: number };

export const ROLE_CRITERIA = {
  critical_path: "Runtime code on a primary user-facing or business-critical path",
  core_domain: "The project's own core logic: analyzer, engine, parser, metric computation",
  adapter_or_integration:
    "Glue between systems: I/O, protocol handling, formatting, harness adapters",
  tooling_or_script: "Development tooling, generators, or test-only utilities",
  unclear: "The available evidence does not identify a role",
} satisfies { [role: string]: string };

export const INHERENT_LEVELS: string[] = [
  "Almost entirely accidental: a rewrite would stay simple",
  "Mostly accidental: clear, low-risk simplifications exist",
  "Mixed: essential branching and accrued complexity are both present",
  "Mostly inherent: the problem itself requires most of this branching",
];

export const ATTENTION_CRITERIA = {
  true: "Flag for attention now: refactor or split it",
  false: "Accept as standing debt for now",
};

/** Code judgments follow the shared confidence default: act at 0.6 confidence,
 * treat attention below 0.4 as standing debt. Raise `actAt` when acting on a
 * wrong code judgment is expensive. */
export const TRIO_POLICY = { acceptBelow: 0.4, actAt: 0.6 };

export function codeItemQuestions(
  index: number,
  options: { includeInherent?: boolean } = {},
): Questions {
  const reference = `items[${index}]`;

  const questions: Questions = {
    [`f${index}__role`]: choice(
      `What role does ${reference} most likely play in this project? Judge from its path, name, and metrics.`,
      ROLE_CRITERIA,
    ),
    [`f${index}__attention`]: noul(
      `Should ${reference} be flagged for attention now (refactor or split) rather than accepted as standing debt?`,
      ATTENTION_CRITERIA,
    ),
  };

  if (options.includeInherent !== false) {
    questions[`f${index}__inherent`] = score(
      `How much of ${reference}'s complexity is inherent to the problem it solves versus accidental and reducible by refactoring?`,
      INHERENT_LEVELS,
    );
  }

  return questions;
}

/** Flattens per-item question sets into one request map. */
export function mergeQuestions(sources: Questions[]): Questions {
  const questions: Questions = {};

  for (const source of sources) {
    for (const [id, question] of Object.entries(source)) {
      questions[id] = question;
    }
  }

  return questions;
}

export type TrioJudgment = {
  role: string | null;
  inherent: number | null;
  attention: number | null;
  confidence: number | null;
  judgments: Judgments;
};

export function parseTrio(answers: Answers, index: number): TrioJudgment {
  const roleAnswer = answers[`f${index}__role`];
  const inherentAnswer = answers[`f${index}__inherent`];
  const attentionAnswer = answers[`f${index}__attention`];
  const roleValue = choiceOf(roleAnswer);
  const inherentValue = scoreOf(inherentAnswer);
  const attentionValue = noulOf(attentionAnswer);

  if (inherentValue !== null && !(inherentValue >= 0 && inherentValue <= 3)) {
    throw new TypeSafeError("TypeSafe inherent score is out of range");
  }

  const judgments: Judgments = { role: judgmentEntry(roleAnswer, roleValue) };

  if (inherentAnswer !== undefined) {
    judgments.inherent = judgmentEntry(inherentAnswer, inherentValue);
  }

  if (attentionValue !== null) {
    judgments.attention = attentionValue;
  }

  return {
    role: roleValue,
    inherent: inherentValue,
    attention: attentionValue,
    confidence: minConfidence(roleAnswer, inherentAnswer),
    judgments,
  };
}

/** Deterministic composition: the run-normalized delta weight times the role
 * weight times a floored inherent-complexity factor times the attention
 * probability. The floor keeps mostly-inherent complexity in the list at low
 * priority instead of dropping it: the factor ranges from 0.2 to 1. */
export function trioPriority(options: {
  weight: number;
  role: string | null;
  inherent: number | null;
  attention: number | null;
}): number {
  const roleWeight = weightOf(ROLE_WEIGHTS, options.role, 0.5);
  const inherentFactor = 0.2 + 0.8 * (1 - (options.inherent ?? 1.5) / 3);
  const attentionWeight = options.attention ?? 1;

  return options.weight * roleWeight * inherentFactor * attentionWeight;
}
