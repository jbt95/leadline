import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  asArray,
  asObject,
  optionalBoolean,
  optionalNumber,
  optionalString,
  parseJson,
  requireObjectValue,
  type JsonObject,
} from "../src/json.ts";
import { stubAnswers, wireBody } from "./helpers.ts";

const cli = fileURLToPath(new URL("../bin/triage.ts", import.meta.url));

const changedFixture = fileURLToPath(new URL("./fixtures/changed.json", import.meta.url));

const checkFixture = fileURLToPath(new URL("./fixtures/check.json", import.meta.url));

const securityFixture = fileURLToPath(new URL("./fixtures/security.json", import.meta.url));

const debtFixture = fileURLToPath(new URL("./fixtures/debt.json", import.meta.url));

const duplicationFixture = fileURLToPath(
  new URL("./fixtures/duplication.json", import.meta.url),
);

const servers: Server[] = [];

after(() => {
  for (const server of servers) {
    try {
      server.close();
    } catch {
      // already closed
    }
  }
});

async function stubServer(
  handler: (request: IncomingMessage, response: ServerResponse) => void,
): Promise<string> {
  const server = createServer(handler);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", () => resolve()));

  return `http://127.0.0.1:${portOf(server)}`;
}

function portOf(server: Server): number {
  const address = server.address();

  if (!isAddressInfo(address)) {
    throw new Error("stub server has no TCP address");
  }

  return address.port;
}

function isAddressInfo(address: AddressInfo | string | null): address is AddressInfo {
  return typeof address === "object" && address !== null;
}

async function answersServer(): Promise<string> {
  return stubServer((request, response) => {
    let body = "";
    request.on("data", (chunk) => {
      body += String(chunk);
    });
    request.on("end", () => {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(wireBody(stubAnswers(parseJson(body))));
    });
  });
}

function runCli(
  args: string[],
  env: { [name: string]: string } = {},
  stdinText: string | null = null,
): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [cli, ...args], {
      env: {
        ...process.env,
        TYPESAFE_API_KEY: "",
        TYPESAFE_BASE_URL: "",
        ...env,
      },
      stdio: [stdinText === null ? "ignore" : "pipe", "pipe", "pipe"],
    });

    if (stdinText !== null && child.stdin !== null) {
      child.stdin.end(stdinText);
    }

    if (child.stdout === null || child.stderr === null) {
      throw new Error("child process streams are missing");
    }

    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += String(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderr += String(chunk);
    });
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
}

function jsonOutput(text: string): JsonObject {
  return requireObjectValue(parseJson(text), "cli output");
}

test("dry run prints planned requests without an API key", async () => {
  const result = await runCli(["changed", "--input", changedFixture, "--dry-run"]);
  assert.equal(result.code, 0, result.stderr);
  const plan = jsonOutput(result.stdout);
  assert.equal(optionalBoolean(plan, "dryRun"), true);
  assert.equal(asArray(plan.items).length, 3);
  const batches = asArray(plan.batches);
  assert.equal(batches.length, 1);
  const batch = asObject(batches[0]);
  assert.ok(batch !== null);
  const questions = batch === null ? null : asObject(batch.questions);
  assert.equal(questions === null ? 0 : Object.keys(questions).length, 9);
});

test("unknown features and options are usage errors", async () => {
  const unknownFeature = await runCli(["nope"]);
  assert.equal(unknownFeature.code, 2);
  const unknownOption = await runCli(["changed", "--wat"]);
  assert.equal(unknownOption.code, 2);
  const badBatch = await runCli(["changed", "--batch-size", "0"]);
  assert.equal(badBatch.code, 2);
  const bigBatch = await runCli(["changed", "--batch-size", "65"]);
  assert.equal(bigBatch.code, 2);
});

test("dry run covers every report feature offline", async () => {
  const cases = [
    { feature: "check", fixture: checkFixture, items: 2 },
    { feature: "security", fixture: securityFixture, items: 2 },
    { feature: "debt", fixture: debtFixture, items: 2 },
    { feature: "duplication", fixture: duplicationFixture, items: 2 },
  ];

  for (const entry of cases) {
    const result = await runCli([entry.feature, "--input", entry.fixture, "--dry-run"]);
    assert.equal(result.code, 0, `${entry.feature}: ${result.stderr}`);
    const plan = jsonOutput(result.stdout);
    assert.equal(optionalBoolean(plan, "dryRun"), true);
    assert.equal(asArray(plan.items).length, entry.items, entry.feature);
  }
});

test("human output renders the ranked worklist", async () => {
  const url = await answersServer();

  const result = await runCli(["changed", "--input", changedFixture], {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_BASE_URL: url,
  });

  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /Changed\/regression triage/);
  assert.match(result.stdout, /priority/);
  assert.match(result.stdout, /usage: \d+ input/);
});

test("a full run ranks items end to end against a stub service", async () => {
  const url = await answersServer();

  const result = await runCli(["changed", "--input", changedFixture, "--json"], {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_BASE_URL: url,
  });

  assert.equal(result.code, 0, result.stderr);
  const output = jsonOutput(result.stdout);
  assert.equal(optionalString(output, "feature"), "changed");
  const items = asArray(output.items);
  assert.equal(items.length, 3);
  const usage = asObject(output.usage);
  assert.equal(usage === null ? 0 : optionalNumber(usage, "inputTokens"), 100);

  const priorities = items.map((entry) => {
    const item = asObject(entry);

    return item === null ? 0 : (optionalNumber(item, "priority") ?? 0);
  });

  assert.deepEqual(
    priorities,
    [...priorities].sort((left, right) => right - left),
  );
});

test("missing credentials fail with guidance", async () => {
  const url = await answersServer();

  const result = await runCli(["changed", "--input", changedFixture, "--json"], {
    TYPESAFE_API_KEY: "",
    TYPESAFE_BASE_URL: url,
  });

  assert.equal(result.code, 3);
  assert.match(result.stderr, /TYPESAFE_API_KEY/);
});

test("route selects a workflow and its command", async () => {
  const url = await answersServer();

  const result = await runCli(["route", "did my last edit make things worse?", "--json"], {
    TYPESAFE_API_KEY: "test-key",
    TYPESAFE_BASE_URL: url,
  });

  assert.equal(result.code, 0, result.stderr);
  const output = jsonOutput(result.stdout);
  assert.equal(optionalString(output, "workflow"), "changed_review");
  assert.equal(optionalString(output, "action"), "run");
  assert.match(optionalString(output, "command") ?? "", /leadline changed/);
});

test("route reads the request from stdin when piped", async () => {
  const url = await answersServer();

  const result = await runCli(
    ["route", "--json"],
    { TYPESAFE_API_KEY: "test-key", TYPESAFE_BASE_URL: url },
    "why did the gate fail on my branch?",
  );

  assert.equal(result.code, 0, result.stderr);
  const output = jsonOutput(result.stdout);
  assert.equal(optionalString(output, "request"), "why did the gate fail on my branch?");
  assert.equal(optionalString(output, "workflow"), "changed_review");
});
