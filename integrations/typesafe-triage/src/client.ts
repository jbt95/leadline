//! Dependency-free client for the TypeSafe evaluation endpoint.
//!
//! This is the only module that talks to the network. Requests are bounded by
//! a timeout, responses are decoded at the boundary, and remote bodies are
//! never echoed: error messages carry the HTTP status only, so a gateway
//! cannot reflect request state (or the API key) into logs.

import {
  asObject,
  optionalNumber,
  optionalString,
  parseJson,
  requireNumber,
  requireNumberMap,
  requireObjectValue,
  requireString,
  type JsonObject,
  type JsonValue,
} from "./json.ts";
import type {
  Answer,
  ApiResponse,
  AskRequest,
  Answers,
  ChoiceQuestion,
  ClientConfig,
  NoulQuestion,
  Questions,
  ScoreQuestion,
  Usage,
} from "./types.ts";

export class TriageError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TriageError";
  }
}

export class TypeSafeError extends TriageError {
  status: number | null;

  constructor(message: string, status: number | null = null) {
    super(message);
    this.name = "TypeSafeError";
    this.status = status;
  }
}

export const DEFAULT_BASE_URL = "https://api.typesafe.ai";

export const DEFAULT_MODEL = "jev-latest";

export const DEFAULT_TIMEOUT_MS = 10_000;

/** Outbound requests are bounded as well as batched: no single request may
 * exceed this body size, however large one report row grows. */
export const MAX_REQUEST_BODY_BYTES = 256_000;

/** Minimal question builders mirroring the three TypeSafe primitives. */
export const choice = (
  instructions: string,
  criteria: { [option: string]: string },
): ChoiceQuestion => ({
  type: "choice",
  instructions,
  criteria,
});

export const score = (instructions: string, levels: string[]): ScoreQuestion => ({
  type: "score",
  instructions,
  criteria: levels,
});

export const noul = (
  instructions: string,
  criteria: { true: string; false: string } | null = null,
): NoulQuestion => {
  if (criteria === null) {
    return { type: "noul", instructions };
  }

  return { type: "noul", instructions, criteria };
};

export function configFromEnv(env: { [name: string]: string | undefined }): ClientConfig {
  const apiKey = (env.TYPESAFE_API_KEY ?? "").trim();

  if (apiKey === "") {
    throw new TypeSafeError(
      "TYPESAFE_API_KEY is not set; export it or use --dry-run to inspect the request",
    );
  }

  const timeout = Number(env.TYPESAFE_TIMEOUT_MS ?? DEFAULT_TIMEOUT_MS);

  return {
    apiKey,
    baseUrl: (env.TYPESAFE_BASE_URL ?? "").trim() || DEFAULT_BASE_URL,
    model: (env.TYPESAFE_DEFAULT_MODEL ?? "").trim() || DEFAULT_MODEL,
    timeoutMs: Number.isFinite(timeout) && timeout > 0 ? timeout : DEFAULT_TIMEOUT_MS,
  };
}

/** One bounded request. Answers come back keyed by question id. */
export async function ask<TState>(request: AskRequest<TState>): Promise<ApiResponse> {
  const { state, questions, config } = request;
  const fetchImpl = request.fetchImpl ?? fetch;
  const url = `${config.baseUrl.replace(/\/+$/, "")}/v1/systemone`;
  const controller = new AbortController();
  let timedOut = false;

  const timer = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, config.timeoutMs);

  try {
    const body = JSON.stringify({ state, model: config.model, questions });

    if (body.length > MAX_REQUEST_BODY_BYTES) {
      throw new TypeSafeError("TypeSafe request exceeds the size limit");
    }

    const response = await fetchImpl(url, {
      method: "POST",
      headers: {
        authorization: `Bearer ${config.apiKey}`,
        "content-type": "application/json",
      },
      body,
      signal: controller.signal,
    });

    if (!response.ok) {
      // Status only: a gateway body can echo request state, so remote text
      // never reaches logs.
      throw new TypeSafeError(`TypeSafe returned ${response.status}`, response.status);
    }

    // Reading the body stays inside the timeout window: a gateway that sends
    // headers and then stalls cannot hang the command.
    const text = await response.text();

    let payload: JsonValue;

    try {
      payload = parseJson(text);
    } catch {
      // Body-free by design: a V8 JSON parse error embeds a snippet of input.
      throw new TypeSafeError("TypeSafe response is not valid JSON");
    }

    return decodeResponse(payload, questions);
  } catch (error) {
    if (error instanceof TypeSafeError) {
      throw error;
    }

    // An abort during the handshake surfaces as AbortError; an abort while the
    // body streams surfaces as a terminated-body TypeError. The flag covers
    // both.
    if (timedOut || (error instanceof Error && error.name === "AbortError")) {
      throw new TypeSafeError(`TypeSafe request timed out after ${config.timeoutMs} ms`);
    }

    throw new TypeSafeError(
      `TypeSafe request failed: ${error instanceof Error ? error.message : "network error"}`,
    );
  } finally {
    clearTimeout(timer);
  }
}

function decodeResponse(payload: JsonValue, questions: Questions): ApiResponse {
  const root = requireObjectValue(payload, "response");
  const source = requireObjectValue(root.answers, "response.answers");
  const answers: Answers = {};

  // Only requested ids are decoded: extra body keys are ignored, every id in
  // an error message is ours, and a missing or mistyped answer fails the
  // request instead of ranking on a null default.
  for (const [id, question] of Object.entries(questions)) {
    answers[id] = decodeAnswer(source[id], id, question.type);
  }

  return {
    model: optionalString(root, "model"),
    answers,
    usage: decodeUsage(asObject(root.usage)),
  };
}

function decodeAnswer(value: JsonValue | undefined, id: string, expected: string): Answer {
  const context = `TypeSafe answer '${id}'`;
  const answer = requireObjectValue(value, context);
  const type = requireString(answer, "type", context);

  if (type !== expected) {
    throw new TypeSafeError(`${context} has the wrong type`);
  }

  if (type === "choice") {
    const choiceValue = requireString(answer, "choice", context);

    if (choiceValue === "") {
      throw new TypeSafeError(`${context} has an empty choice`);
    }

    return {
      type: "choice",
      choice: choiceValue,
      probabilities: requireNumberMap(answer, "probabilities", context),
      confidence: requireConfidence(answer, context),
    };
  }

  if (type === "score") {
    const scoreValue = requireNumber(answer, "score", context);

    if (scoreValue < 0) {
      throw new TypeSafeError(`${context} has an out-of-range score`);
    }

    return {
      type: "score",
      score: scoreValue,
      probabilities: requireNumberMap(answer, "probabilities", context),
      confidence: requireConfidence(answer, context),
    };
  }

  const noulValue = requireNumber(answer, "noul", context);

  if (noulValue < 0 || noulValue > 1) {
    throw new TypeSafeError(`${context} has an out-of-range probability`);
  }

  return { type: "noul", noul: noulValue };
}

function requireConfidence(answer: JsonObject, context: string): number {
  const confidence = requireNumber(answer, "confidence", context);

  if (confidence < 0 || confidence > 1) {
    throw new TypeSafeError(`${context} has an out-of-range confidence`);
  }

  return confidence;
}

function decodeUsage(usage: JsonObject | null): Usage {
  return {
    inputTokens: usage === null ? 0 : (optionalNumber(usage, "input_tokens") ?? 0),
    outputTokens: usage === null ? 0 : (optionalNumber(usage, "output_tokens") ?? 0),
  };
}
