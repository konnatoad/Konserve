const std = @import("std");
const gzip = std.compress.gzip;

pub export fn konserve_gzip_tar_v2(
    in_path_z_opt: ?[*:0]const u8,
    out_path_z_opt: ?[*:0]const u8,
) c_int {
    if (in_path_z_opt == null or out_path_z_opt == null) return 12;

    const in_path_z = in_path_z_opt.?;
    const out_path_z = out_path_z_opt.?;

    const in_path = std.mem.span(in_path_z);
    const out_path = std.mem.span(out_path_z);
    std.debug.print("[zig] konserve_gzip_tar_v2: in='{s}' out='{s}'\n", .{ in_path, out_path });

    if (in_path.len == 0 or out_path.len == 0 or std.mem.eql(u8, in_path, out_path)) return 10;

    var in_file = std.fs.cwd().openFile(in_path, .{}) catch |e| {
        std.debug.print("[zig] open in failed: {s}\n", .{@errorName(e)});
        return 1;
    };
    defer in_file.close();

    var out_file = std.fs.cwd().createFile(out_path, .{ .truncate = true }) catch |e| {
        std.debug.print("[zig] create out failed: {s}\n", .{@errorName(e)});
        return 2;
    };
    defer out_file.close();

    const opts: gzip.Options = .{};
    if (gzip.compress(in_file.reader(), out_file.writer(), opts)) |_| {
        return 0;
    } else |e| {
        std.debug.print("[zig] gzip error: {s}\n", .{@errorName(e)});
        return 3;
    }
}

// Back-compat wrapper to satisfy Rust calling `konserve_gzip_tar`
pub export fn konserve_gzip_tar(
    in_path_z: [*:0]const u8,
    out_path_z: [*:0]const u8,
) c_int {
    return konserve_gzip_tar_v2(in_path_z, out_path_z);
}
