import { allowed } from "./rules";
import { save } from "./store";

export function process(items: number[]): number {
  let accepted = 0;
  for (const item of items) {
    if (item > 0) {
      accepted += item;
    }
  }
  if (!allowed(items)) {
    return 0;
  }
  save(items);
  return accepted;
}
