fn log(x: i32) {
    println!("value {}", x);
}

fn pick_macro(x: i32) -> Vec<i32> {
    vec![if x > 1 { 1 } else { 0 }]
}

fn guarded(x: i32) -> i32 {
    if x > 1 {
        x
    } else {
        panic!("small")
    }
}

fn checked(text: &str) -> i32 {
    let value = text.parse::<i32>().expect("number");
    value
}

fn allow(flag: bool) -> bool {
    flag || true
}
