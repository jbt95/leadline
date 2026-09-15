import { process } from "./service";

export function allowed(items: number[]): boolean {
  return items.length === 0 || process(items) > 0;
}
