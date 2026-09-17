//! Feature execution: turn one leadline report into ranked triage items.
//!
//! Items are prepared once (including their run-normalized deterministic
//! weight, so priorities stay comparable across batches), then split into
//! bounded batches of questions. Every batch is one TypeSafe request.

import { ask } from "./client.ts";
import { sortRanked } from "./compose.ts";
import type { JsonValue } from "./json.ts";
import type { Answers, ClientConfig, Questions, Usage } from "./types.ts";

/** Keep requests well under the model's context budget; raise it when reports
 * are small and request count matters more than margin. */
export const DEFAULT_BATCH_SIZE = 16;

/** Hard ceiling: batch counts are unbounded, so each request stays bounded
 * only when one batch cannot grow without limit. */
export const MAX_BATCH_SIZE = 64;

export type RunOptions = {
  config: ClientConfig;
  batchSize?: number;
  fetchImpl?: typeof fetch;
};

export type BuiltItems<TItem, TContext> = {
  items: TItem[];
  notes: string[];
  context: TContext;
};

export type Feature<TItem, TRanked extends { priority: number }, TContext, TState> = {
  id: string;
  title: string;
  summary: string;
  buildItems: (report: JsonValue) => BuiltItems<TItem, TContext>;
  buildState: (items: TItem[], context: TContext) => TState;
  buildQuestions: (items: TItem[]) => Questions;
  rankItem: (item: TItem, answers: Answers, index: number) => TRanked;
};

export type RunResult<TRanked> = {
  feature: string;
  model: string | null;
  usage: Usage;
  batches: number;
  notes: string[];
  items: TRanked[];
};

export function chunk<T>(items: T[], size: number): T[][] {
  if (!Number.isInteger(size) || size < 1 || size > MAX_BATCH_SIZE) {
    throw new Error(`batch size must be an integer from 1 to ${MAX_BATCH_SIZE}`);
  }

  const batches: T[][] = [];

  for (let index = 0; index < items.length; index += size) {
    batches.push(items.slice(index, index + size));
  }

  return batches;
}

export async function runFeature<
  TItem,
  TRanked extends { priority: number },
  TContext,
  TState,
>(options: {
  feature: Feature<TItem, TRanked, TContext, TState>;
  report: JsonValue;
  run: RunOptions;
}): Promise<RunResult<TRanked>> {
  const { feature, report, run } = options;
  const built = feature.buildItems(report);
  const usage: Usage = { inputTokens: 0, outputTokens: 0 };

  if (built.items.length === 0) {
    return {
      feature: feature.id,
      model: null,
      usage,
      batches: 0,
      notes: built.notes,
      items: [],
    };
  }

  const batches = chunk(built.items, run.batchSize ?? DEFAULT_BATCH_SIZE);
  let model: string | null = null;
  const ranked: TRanked[] = [];

  for (const batch of batches) {
    const response = await ask({
      state: feature.buildState(batch, built.context),
      questions: feature.buildQuestions(batch),
      config: run.config,
      fetchImpl: run.fetchImpl,
    });

    model = response.model ?? model;
    usage.inputTokens += response.usage.inputTokens;
    usage.outputTokens += response.usage.outputTokens;
    batch.forEach((item, index) => {
      ranked.push(feature.rankItem(item, response.answers, index));
    });
  }

  return {
    feature: feature.id,
    model,
    usage,
    batches: batches.length,
    notes: built.notes,
    items: sortRanked(ranked),
  };
}

/** A planned request batch set, also used for `--dry-run` output. */
export type FeaturePlan<TItem, TState> = {
  feature: string;
  items: TItem[];
  notes: string[];
  batches: { state: TState; questions: Questions }[];
};

/** Builds the requests without calling TypeSafe, for `--dry-run` and tests. */
export function planFeature<
  TItem,
  TRanked extends { priority: number },
  TContext,
  TState,
>(options: {
  feature: Feature<TItem, TRanked, TContext, TState>;
  report: JsonValue;
  batchSize: number;
}): FeaturePlan<TItem, TState> {
  const built = options.feature.buildItems(options.report);

  return {
    feature: options.feature.id,
    items: built.items,
    notes: built.notes,
    batches: chunk(built.items, options.batchSize).map((batch) => ({
      state: options.feature.buildState(batch, built.context),
      questions: options.feature.buildQuestions(batch),
    })),
  };
}
