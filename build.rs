#[cfg(windows)]
fn main() {
    // Only force GUI mode in release builds
    if std::env::var("PROFILE").unwrap_or_default() == "release" {
        println!("cargo:rustc-link-arg=/SUBSYSTEM:windows");
    }

    let _ = embed_resource::compile("assets/windows.rc", embed_resource::NONE);
}

#[cfg(not(windows))]
fn main() {}
