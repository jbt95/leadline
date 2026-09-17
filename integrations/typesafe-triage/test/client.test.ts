import assert from "node:assert/strict";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { after, test } from "node:test";

import {
  TypeSafeError,
  ask,
  choice,
  configFromEnv,
  noul,
  score,
} from "../src/client.ts";
import { isJsonObject, parseJson } from "../src/json.ts";
import type { ClientConfig } from "../src/types.ts";

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

function configFor(baseUrl: string, overrides: Partial<ClientConfig> = {}): ClientConfig {
  return {
    apiKey: "test-key",
    baseUrl,
    model: "jev-test",
    timeoutMs: 1_000,
    ...overrides,
  };
}

test("configFromEnv requires a key and applies overrides", () => {
  assert.throws(() => configFromEnv({}), /TYPESAFE_API_KEY/);
  assert.throws(() => configFromEnv({ TYPESAFE_API_KEY: "  " }), /TYPESAFE_API_KEY/);

  const defaults = configFromEnv({ TYPESAFE_API_KEY: "key" });
  assert.equal(defaults.baseUrl, "https://api.typesafe.ai");
  assert.equal(defaults.model, "jev-latest");
  assert.equal(defaults.timeoutMs, 10_000);

  const overridden = configFromEnv({
    TYPESAFE_API_KEY: "key",
    TYPESAFE_BASE_URL: "http://127.0.0.1:1/",
    TYPESAFE_DEFAULT_MODEL: "jev-test",
    TYPESAFE_TIMEOUT_MS: "500",
  });

  assert.equal(overridden.baseUrl, "http://127.0.0.1:1/");
  assert.equal(overridden.model, "jev-test");
  assert.equal(overridden.timeoutMs, 500);
});

test("ask sends one bounded request and returns the answers", async () => {
  let resolveObserved: (value: {
    url: string;
    method: string;
    auth: string | undefined;
    body: string;
  }) => void = () => {};

  const observed = new Promise<{
    url: string;
    method: string;
    auth: string | undefined;
    body: string;
  }>((resolve) => {
    resolveObserved = resolve;
  });

  const url = await stubServer((request, response) => {
    let body = "";
    request.on("data", (chunk) => {
      body += String(chunk);
    });
    request.on("end", () => {
      resolveObserved({
        url: request.url ?? "",
        method: request.method ?? "",
        auth: request.headers.authorization,
        body,
      });
      response.writeHead(200, { "content-type": "application/json" });
      response.end(
        JSON.stringify({
          model: "jev-test",
          answers: { urgency: { type: "noul", noul: 0.8 } },
          usage: { input_tokens: 10, output_tokens: 2 },
        }),
      );
    });
  });

  const payload = await ask({
    state: { ticket: "help" },
    questions: { urgency: noul("is it urgent?") },
    config: configFor(url),
  });

  const request = await observed;
  assert.equal(request.url, "/v1/systemone");
  assert.equal(request.method, "POST");
  assert.equal(request.auth, "Bearer test-key");
  const body = parseJson(request.body);
  assert.ok(isJsonObject(body));
  assert.equal(payload.answers.urgency?.type, "noul");
  assert.equal(payload.usage.inputTokens, 10);
});

test("ask surfaces HTTP failures and malformed payloads", async () => {
  const failingUrl = await stubServer((request, response) => {
    response.writeHead(500, { "content-type": "application/json" });
    response.end(JSON.stringify({ error: "boom" }));
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: noul("yes?") },
      config: configFor(failingUrl),
    }),
    (error: Error) => error instanceof TypeSafeError && error.status === 500,
  );

  const emptyUrl = await stubServer((request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ model: "m" }));
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: noul("yes?") },
      config: configFor(emptyUrl),
    }),
    /response\.answers must be a JSON object/,
  );

  const unknownTypeUrl = await stubServer((request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ answers: { a: { type: "ranking" } } }));
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: choice("pick", { one: "first", two: "second" }) },
      config: configFor(unknownTypeUrl),
    }),
    /wrong type/,
  );
});

test("a non-JSON success body never echoes the body", async () => {
  const htmlUrl = await stubServer((request, response) => {
    response.writeHead(200, { "content-type": "text/html" });
    response.end("<html><body>gateway error</body></html>");
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: noul("yes?") },
      config: configFor(htmlUrl),
    }),
    (error: Error) => {
      assert.equal(error.message, "TypeSafe response is not valid JSON");
      assert.ok(!error.message.includes("<html"));

      return true;
    },
  );
});

