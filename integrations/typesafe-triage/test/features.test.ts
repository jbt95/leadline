import assert from "node:assert/strict";
import { test } from "node:test";

import { changedFeature } from "../src/features/changed.ts";
import { checkFeature } from "../src/features/check.ts";
import { debtFeature } from "../src/features/debt.ts";
import { duplicationFeature } from "../src/features/duplication.ts";
import { WORKFLOWS, planRoute, routeRequest } from "../src/features/route.ts";
import { securityFeature } from "../src/features/security.ts";
import { parseJson } from "../src/json.ts";
import { planFeature, runFeature } from "../src/pipeline.ts";
import type { ClientConfig } from "../src/types.ts";
import {
  answerSet,
  exposureSet,
  fixture,
  intentSet,
  stubAnswers,
  wireBody,
} from "./helpers.ts";

const config: ClientConfig = {
  apiKey: "test",
  baseUrl: "http://127.0.0.1:1",
  model: "jev-test",
  timeoutMs: 1_000,
};

test("changed keeps regressions and new functions, drops unchanged ones", async () => {
  const report = await fixture("changed.json");
  const { items, notes } = changedFeature.buildItems(report);
  assert.deepEqual(
    items.map((item) => item.name),
    ["reconcile", "importRows", "brandNew"],
  );
  assert.equal(items[0].deltas.cognitive, 19);
  assert.equal(items[0].weight, 1);
  assert.equal(items[1].weight, 0);
  assert.equal(items[2].added, true);
  assert.equal(notes.length, 0);
  assert.equal(Object.keys(changedFeature.buildQuestions(items)).length, 9);
});

test("changed ranks critical, accidental regressions above accepted debt", async () => {
  const report = await fixture("changed.json");
  const { items } = changedFeature.buildItems(report);

  const answers = answerSet([
    { role: "critical_path", roleConfidence: 0.95, inherent: 0.5, inherentConfidence: 0.9, attention: 0.9 },
    { role: "tooling_or_script", inherent: 2.5, attention: 0.2 },
    { role: "adapter_or_integration", roleConfidence: 0.5, inherent: 1.5, inherentConfidence: 0.5, attention: 0.7 },
  ]);

  const ranked = items.map((item, index) => changedFeature.rankItem(item, answers, index));
  assert.deepEqual(
    ranked.map((item) => item.bucket),
    ["act", "accept", "review"],
  );
  assert.ok(ranked[0].priority > ranked[2].priority);
  assert.ok(ranked[2].priority > ranked[1].priority);
  assert.equal(ranked[0].line, 120);
});

test("changed keeps a CRAP-only regression on the worklist", async () => {
  const report = await fixture("changed-crap.json");
  const { items } = changedFeature.buildItems(report);
  assert.equal(items.length, 1);
  assert.equal(items[0].deltas.crap, 36);
  assert.equal(items[0].deltaTotal, 36);
  assert.equal(items[0].weight, 0.5);
  const answers = answerSet([{ role: "core_domain", inherent: 1, attention: 0.9 }]);
  const ranked = changedFeature.rankItem(items[0], answers, 0);
  assert.ok(Number.isFinite(ranked.priority));
});

test("changed never lets an improvement cancel a regression", async () => {
  const mixed = {
    functions: [
      {
        path: "mixed.ts",
        name: "mixed",
        before: { metrics: { cognitive: 1, cyclomatic: 5, max_nesting: 1, crap: 1 } },
        after: { metrics: { cognitive: 3, cyclomatic: 3, max_nesting: 1, crap: 1 } },
      },
    ],
  };

  const { items } = changedFeature.buildItems(mixed);
  assert.equal(items.length, 1);
  assert.equal(items[0].deltaTotal, 2);

  const multi = {
    functions: [
      {
        path: "struct.ts",
        name: "structural",
        before: { metrics: { cognitive: 1, cyclomatic: 1, max_nesting: 1, crap: 1 } },
        after: { metrics: { cognitive: 2, cyclomatic: 1, max_nesting: 1, crap: 1 } },
      },
      {
        path: "crap.ts",
        name: "coverage",
        before: { metrics: { cognitive: 1, cyclomatic: 1, max_nesting: 1, crap: 1 } },
        after: { metrics: { cognitive: 1, cyclomatic: 1, max_nesting: 1, crap: 10 } },
      },
    ],
  };

  const both = changedFeature.buildItems(multi);
  assert.equal(both.items.length, 2);
  assert.equal(both.items[1].weight, 1);
});

