class Arrow {
    java.util.function.IntUnaryOperator twice(int scale) {
        java.util.function.IntUnaryOperator op = x -> x * scale;
        return op;
    }
}
