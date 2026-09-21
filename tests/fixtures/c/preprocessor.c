int preprocessed(int value) {
#if ENABLED
  return value;
#else
  return 0;
#endif
}
