import { allowed } from "./policy";
import { save } from "./store";

export function process(items: number[]): number {
  let accepted = 0;
  for (const item of items) {
    if (item < 0) {
      continue;
    }
    if (item > 100) {
      accepted += item * 2;
    } else if (item > 10) {
      accepted += item;
    } else {
      accepted += 1;
    }
  }
  if (!allowed(items) || accepted === 0) {
    return 0;
  }
  save(items);
  return accepted;
}
