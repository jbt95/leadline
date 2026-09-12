// Canonical JSON boundary for this package.
//
// Raw `leadline` stdout is parsed into JsonValue exactly once, in
// core/index.ts. Decoders check each used property with typeof,
// Array.isArray, or `in` and build named domain types from it.
// isJsonObject is the single canonical object guard for the package.

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue | undefined };

export type JsonObject = { [key: string]: JsonValue | undefined };

// Type guard preserves narrowing for every decoder below.
export function isJsonObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
