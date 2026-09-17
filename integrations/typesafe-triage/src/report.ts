//! Reads one leadline JSON report from a file or stdin. Every triage feature
//! consumes leadline output verbatim; the companion never re-analyzes code.

import { readFile } from "node:fs/promises";
import { parseJson, type JsonValue } from "./json.ts";

export async function readReport(options: { input: string | null }): Promise<JsonValue> {
  const text =
    options.input !== null && options.input !== "-"
      ? await readFile(options.input, "utf8")
      : await readStdin();

  if (text.trim() === "") {
    throw new Error(
      "no report received: pass --input FILE or pipe leadline output on stdin",
    );
  }

  return parseJson(text);
}

async function readStdin(): Promise<string> {
  const chunks: Uint8Array[] = [];

  for await (const chunk of process.stdin) {
    chunks.push(Buffer.from(chunk));
  }

  return Buffer.concat(chunks).toString("utf8");
}
