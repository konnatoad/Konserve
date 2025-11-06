use std::{env, fs, path::PathBuf, process::Command};

fn embed_fingerprint() {
    const KEY: &str = "FINGERPRINT";

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let env_path: PathBuf = [manifest_dir.as_str(), ".env"].iter().collect();

    if let Some(val) = fs::read_to_string(&env_path).ok().and_then(|c| {
        c.lines()
            .find_map(|line| line.trim_start().strip_prefix(&format!("{KEY}=")))
            .map(str::to_owned)
    }) {
        println!("cargo:rustc-env={KEY}={val}");
        println!("cargo:rerun-if-changed={}", env_path.display());
        println!("cargo:warning=build.rs saw FINGERPRINT=\"{val}\"");
    }
}

fn build_zig() {
    // Rebuild when Zig sources change
    println!("cargo:rerun-if-changed=zig-archiver/src/lib.zig");
    println!("cargo:rerun-if-changed=zig-archiver/src/cli.zig");
    println!("cargo:rerun-if-changed=zig-archiver/build.zig");
    println!("cargo:rerun-if-env-changed=PROFILE");

    // Optimize Zig build based on Cargo profile
    let optimize_flag = match env::var("PROFILE").as_deref() {
        Ok("release") => "-Doptimize=ReleaseSafe",
        _ => "-Doptimize=Debug",
    };

    let status = Command::new("zig")
        .current_dir("zig-archiver")
        .args(["build", optimize_flag])
        .status()
        .expect("failed to run `zig build` (is Zig on PATH?)");

    if !status.success() {
        panic!("zig build failed");
    }

    // Link the produced static lib
    let lib_dir = PathBuf::from("zig-archiver/zig-out/lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=konserve_archiver");
}

#[cfg(windows)]
fn main() {
    build_zig();
    embed_fingerprint();

    // Only force GUI subsystem in release builds
    if env::var("PROFILE").unwrap_or_default() == "release" {
        println!("cargo:rustc-link-arg=/SUBSYSTEM:windows");
    }

    // Optional: embed Windows resources (icon, etc.)
    let _ = embed_resource::compile("assets/windows.rc", embed_resource::NONE);
}

#[cfg(not(windows))]
fn main() {
    build_zig();
    embed_fingerprint();
}
