//! Domain types shared by the client, the pipeline, and the six features.

import type { NumberMap } from "./json.ts";

export type Bucket = "act" | "review" | "accept";

export type ChoiceQuestion = {
  type: "choice";
  instructions: string;
  criteria: { [option: string]: string };
};

export type ScoreQuestion = {
  type: "score";
  instructions: string;
  criteria: string[];
};

export type NoulQuestion = {
  type: "noul";
  instructions: string;
  criteria?: { true: string; false: string };
};

export type Question = ChoiceQuestion | ScoreQuestion | NoulQuestion;

export type Questions = { [id: string]: Question };

export type ChoiceAnswer = {
  type: "choice";
  choice: string;
  probabilities: NumberMap;
  confidence: number;
};

export type ScoreAnswer = {
  type: "score";
  score: number;
  probabilities: NumberMap;
  confidence: number;
};

export type NoulAnswer = { type: "noul"; noul: number };

export type Answer = ChoiceAnswer | ScoreAnswer | NoulAnswer;

export type Answers = { [id: string]: Answer };

export type Usage = { inputTokens: number; outputTokens: number };

export type ApiResponse = { model: string | null; answers: Answers; usage: Usage };

export type ClientConfig = {
  apiKey: string;
  baseUrl: string;
  model: string;
  timeoutMs: number;
};

/** One judgment a feature reports: a selected value (when the primitive has
 * one) and the model's confidence (absent for Noul). */
export type JudgmentEntry = { value: string | number | null; confidence: number | null };

/** The union of judgments any feature can report; each feature sets only the
 * entries it asks for. */
export type Judgments = {
  role?: JudgmentEntry;
  inherent?: JudgmentEntry;
  attention?: number;
  exposure?: JudgmentEntry;
  reachable?: number;
  intent?: JudgmentEntry;
};

export type AskRequest<TState> = {
  state: TState;
  questions: Questions;
  config: ClientConfig;
  fetchImpl?: typeof fetch;
};

export type FeatureSpec = {
  id: string;
  title: string;
  summary: string;
  input: "report" | "text";
};
