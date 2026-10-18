function outer(flag) {
  if (flag) {
    const inner = x => x ? 1 : 0;
    return inner(1);
  }
  return 0;
}
