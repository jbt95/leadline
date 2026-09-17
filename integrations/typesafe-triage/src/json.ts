//! The JSON boundary. Reports and API responses arrive as text; this module
//! parses that text once and decodes it explicitly, so no `unknown` value ever
//! travels deeper into the program (anti-slop `no-unknown-*` rules).

export class JsonError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "JsonError";
  }
}

export type JsonObject = { [key: string]: JsonValue };

export type JsonValue = string | number | boolean | null | JsonValue[] | JsonObject;

export type NumberMap = { [key: string]: number };

function isFiniteNumber(value: JsonValue | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isBoolean(value: JsonValue | undefined): value is boolean {
  return typeof value === "boolean";
}

export function parseJson(text: string): JsonValue {
  // SAFETY: JSON.parse either throws or returns a value from the JSON grammar,
  // which is exactly this union.
  return JSON.parse(text) as JsonValue;
}

export function isJsonObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isJsonArray(value: JsonValue | undefined): value is JsonValue[] {
  return Array.isArray(value);
}

export function isString(value: JsonValue | undefined): value is string {
  return typeof value === "string";
}

/** Nested access with a null result instead of a throw, for optional rows. */
export function asObject(value: JsonValue | undefined): JsonObject | null {
  return isJsonObject(value) ? value : null;
}

export function asArray(value: JsonValue | undefined): JsonValue[] {
  return isJsonArray(value) ? value : [];
}

export function optionalString(object: JsonObject, key: string): string | null {
  const value = object[key];

  return isString(value) ? value : null;
}

export function optionalNumber(object: JsonObject, key: string): number | null {
  const value = object[key];

  return isFiniteNumber(value) ? value : null;
}

export function optionalBoolean(object: JsonObject, key: string): boolean | null {
  const value = object[key];

  return isBoolean(value) ? value : null;
}

export function requireString(object: JsonObject, key: string, context: string): string {
  const value = optionalString(object, key);

  if (value === null) {
    throw new JsonError(`${context}.${key} must be a string`);
  }

  return value;
}

export function requireNumber(object: JsonObject, key: string, context: string): number {
  const value = optionalNumber(object, key);

  if (value === null) {
    throw new JsonError(`${context}.${key} must be a finite number`);
  }

  return value;
}

export function requireNumberMap(object: JsonObject, key: string, context: string): NumberMap {
  const source = requireObjectValue(object[key], `${context}.${key}`);
  const entries: [string, number][] = [];

  for (const [name, value] of Object.entries(source)) {
    if (!isFiniteNumber(value) || value < 0 || value > 1) {
      // The key comes from the response body, so it never enters the message.
      throw new JsonError(`${context}.${key} has an invalid probability`);
    }

    entries.push([name, value]);
  }

  return Object.fromEntries(entries);
}

export function requireObjectValue(value: JsonValue | undefined, context: string): JsonObject {
  if (!isJsonObject(value)) {
    throw new JsonError(`${context} must be a JSON object`);
  }

  return value;
}
