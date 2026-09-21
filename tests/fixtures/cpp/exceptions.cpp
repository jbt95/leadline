int parse(int value) {
    try {
        if (value < 0) {
            throw value;
        }
    } catch (...) {
        return 0;
    }
    return value;
}
