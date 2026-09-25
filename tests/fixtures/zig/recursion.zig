pub fn fact(n: u32) u32 {
    if (n <= 1) return 1;
    return n * fact(n - 1);
}

pub fn two(a: i32, b: i32) i32 {
    return a + b;
}
