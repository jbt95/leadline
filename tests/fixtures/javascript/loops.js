function loops(items) {
  for (const item of items) {
    while (item.ready) {
      break;
    }
  }
}
