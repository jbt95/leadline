pub fn find(db: anytype, name: []const u8) void {
    _ = db.query("SELECT * FROM users WHERE name = '" ++ name ++ "'");
}

pub fn get(db: anytype, id: i64) void {
    _ = db.query(
        // A literal and bound parameters do not make SQL text dynamic.
        "SELECT * FROM users WHERE id = ?",
        .{id},
    );
}

pub fn all(db: anytype, names: []const []const u8) void {
    for (names) |name| {
        _ = db.execute("SELECT * FROM users WHERE name = ?", .{name});
    }
}
