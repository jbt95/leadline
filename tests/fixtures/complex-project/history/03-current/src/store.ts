export function save(items: number[]): number {
  let written = 0;
  for (const item of items) {
    if (item > 0) {
      written += 1;
    }
  }
  return written;
}
