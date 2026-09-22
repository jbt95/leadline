import { test } from "node:test";
import assert from "node:assert/strict";
import { quantile, mergeHistogram, slugFromHash } from "../src/lib/pure.mjs";

// Explicit expected values, per CONTRIBUTING.md: every metric-facing helper
// gets a small fixture with the numbers written out.

// ── quantile ─────────────────────────────────────────────────────────────
// Three buckets: (0,10] holds 2, (10,20] holds 4, (20,30] holds 4 of 10.
const bounds = [10, 20, 30];
const cumulative = [2, 6, 10];

test("quantile interpolates inside a bucket", () => {
  // rank 5 sits in bucket 1, which spans 2..6 over 10..20: 10 + (3/4)*10.
  assert.equal(quantile(bounds, cumulative, 10, 0.5), 17.5);
});

test("quantile returns the first bucket's upper edge at its boundary", () => {
  // rank 2 is exactly the first cumulative count: lower edge is 0, so the
  // interpolation lands on the upper edge.
  assert.equal(quantile(bounds, cumulative, 10, 0.2), 10);
});

test("quantile returns the last bucket's upper edge at the top", () => {
  assert.equal(quantile(bounds, cumulative, 10, 1), 30);
});

test("quantile clamps to the last finite edge above the final bucket", () => {
  const open = [10, 20, Infinity];
  const counts = [2, 6, 10];
  // rank 9.5 lands in the +Inf bucket; there is no width to interpolate.
  assert.equal(quantile(open, counts, 10, 0.95), 20);
  // A single open bucket has nothing finite to fall back to.
  assert.equal(quantile([Infinity], [1], 1, 0.5), null);
});

test("quantile returns null with nothing to interpolate", () => {
  assert.equal(quantile(bounds, cumulative, 0, 0.5), null);
  assert.equal(quantile([], [], 10, 0.5), null);
  assert.equal(quantile(bounds, [2, 6], 10, 0.5), null);
});

// ── mergeHistogram ───────────────────────────────────────────────────────

test("mergeHistogram sums rows that share bounds", () => {
  const merged = mergeHistogram([
    { bounds: [1, 2], buckets: [1, 2], count: 3, sum: 5 },
    { bounds: [1, 2], buckets: [3, 4], count: 7, sum: 9 },
  ]);
  assert.deepEqual(merged, { count: 10, sum: 14, buckets: [4, 6], bounds: [1, 2] });
});

test("mergeHistogram drops rows whose buckets disagree with their bounds", () => {
  const merged = mergeHistogram([
    { bounds: [1, 2], buckets: [1], count: 3, sum: 5 },
    { bounds: [1, 2], buckets: [3, 4], count: 7, sum: 9 },
  ]);
  assert.deepEqual(merged, { count: 7, sum: 9, buckets: [3, 4], bounds: [1, 2] });
});

test("mergeHistogram refuses families with different bounds", () => {
  assert.equal(
    mergeHistogram([
      { bounds: [1, 2], buckets: [1, 2], count: 3, sum: 5 },
      { bounds: [1, 3], buckets: [3, 4], count: 7, sum: 9 },
    ]),
    null,
  );
  assert.equal(mergeHistogram([]), null);
});

// ── slugFromHash ─────────────────────────────────────────────────────────

const slugs = ["overview", "hotspots", "telemetry"];

test("slugFromHash reads a #/slug route", () => {
  assert.equal(slugFromHash("#/hotspots", slugs, "overview"), "hotspots");
  assert.equal(slugFromHash("#/hotspots?range=1h", slugs, "overview"), "hotspots");
  assert.equal(slugFromHash("#/hotspots/nested", slugs, "overview"), "hotspots");
});

test("slugFromHash falls back for anything that is not a route", () => {
  assert.equal(slugFromHash("#hotspots", slugs, "overview"), "overview");
  assert.equal(slugFromHash("#/nope", slugs, "overview"), "overview");
  assert.equal(slugFromHash("", slugs, "overview"), "overview");
});
