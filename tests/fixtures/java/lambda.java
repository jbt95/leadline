class Lambda {
    int wrap(int value) {
        java.util.function.IntUnaryOperator twice = x -> x > 0 ? x * 2 : 0;
        return twice.applyAsInt(value);
    }
}
