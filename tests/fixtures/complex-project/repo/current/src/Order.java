package src;

record Order(int start, int end) {
    Order {
        if (start > end) {
            throw new IllegalArgumentException();
        }
    }

    int accepted(java.util.List<Integer> values) {
        return (int) values.stream()
            .filter(value -> value >= start && value <= end)
            .count();
    }
}
