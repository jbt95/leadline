pub fn logic(a: bool, b: bool, c: bool) bool {
    if (a and b or c) {
        return tryValue(a);
    }
    return a catch false;
}

fn tryValue(value: bool) bool {
    return value orelse false;
}
