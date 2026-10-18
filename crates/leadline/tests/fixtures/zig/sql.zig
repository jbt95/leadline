pub fn find(db: anytype, name: []const u8) void {
    _ = db.query(
        // This comment precedes the dynamic first argument.
        "SELECT * FROM users WHERE name = '" ++ name ++ "'",
    );
}

pub fn get(db: anytype, id: i64) void {
    _ = db.query(
        "SELECT * FROM users WHERE id = ?",
        .{id},
    );
}

pub fn arithmetic(db: anytype, left: i64, right: i64) void {
    _ = db.query(left + right);
}

pub fn all(db: anytype, names: []const []const u8) void {
    for (names) |name| {
        _ = db.execute("SELECT * FROM users WHERE name = ?", .{name});
    }
}
