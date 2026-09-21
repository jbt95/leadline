fn label(code: i32) -> &'static str {
    match code {
        0 => "zero",
        1 | 2 => "small",
        _ => "other",
    }
}

fn label_last(code: i32) -> &'static str {
    match code {
        0 => "zero",
        1 | 2 => "small",
        _ => "other"
    }
}
