const std = @import("std");

pub fn main() !void {
    const gpa = std.heap.page_allocator;
    const args = try std.process.argsAlloc(gpa);
    defer std.process.argsFree(gpa, args);

    std.debug.print("Konserve Zig CLI started with {} args\n", .{args.len});
}
