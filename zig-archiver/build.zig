const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const lib = b.addLibrary(.{
        .name = "konserve_archiver",
        .linkage = .static,
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/lib.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });

    lib.root_module.pic = true;
    lib.root_module.stack_check = false;

    b.installArtifact(lib);

    const exe = b.addExecutable(.{
        .name = "konserve-archiver",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/cli.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });

    exe.root_module.addImport("konserve_archiver", lib.root_module);

    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    if (b.args) |args| run_cmd.addArgs(args);

    const run_step = b.step("run", "Run the konserve-archiver CLI");
    run_step.dependOn(&run_cmd.step);
}
