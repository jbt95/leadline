import { allowed } from "./rules";
import { save } from "./store";

export function process(items: number[]): number {
  if (!allowed(items)) {
    return 0;
  }
  save(items);
  return items.length;
}
