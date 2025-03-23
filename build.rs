#[cfg(windows)]
fn main() {
    println!("cargo:rustc-link-arg=/SUBSYSTEM:windows");
}

#[cfg(not(windows))]
fn main() {}
