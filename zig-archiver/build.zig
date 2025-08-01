const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // ---- static library used by Rust ----
    const lib = b.addStaticLibrary(.{
        .name = "konserve_archiver",
        .root_source_file = b.path("src/lib.zig"),
        .target = target,
        .optimize = optimize,
    });
    // Make it linkable into Rust’s PIE:
    lib.root_module.pic = true;
    // Optional: avoid needing __zig_probe_stack when linked into Rust:
    lib.root_module.stack_check = false;

    b.installArtifact(lib);

    // (optional) small CLI for debugging
    const exe = b.addExecutable(.{
        .name = "konserve-archiver",
        .root_source_file = b.path("src/cli.zig"),
        .target = target,
        .optimize = optimize,
    });
    b.installArtifact(exe);
}
