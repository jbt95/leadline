//! Feature registry: the six triage capabilities and the typed dispatch the
//! CLI uses. One runner map holds each feature's spec plus its plan/run
//! closures, so adding a feature edits this file once.

import { checkFeature, type RankedCheckItem } from "./features/check.ts";
import { changedFeature, type RankedChangedItem } from "./features/changed.ts";
import { debtFeature, type RankedDebtItem } from "./features/debt.ts";
import {
  duplicationFeature,
  type RankedDuplicationItem,
} from "./features/duplication.ts";
import { routeSpec } from "./features/route.ts";
import { securityFeature, type RankedSecurityItem } from "./features/security.ts";
import type { JsonValue } from "./json.ts";
import {
  planFeature,
  runFeature,
  type Feature,
  type RunOptions,
  type RunResult,
} from "./pipeline.ts";
import type { FeatureSpec } from "./types.ts";

export type RankedItem =
  | RankedChangedItem
  | RankedCheckItem
  | RankedSecurityItem
  | RankedDebtItem
  | RankedDuplicationItem;

type ReportRunner = {
  spec: FeatureSpec;
  plan: (report: JsonValue, batchSize: number) => string;
  run: (report: JsonValue, run: RunOptions) => Promise<RunResult<RankedItem>>;
};

function runnerFor<
  TItem,
  TRanked extends RankedItem,
  TContext,
  TState,
>(feature: Feature<TItem, TRanked, TContext, TState>): ReportRunner {
  return {
    spec: {
      id: feature.id,
      title: feature.title,
      summary: feature.summary,
      input: "report",
    },
    plan: (report: JsonValue, batchSize: number) =>
      JSON.stringify({ dryRun: true, ...planFeature({ feature, report, batchSize }) }, null, 2),
    run: (report: JsonValue, run: RunOptions) => runFeature({ feature, report, run }),
  };
}

const REPORT_RUNNERS = {
  changed: runnerFor(changedFeature),
  check: runnerFor(checkFeature),
  security: runnerFor(securityFeature),
  debt: runnerFor(debtFeature),
  duplication: runnerFor(duplicationFeature),
} satisfies { [id: string]: ReportRunner };

export type ReportFeatureId = keyof typeof REPORT_RUNNERS;

export function featureSpecs(): FeatureSpec[] {
  return [...Object.values(REPORT_RUNNERS).map((runner) => runner.spec), routeSpec];
}

export function isReportFeatureId(id: string): id is ReportFeatureId {
  return Object.hasOwn(REPORT_RUNNERS, id);
}

export function planReportFeature(
  id: ReportFeatureId,
  report: JsonValue,
  batchSize: number,
): string {
  return REPORT_RUNNERS[id].plan(report, batchSize);
}

export async function runReportFeature(
  id: ReportFeatureId,
  report: JsonValue,
  run: RunOptions,
): Promise<RunResult<RankedItem>> {
  return REPORT_RUNNERS[id].run(report, run);
}
