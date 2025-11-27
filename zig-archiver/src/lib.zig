const std = @import("std");

/// Core implementation (nullable args, more defensive).
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

    if (in_path.len == 0 or out_path.len == 0 or std.mem.eql(u8, in_path, out_path)) {
        return 10; // invalid arguments
    }

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

    // NOTE: For now we just do a raw copy (NO gzip compression).
    var buf: [16 * 1024]u8 = undefined;

    while (true) {
        const read_bytes = in_file.read(&buf) catch |e| {
            std.debug.print("[zig] read error: {s}\n", .{@errorName(e)});
            return 4;
        };
        if (read_bytes == 0) break;

        out_file.writeAll(buf[0..read_bytes]) catch |e| {
            std.debug.print("[zig] write error: {s}\n", .{@errorName(e)});
            return 5;
        };
    }

    return 0;
}

/// Backwards-compatible wrapper for older Rust code that calls `konserve_gzip_tar`.
pub export fn konserve_gzip_tar(
    in_path_z: [*:0]const u8,
    out_path_z: [*:0]const u8,
) c_int {
    return konserve_gzip_tar_v2(in_path_z, out_path_z);
}

/// Future, more flexible FFI entry point (currently a stub).
pub export fn konserve_compress(
    level: u8,
    output: [*:0]const u8,
    files: [*]const [*:0]const u8,
    file_count: usize,
) c_int {
    std.debug.print(
        "[zig] konserve_compress stub -> level {}, output: {s}, files: {d}\n",
        .{ level, output, file_count },
    );

    if (file_count == 0) {
        std.debug.print("[zig] konserve_compress: no files provided (stub)\n", .{});
    } else {
        std.debug.print("[zig] konserve_compress: file list (stub):\n", .{});
        var i: usize = 0;
        while (i < file_count) : (i += 1) {
            const p = files[i];
            std.debug.print("  {s}\n", .{p});
        }
    }

    return 0;
}
