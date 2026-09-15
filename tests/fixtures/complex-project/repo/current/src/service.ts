import { validRegion } from "./rules";
import { saveOrder } from "./store";
import lodash from "lodash";

export function normalizeOrder(items: number[]): number[] {
  return lodash.uniq(items).filter((item) => item > 0);
}

export function priceOrder(items: number[], region: string): number {
  const normalized = normalizeOrder(items);
  let total = 0;
  for (const item of normalized) {
    if (item > 100) {
      total += item * 0.9;
    } else if (item > 10) {
      total += item * 0.95;
    } else {
      total += item;
    }
  }
  if (!validRegion(region) || total < 0) {
    throw new Error("invalid order");
  }
  saveOrder(normalized);
  return total;
}
