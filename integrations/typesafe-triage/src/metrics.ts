//! Shared metric decoding for code-item features (`changed`, `check`).
//! One place owns the metric key set, so a leadline metric rename is a
//! single-file change and every feature reports the same columns.

import { optionalNumber, type JsonObject } from "./json.ts";

export type MetricSet = {
  cognitive: number | null;
  cyclomatic: number | null;
  max_nesting: number | null;
  crap: number | null;
};

export function metricSet(metrics: JsonObject | null): MetricSet {
  if (metrics === null) {
    return { cognitive: null, cyclomatic: null, max_nesting: null, crap: null };
  }

  return {
    cognitive: optionalNumber(metrics, "cognitive"),
    cyclomatic: optionalNumber(metrics, "cyclomatic"),
    max_nesting: optionalNumber(metrics, "max_nesting"),
    crap: optionalNumber(metrics, "crap"),
  };
}