test("a response missing a requested answer fails without echoing the body", async () => {
  const partialUrl = await stubServer((request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(
      JSON.stringify({
        answers: { a: { type: "noul", noul: 0.5 } },
      }),
    );
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: noul("yes?"), b: noul("also?") },
      config: configFor(partialUrl),
    }),
    (error: Error) => {
      assert.match(error.message, /TypeSafe answer 'b'/);
      assert.ok(!error.message.includes("also?"));

      return true;
    },
  );
});

test("out-of-range values and probability keys never echo body text", async () => {
  const badConfidenceUrl = await stubServer((request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(
      JSON.stringify({
        answers: {
          a: {
            type: "choice",
            choice: "body-secret-choice",
            probabilities: { "body-secret-choice": 0.9 },
            confidence: 2,
          },
        },
      }),
    );
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: choice("pick", { one: "first", two: "second" }) },
      config: configFor(badConfidenceUrl),
    }),
    (error: Error) => {
      assert.match(error.message, /out-of-range confidence/);
      assert.ok(!error.message.includes("body-secret-choice"));

      return true;
    },
  );

  const badProbabilityUrl = await stubServer((request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(
      JSON.stringify({
        answers: {
          a: {
            type: "choice",
            choice: "one",
            probabilities: { "body-secret-key": 7 },
            confidence: 0.5,
          },
        },
      }),
    );
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: choice("pick", { one: "first", two: "second" }) },
      config: configFor(badProbabilityUrl),
    }),
    (error: Error) => {
      assert.match(error.message, /invalid probability/);
      assert.ok(!error.message.includes("body-secret-key"));

      return true;
    },
  );
});

test("ask times out instead of hanging", async () => {
  const slowUrl = await stubServer((request, response) => {
    setTimeout(() => {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify({ answers: { a: { type: "noul", noul: 0.5 } } }));
    }, 500);
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: noul("yes?") },
      config: configFor(slowUrl, { timeoutMs: 30 }),
    }),
    /timed out/,
  );
});

test("the timeout also covers a stalled response body", async () => {
  const stalledUrl = await stubServer((request, response) => {
    // Headers arrive immediately; the body never finishes within the budget.
    response.writeHead(200, { "content-type": "application/json" });
    response.flushHeaders();
    setTimeout(() => response.end("{}"), 500);
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: noul("yes?") },
      config: configFor(stalledUrl, { timeoutMs: 30 }),
    }),
    /timed out/,
  );
});

test("error messages carry the status and never echo the key", async () => {
  const echoUrl = await stubServer((request, response) => {
    response.writeHead(400, { "content-type": "application/json" });
    response.end(JSON.stringify({ echoed: request.headers.authorization }));
  });

  await assert.rejects(
    ask({
      state: "x",
      questions: { a: noul("yes?") },
      config: configFor(echoUrl, { apiKey: "super-secret-key" }),
    }),
    (error: Error) => {
      assert.equal(error instanceof TypeSafeError ? error.status : null, 400);
      assert.equal(error.message, "TypeSafe returned 400");
      assert.ok(!error.message.includes("super-secret-key"));

      return true;
    },
  );
});

test("score questions are sent with their levels", async () => {
  let resolveBody: (value: string) => void = () => {};

  const receivedBody = new Promise<string>((resolve) => {
    resolveBody = resolve;
  });

  const url = await stubServer((request, response) => {
    let received = "";
    request.on("data", (chunk) => {
      received += String(chunk);
    });
    request.on("end", () => {
      resolveBody(received);
      response.writeHead(200, { "content-type": "application/json" });
      response.end(
        JSON.stringify({
          answers: {
            inherent: {
              type: "score",
              score: 1.5,
              probabilities: { 0: 0.2, 1: 0.6, 2: 0.2 },
              confidence: 0.7,
            },
          },
        }),
      );
    });
  });

  const payload = await ask({
    state: { item: 1 },
    questions: { inherent: score("how inherent?", ["low", "medium", "high"]) },
    config: configFor(url),
  });

  const body = await receivedBody;
  assert.ok(body.includes('"criteria":["low","medium","high"]'));
  assert.equal(payload.answers.inherent?.type, "score");
});
