fn fact(n: i32) -> i32 {
    if n <= 1 {
        return 1;
    }
    n * fact(n - 1)
}

fn down<T>(items: &[T]) -> i32 {
    if items.is_empty() {
        return 0;
    }
    1 + down::<T>(&items[1..])
}
