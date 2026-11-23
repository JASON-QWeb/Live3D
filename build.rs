use std::env;

fn main() {
    println!("cargo:rerun-if-changed=native/desktop");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    cc::Build::new()
        .files(["native/desktop/Window.m", "native/desktop/FilePicker.m"])
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .compile("avatar3d_desktop");
    for framework in ["AppKit", "Foundation", "WebKit", "UniformTypeIdentifiers"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-lib=objc");
}
