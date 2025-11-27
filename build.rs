use std::{env, fs, path::PathBuf, process::Command};

fn embed_fingerprint() {
    const KEY: &str = "FINGERPRINT";

    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set (build.rs is running outside cargo?)");

    let env_path: PathBuf = [manifest_dir.as_str(), ".env"].iter().collect();

    // Read .env and look for a line starting with FINGERPRINT=
    if let Some(val) = fs::read_to_string(&env_path).ok().and_then(|contents| {
        contents
            .lines()
            .find_map(|line| line.trim_start().strip_prefix(&format!("{KEY}=")))
            .map(str::to_owned)
    }) {
        // Expose it to the Rust code at compile time as env!("FINGERPRINT")
        println!("cargo:rustc-env={KEY}={val}");
        println!("cargo:rerun-if-changed={}", env_path.display());
        println!("cargo:warning=build.rs saw FINGERPRINT=\"{val}\"");
    }
}

fn build_zig() {
    // Rebuild when Zig sources or build scripts change
    println!("cargo:rerun-if-changed=zig-archiver/src/lib.zig");
    println!("cargo:rerun-if-changed=zig-archiver/src/cli.zig");
    println!("cargo:rerun-if-changed=zig-archiver/build.zig");
    println!("cargo:rerun-if-changed=zig-archiver/build.zig.zon");
    println!("cargo:rerun-if-env-changed=PROFILE");

    // Match Zig optimize mode to Cargo profile
    let optimize_flag = match env::var("PROFILE").as_deref() {
        Ok("release") => "-Doptimize=ReleaseSafe",
        _ => "-Doptimize=Debug",
    };

    // Cargo gives us OUT_DIR, we keep all Zig artifacts + cache under there
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let zig_prefix = out_dir.join("zig-out");
    let cache_dir = out_dir.join("zig-cache");
    let global_cache = out_dir.join("zig-global-cache");
    let _ = fs::create_dir_all(&zig_prefix);
    let _ = fs::create_dir_all(&cache_dir);
    let _ = fs::create_dir_all(&global_cache);

    // Build the Zig subproject
    let status = Command::new("zig")
        .current_dir("zig-archiver")
        .args([
            "build",
            optimize_flag,
            "--cache-dir",
            cache_dir.to_string_lossy().as_ref(),
            "--global-cache-dir",
            global_cache.to_string_lossy().as_ref(),
            "-p",
            zig_prefix.to_string_lossy().as_ref(),
        ])
        .status()
        .expect("failed to run `zig build` (is `zig` on PATH?)");

    if !status.success() {
        panic!("zig build failed");
    }

    // Tell rustc where to find libkonserve_archiver.a
    let lib_dir = zig_prefix.join("lib");
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

    let _ = embed_resource::compile("assets/windows.rc", embed_resource::NONE);
}

#[cfg(not(windows))]
fn main() {
    build_zig();
    embed_fingerprint();
}
