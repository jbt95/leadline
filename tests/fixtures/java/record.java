record Range(int start, int end) {
    Range {
        if (start > end) {
            throw new IllegalArgumentException();
        }
    }
}
