import { priceOrder } from "./service";
import "./missing";

export interface QuoteInput {
  items: number[];
  region: string;
}

export function quote(input: QuoteInput): number {
  if (input.items.length === 0) {
    return 0;
  }
  switch (input.region) {
    case "EU":
      return priceOrder(input.items, input.region) * 1.2;
    case "US":
      return priceOrder(input.items, input.region);
    default:
      throw new Error("unsupported region");
  }
}
