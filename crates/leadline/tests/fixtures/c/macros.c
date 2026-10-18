#define PICK(value) do { if (value) { break; } } while (0)

int macro_body(int value) {
  PICK(value);
  return value;
}
