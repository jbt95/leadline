//! Test helpers: load fixtures and build synthetic TypeSafe answers with known
//! values, so composition can be asserted exactly without calling the API.

import { readFile } from "node:fs/promises";
import { asObject, parseJson, requireObjectValue, type JsonValue } from "../src/json.ts";
import type { Answer, Answers } from "../src/types.ts";

const fixtureDirectory = new URL("./fixtures/", import.meta.url);

export async function fixture(name: string): Promise<JsonValue> {
  return parseJson(await readFile(new URL(name, fixtureDirectory), "utf8"));
}

/** One recorded trio judgment per item. Defaults describe a plausible
 * mid-everything finding; pass overrides per item. */
export type TrioSpec = {
  role?: string;
  roleConfidence?: number;
  inherent?: number | null;
  inherentConfidence?: number;
  attention?: number;
};

export function answerSet(specifications: TrioSpec[]): Answers {
  const answers: Answers = {};
  specifications.forEach((specification, index) => {
    const role = specification.role ?? "adapter_or_integration";
    const roleConfidence = specification.roleConfidence ?? 0.9;
    answers[`f${index}__role`] = {
      type: "choice",
      choice: role,
      probabilities: { [role]: roleConfidence, unclear: 1 - roleConfidence },
      confidence: roleConfidence,
    };
    const inherent = specification.inherent === undefined ? 1.5 : specification.inherent;

    if (inherent !== null) {
      const inherentConfidence = specification.inherentConfidence ?? 0.7;
      answers[`f${index}__inherent`] = {
        type: "score",
        score: inherent,
        probabilities: { 0: 0.25, 1: 0.25, 2: 0.25, 3: 0.25 },
        confidence: inherentConfidence,
      };
    }

    answers[`f${index}__attention`] = {
      type: "noul",
      noul: specification.attention ?? 0.8,
    };
  });

  return answers;
}

export type ExposureSpec = {
  exposure?: string;
  exposureConfidence?: number;
  reachable?: number;
};

/** Answers for the two security questions per finding. */
export function exposureSet(specifications: ExposureSpec[]): Answers {
  const answers: Answers = {};
  specifications.forEach((specification, index) => {
    const exposure = specification.exposure ?? "direct_runtime";
    const exposureConfidence = specification.exposureConfidence ?? 0.85;
    answers[`f${index}__exposure`] = {
      type: "choice",
      choice: exposure,
      probabilities: { [exposure]: exposureConfidence, unclear: 1 - exposureConfidence },
      confidence: exposureConfidence,
    };
    answers[`f${index}__reachable`] = {
      type: "noul",
      noul: specification.reachable ?? 0.8,
    };
  });

  return answers;
}

export type IntentSpec = { intent?: string; confidence?: number };

/** Answers for the single duplication intent question per group. */
export function intentSet(specifications: IntentSpec[]): Answers {
  const answers: Answers = {};
  specifications.forEach((specification, index) => {
    const intent = specification.intent ?? "extract_candidate";
    const confidence = specification.confidence ?? 0.8;
    answers[`f${index}__intent`] = {
      type: "choice",
      choice: intent,
      probabilities: { [intent]: confidence, unclear: 1 - confidence },
      confidence,
    };
  });

  return answers;
}

/** Answers for every question id in a request body, used by pipeline and CLI
 * tests: each id gets a fixed, plausible answer. */
export function stubAnswers(requestBody: JsonValue): Answers {
  const root = asObject(requestBody);

  if (root === null) {
    throw new Error("request body must be an object");
  }

  const questions = requireObjectValue(root.questions, "request.questions");
  const answers: Answers = {};

  for (const id of Object.keys(questions)) {
    answers[id] = stubAnswer(id);
  }

  return answers;
}

/** Serializes an answer map as the wire-format response body TypeSafe returns
 * (`usage` uses snake_case keys on the wire). */
export function wireBody(answers: Answers, options: { model?: string } = {}): string {
  return JSON.stringify({
    model: options.model ?? "jev-test",
    answers,
    usage: { input_tokens: 100, output_tokens: 20 },
  });
}

function stubAnswer(id: string): Answer {
  if (id === "workflow") {
    return {
      type: "choice",
      choice: "changed_review",
      probabilities: { changed_review: 0.7, gate_check: 0.2, not_leadline: 0.1 },
      confidence: 0.7,
    };
  }

  const judgment = judgmentOf(id);

  switch (judgment) {
    case "role":
      return {
        type: "choice",
        choice: "core_domain",
        probabilities: { core_domain: 0.8, unclear: 0.2 },
        confidence: 0.8,
      };
    case "inherent":
      return {
        type: "score",
        score: 1,
        probabilities: { 0: 0.1, 1: 0.6, 2: 0.2, 3: 0.1 },
        confidence: 0.75,
      };
    case "attention":
      return { type: "noul", noul: 0.7 };
    case "exposure":
      return {
        type: "choice",
        choice: "direct_runtime",
        probabilities: { direct_runtime: 0.6, unclear: 0.4 },
        confidence: 0.6,
      };
    case "reachable":
      return { type: "noul", noul: 0.6 };
    case "intent":
      return {
        type: "choice",
        choice: "extract_candidate",
        probabilities: { extract_candidate: 0.8, unclear: 0.2 },
        confidence: 0.8,
      };
    default:
      throw new Error(`unexpected judgment '${judgment}' in '${id}'`);
  }
}

function judgmentOf(id: string): string {
  const match = /^f\d+__(.+)$/.exec(id);

  if (match === null) {
    throw new Error(`unexpected question id '${id}'`);
  }

  return match[1];
}
