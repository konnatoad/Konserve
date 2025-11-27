const std = @import("std");
const arch = @import("konserve_archiver");

pub fn main() !void {
    const gpa = std.heap.page_allocator;
    const args = try std.process.argsAlloc(gpa);
    defer std.process.argsFree(gpa, args);

    std.debug.print("konserve-archiver CLI started with {} args\n", .{args.len});

    if (args.len != 3) {
        std.debug.print("Usage: konserve-archiver <in_path> <out_path>\n", .{});
        return;
    }

    const in_path: [*:0]const u8 = args[1];
    const out_path: [*:0]const u8 = args[2];

    const code = arch.konserve_gzip_tar(in_path, out_path);
    std.debug.print("konserve_gzip_tar returned: {d}\n", .{code});
}
