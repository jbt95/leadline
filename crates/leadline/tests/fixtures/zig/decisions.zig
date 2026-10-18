pub fn choose(value: i32) i32 {
    if (value > 0) {
        return 1;
    } else if (value < 0) {
        return -1;
    } else {
        return 0;
    }
}

pub fn loops(values: []const i32) i32 {
    var total: i32 = 0;
    for (values) |value| {
        if (value == 0) continue;
        total += value;
    }
    while (total > 100) {
        total -= 1;
    }
    return total;
}

pub fn classify(value: i32) i32 {
    return switch (value) {
        0 => 0,
        1 => 1,
        else => -1,
    };
}

pub fn labeled(values: []const i32) i32 {
    outer: for (values) |value| {
        if (value < 0) break :outer;
    }
    return 0;
}