test("check collects parse errors as must-fix notes", async () => {
  const report = await fixture("check.json");
  const { items, notes } = checkFeature.buildItems(report);
  assert.deepEqual(
    items.map((item) => item.name),
    ["dispatch", "retry"],
  );
  assert.equal(items[0].weight, 1);
  assert.equal(items[1].weight, 0);
  assert.match(notes[0], /must fix first: 1 parse error/);
  assert.equal(Object.keys(checkFeature.buildQuestions(items)).length, 6);
});

test("security normalizes both scanner report shapes", async () => {
  const securityReport = await fixture("security.json");
  const fromSarif = securityFeature.buildItems(securityReport);
  assert.deepEqual(
    fromSarif.items.map((item) => item.scanner),
    ["security", "security"],
  );
  assert.deepEqual(
    fromSarif.items.map((item) => item.weight),
    [0.8, 0.35],
  );

  const vulnerabilityReport = await fixture("vulnerabilities.json");
  const fromOsv = securityFeature.buildItems(vulnerabilityReport);
  assert.equal(fromOsv.items.length, 1);
  assert.equal(fromOsv.items[0].scanner, "vulnerabilities");
  assert.equal(fromOsv.items[0].weight, 1);
  assert.match(fromOsv.items[0].label, /lodash GHSA-/);
  assert.deepEqual(fromOsv.items[0].detail.changed_imports, ["src/report.ts"]);

  const unknown = securityFeature.buildItems({
    findings: [{ something: true }],
  });

  // Unrecognized rows are preserved, never suppressed.
  assert.equal(unknown.items.length, 1);
  assert.equal(unknown.items[0].severity, "unknown");
  assert.equal(unknown.items[0].label, "(unrecognized finding)");
});

test("security ranks exposed, reachable findings above dev-only ones", async () => {
  const report = await fixture("security.json");
  const { items } = securityFeature.buildItems(report);

  const answers = exposureSet([
    { exposure: "direct_runtime", exposureConfidence: 0.9, reachable: 0.9 },
    { exposure: "dev_or_build", exposureConfidence: 0.4, reachable: 0.2 },
  ]);

  const ranked = items.map((item, index) => securityFeature.rankItem(item, answers, index));
  assert.equal(ranked[0].priority, 0.736);
  assert.equal(ranked[1].priority, 0.044);
  assert.deepEqual(
    ranked.map((item) => item.bucket),
    ["act", "review"],
  );
  // Findings are never suppressed: no `accept` bucket exists here.
  assert.ok(ranked.every((item) => item.bucket !== "accept"));
});

test("debt triages new function debt and increased risk only", async () => {
  const report = await fixture("debt.json");
  const { items, notes } = debtFeature.buildItems(report);
  assert.equal(items.length, 2);
  assert.deepEqual(
    items.map((item) => item.group),
    ["function", "risk"],
  );
  assert.match(notes[0], /1 new, 2 resolved, 1 risk increased/);
  assert.equal(Object.keys(debtFeature.buildQuestions(items)).length, 5);

  const answers = answerSet([
    { role: "critical_path", inherent: 1, attention: 0.9 },
    { role: "core_domain", inherent: null, attention: 0.6 },
  ]);

  const ranked = items.map((item, index) => debtFeature.rankItem(item, answers, index));
  assert.deepEqual(
    ranked.map((item) => item.bucket),
    ["act", "act"],
  );
  assert.ok(ranked[0].priority > ranked[1].priority);
});

test("duplication accepts intentional shapes and ranks extraction candidates", async () => {
  const report = await fixture("duplication.json");
  const { items } = duplicationFeature.buildItems(report);
  assert.equal(items.length, 2);
  assert.deepEqual(
    items.map((item) => item.tokens),
    [318, 150],
  );

  const answers = intentSet([
    { intent: "extract_candidate", confidence: 0.8 },
    { intent: "intentional_boilerplate", confidence: 0.9 },
  ]);

  const ranked = items.map((item, index) => duplicationFeature.rankItem(item, answers, index));
  assert.equal(ranked[0].bucket, "act");
  assert.equal(ranked[0].priority, 0.8);
  assert.equal(ranked[1].bucket, "accept");
});

