fn both(left: Option<i32>, right: Option<i32>) -> bool {
    if let Some(a) = left
        && let Some(b) = right
    {
        return a == b;
    }
    false
}
