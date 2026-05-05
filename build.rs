use std::{env, fs, path::PathBuf};

fn embed_fingerprint() {
    const KEY: &str = "FINGERPRINT";

    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set (build.rs is running outside cargo?)");

    let env_path: PathBuf = [manifest_dir.as_str(), ".env"].iter().collect();

    if let Some(val) = fs::read_to_string(&env_path).ok().and_then(|contents| {
        contents
            .lines()
            .find_map(|line| line.trim_start().strip_prefix(&format!("{KEY}=")))
            .map(str::to_owned)
    }) {
        println!("cargo:rustc-env={KEY}={val}");
        println!("cargo:rerun-if-changed={}", env_path.display());
        println!("cargo:warning=build.rs saw FINGERPRINT=\"{val}\"");
    }
}

#[cfg(windows)]
fn main() {
    embed_fingerprint();

    if env::var("PROFILE").unwrap_or_default() == "release" {
        println!("cargo:rustc-link-arg=/SUBSYSTEM:windows");
    }

    let _ = embed_resource::compile("assets/windows.rc", embed_resource::NONE);
}

#[cfg(not(windows))]
fn main() {
    embed_fingerprint();
}
