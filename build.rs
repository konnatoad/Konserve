use std::{env, fs, path::PathBuf};

fn embed_fingerprint() {
    const KEY: &str = "FINGERPRINT";

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let env_path: PathBuf = [manifest_dir.as_str(), ".env"].iter().collect();
    if let Ok(content) = fs::read_to_string(&env_path) {
        if let Some(val) = content
            .lines()
            .find_map(|line| line.trim_start().strip_prefix(&format!("{KEY}=")))
        {
            println!("cargo:rustc-env={KEY}={val}");
            println!("cargo:rerun-if-changed={}", env_path.display());
            println!("cargo:warning=build.rs saw FINGERPRINT=\"{val}\"");
        }
    }
}

#[cfg(windows)]
fn main() {
    embed_fingerprint();
    // Only force GUI mode in release builds
    if std::env::var("PROFILE").unwrap_or_default() == "release" {
        println!("cargo:rustc-link-arg=/SUBSYSTEM:windows");
    }

    let _ = embed_resource::compile("assets/windows.rc", embed_resource::NONE);
}

#[cfg(not(windows))]
fn main() {
    embed_fingerprint();
}
