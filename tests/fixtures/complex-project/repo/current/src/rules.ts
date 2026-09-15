import { normalizeOrder } from "./service";

export function validRegion(region: string): boolean {
  const normalized = normalizeOrder([region.length]);
  return normalized.length > 0 && (region === "EU" || region === "US");
}
