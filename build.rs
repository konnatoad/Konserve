#[cfg(windows)]
fn main() {
    println!("cargo:rustc-link-arg=/SUBSYSTEM:windows");

    let _ = embed_resource::compile("assets/windows.rc", embed_resource::NONE);
}

#[cfg(not(windows))]
fn main() {}
