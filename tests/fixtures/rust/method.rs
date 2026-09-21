struct Counter {
    n: i32,
}

impl Counter {
    fn add(&mut self, delta: i32) -> i32 {
        self.n += delta;
        self.n
    }
}
