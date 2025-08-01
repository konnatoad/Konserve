// Konserve Zig Compression Module
//
// This Zig file serves two purposes:
//
// 1. Provides a C-ABI compatible function 'konserve_compress' that Rust can call.
//  - This will eventually handle creating .tar.gz backups with a given compression level and output path
//  - Right now, it's only a stub for testing FFI integration with Rust.
//
// 2. Allows running the Zig binary standalone for debugging.
//  - Supports basic CLI arguments: '--level', '--output' and a list of files.
//  - Prints parsed arguments so i can confirm everything works before wiring
//    in the actual compression logic.
//
// Build setup:
//  - Zig build script is configured to produce a static library
//    so that cargo can link it directly into the main Konserve executable.
//  - This ensures the end user only needs a single binary, while letting me write the compression backend in Zig.
//
// TODO: Replace the stubbed function with real .tar.gz logic.
// TODO: Extend FFI interface to accept file lists from Rust.
// TODO: Wire compression settings from konserve's Rust config into the Zig compressor.

const std = @import("std");
const builtin = @import("builtin");
const cli_build = @import("root").cli_build; // build.zig sets this

pub export fn konserve_compress(
    level: u8,
    output: [*:0]const u8,
    files: [*]const [*:0]const u8,
    file_count: usize,
) c_int {
    std.debug.print("Zig compression stub -> level {}, output: {s}\n", .{ level, output });
    return 0;
}

pub fn main() !void {
    if (!cli_build) return; // don’t compile CLI main for library builds

    const gpa = std.heap.page_allocator;
    const args = try std.process.argsAlloc(gpa);
    defer std.process.argsFree(gpa, args);

    // CLI arg parsing stays as you wrote
    if (args.len > 1 and std.mem.eql(u8, args[1], "--help")) {
        std.debug.print(
            "Usage: konserve-archiver [--level N] [--output FILE] [files...]\n\n" ++
                "Options:\n" ++
                "   --level N       Compression level (1-9), default 6\n" ++
                "   --output FILE   Output filename, default backup.tar.gz\n",
            .{},
        );
        return;
    }

    var level: []const u8 = "6";
    var output: []const u8 = "backup.tar.gz";
    var files = std.ArrayList([]const u8).init(gpa);
    defer files.deinit();

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        if (std.mem.eql(u8, args[i], "--level")) {
            if (i + 1 < args.len) {
                i += 1;
                level = args[i];
            }
        } else if (std.mem.eql(u8, args[i], "--output")) {
            if (i + 1 < args.len) {
                i += 1;
                output = args[i];
            }
        } else {
            try files.append(args[i]);
        }
    }

    std.debug.print("level: {s}\n", .{level});
    std.debug.print("output: {s}\n", .{output});
    if (files.items.len == 0) {
        std.debug.print("no files passed\n", .{});
    } else {
        std.debug.print("files:\n", .{});
        for (files.items) |f| {
            std.debug.print("  {s}\n", .{f});
        }
    }
}
