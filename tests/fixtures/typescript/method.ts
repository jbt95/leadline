class Counter {
  update(value: number, limit: number): number {
    return value > limit ? limit : value;
  }
}
