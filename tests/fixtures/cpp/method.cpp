class Counter {
public:
    int add(int delta) {
        if (delta > 0) {
            return delta;
        }
        return 0;
    }
};
