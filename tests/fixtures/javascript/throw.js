function fail(value) {
  if (value < 0) {
    throw new Error("negative");
  }
  return value;
}