test("route plans one request and maps the answer to a command", async () => {
  const plan = planRoute({ request: "review my change" });
  assert.equal(plan.batches.length, 1);
  const workflowQuestion = plan.batches[0].questions.workflow;
  assert.equal(workflowQuestion?.type, "choice");

  if (workflowQuestion?.type === "choice") {
    assert.equal(Object.keys(workflowQuestion.criteria).length, 13);
  }

  assert.equal(
    plan.batches[0].state.workflows.not_leadline,
    "The request is not a leadline analysis task",
  );

  const fetchImpl: typeof fetch = async () =>
    new Response(
      JSON.stringify({
        model: "jev-test",
        answers: {
          workflow: {
            type: "choice",
            choice: "changed_review",
            probabilities: { changed_review: 0.7, gate_check: 0.2, not_leadline: 0.1 },
            confidence: 0.7,
          },
        },
        usage: { input_tokens: 50, output_tokens: 5 },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );

  const routed = await routeRequest({ request: "review my change", config, fetchImpl });
  assert.equal(routed.workflow, "changed_review");
  assert.equal(routed.action, "run");
  assert.equal(routed.command, "leadline changed --base <REV> --json");
  assert.equal(routed.alternatives[0].workflow, "changed_review");
});

test("route sends low-confidence answers to the user", async () => {
  const fetchImpl: typeof fetch = async () =>
    new Response(
      JSON.stringify({
        answers: {
          workflow: {
            type: "choice",
            choice: "hotspots",
            probabilities: { hotspots: 0.35, repo_overview: 0.3, changed_review: 0.35 },
            confidence: 0.3,
          },
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );

  const routed = await routeRequest({ request: "hmm", config, fetchImpl });
  assert.equal(routed.action, "ask_user");
});

test("route asks the user when the workflow is not in the catalog", async () => {
  const fetchImpl: typeof fetch = async () =>
    new Response(
      JSON.stringify({
        answers: {
          workflow: {
            type: "choice",
            choice: "frobnicate",
            probabilities: { frobnicate: 0.9 },
            confidence: 0.9,
          },
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );

  const routed = await routeRequest({ request: "??", config, fetchImpl });
  assert.equal(routed.action, "ask_user");
  assert.equal(routed.command, null);
});

test("every route command is one runnable leadline invocation", () => {
  for (const [id, workflow] of Object.entries(WORKFLOWS)) {
    if (workflow.command === null) {
      assert.equal(id, "not_leadline");
      continue;
    }

    assert.ok(workflow.command.startsWith("leadline "), `${id} must start with leadline`);
    assert.ok(!workflow.command.includes("|"), `${id} must not pipe two commands`);
  }

  assert.match(WORKFLOWS.debt_review.command ?? "", /--fail-on-regression/);
  assert.match(WORKFLOWS.duplication_triage.command ?? "", /--base <REV>/);
});

test("route rejects oversized request text", async () => {
  await assert.rejects(
    routeRequest({ request: "x".repeat(8_001), config }),
    /exceeds 8000/,
  );
});

test("runFeature batches requests and aggregates usage", async () => {
  const report = await fixture("changed.json");
  let calls = 0;

  const fetchImpl: typeof fetch = async (input, init) => {
    calls += 1;

    return new Response(wireBody(stubAnswers(parseJson(String(init?.body)))), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const result = await runFeature({
    feature: changedFeature,
    report,
    run: { config, batchSize: 2, fetchImpl },
  });

  assert.equal(calls, 2);
  assert.equal(result.batches, 2);
  assert.equal(result.items.length, 3);
  assert.equal(result.usage.inputTokens, 200);
  assert.equal(result.model, "jev-test");
  const plan = planFeature({ feature: changedFeature, report, batchSize: 2 });
  assert.equal(plan.batches.length, 2);
});

test("a missing answer fails the run instead of ranking on defaults", async () => {
  const report = await fixture("changed.json");

  const fetchImpl: typeof fetch = async (input, init) => {
    const answers = stubAnswers(parseJson(String(init?.body)));
    // Drop one judgment entirely, as a truncated response would.
    delete answers["f0__inherent"];

    return new Response(wireBody(answers), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  await assert.rejects(
    runFeature({ feature: changedFeature, report, run: { config, fetchImpl } }),
    /TypeSafe answer 'f0__inherent'/,
  );
});
