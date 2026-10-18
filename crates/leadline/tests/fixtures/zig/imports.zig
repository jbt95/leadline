const helper = @import("helper.zig");
const external = @import("some-package");
const missing = @import("missing.zig");
const outside = @import("../../outside.zig");
const styles = @import("./styles.css");
const c = @cImport("header.h");
const embedded = @embedFile("data.txt");

pub fn main() void {
    _ = helper.run();
    _ = external;
    _ = missing;
    _ = outside;
    _ = styles;
    _ = c;
    _ = embedded;
}
