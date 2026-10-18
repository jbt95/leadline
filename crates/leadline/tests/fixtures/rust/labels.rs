fn find(values: &[i32], target: i32) -> bool {
    'outer: for value in values {
        if *value == target {
            break 'outer;
        }
    }
    false
}
