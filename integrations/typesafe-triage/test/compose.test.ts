import assert from "node:assert/strict";
import { test } from "node:test";

import {
  bucketFor,
  minConfidence,
  normalize,
  sortRanked,
  weightOf,
} from "../src/compose.ts";
import type { Answer } from "../src/types.ts";

const choiceAnswer: Answer = {
  type: "choice",
  choice: "a",
  probabilities: { a: 1 },
  confidence: 0.9,
};

const hesitatingAnswer: Answer = {
  type: "choice",
  choice: "b",
  probabilities: { b: 0.4, c: 0.6 },
  confidence: 0.4,
};

const noulAnswer: Answer = { type: "noul", noul: 0.5 };

test("normalize spreads values over 0..1 and flattens ties", () => {
  assert.deepEqual(normalize([0, 5, 10]), [0, 0.5, 1]);
  assert.deepEqual(normalize([3, 3]), [0.5, 0.5]);
  assert.deepEqual(normalize([]), []);
});

test("bucketFor prefers accept, then review on low confidence", () => {
  assert.equal(bucketFor({ attention: 0.2, confidence: 0.9 }), "accept");
  assert.equal(bucketFor({ attention: 0.8, confidence: 0.4 }), "review");
  assert.equal(bucketFor({ attention: 0.8, confidence: 0.8 }), "act");
  assert.equal(bucketFor({ attention: null, confidence: null }), "act");
});

test("weightOf falls back for unknown options", () => {
  assert.equal(weightOf({ a: 1, b: 0.5 }, "a"), 1);
  assert.equal(weightOf({ a: 1 }, "missing", 0.25), 0.25);
  assert.equal(weightOf({ a: 1 }, null, 0.5), 0.5);
});

test("minConfidence ignores answers without one and returns null when none have one", () => {
  assert.equal(minConfidence(choiceAnswer, noulAnswer), 0.9);
  assert.equal(minConfidence(choiceAnswer, hesitatingAnswer), 0.4);
  assert.equal(minConfidence(noulAnswer), null);
  assert.equal(minConfidence(undefined), null);
});

test("sortRanked sorts by priority and keeps the original order on ties", () => {
  const ranked = sortRanked([
    { priority: 0.2, name: "a" },
    { priority: 0.9, name: "b" },
    { priority: 0.2, name: "c" },
  ]);

  assert.deepEqual(
    ranked.map((item) => item.name),
    ["b", "a", "c"],
  );
});
