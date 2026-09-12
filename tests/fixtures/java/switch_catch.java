class SwitchCatch {
    int parse(int x) {
        try {
            switch (x) {
                case 1: return 1;
                case 2: return 2;
                default: return 0;
            }
        } catch (RuntimeException error) {
            return -1;
        }
    }
}
