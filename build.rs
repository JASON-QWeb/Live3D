use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=native/douyin-companion");
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
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("unsupported macOS architecture {other}"),
    };
    let binary = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("avatar3d-douyin");
    let status = Command::new("xcrun")
        .args(["--sdk", "macosx", "swiftc", "-O", "-target"])
        .arg(format!("{arch}-apple-macosx14.0"))
        .args([
            "native/douyin-companion/Messages.swift",
            "native/douyin-companion/Accessibility.swift",
            "native/douyin-companion/Reader.swift",
            "native/douyin-companion/main.swift",
            "-o",
        ])
        .arg(&binary)
        .status()
        .expect("building the Mac companion reader requires Xcode Command Line Tools");
    assert!(
        status.success(),
        "could not compile the Mac companion reader"
    );
    println!(
        "cargo:rustc-env=AVATAR3D_COMPILED_COMPANION={}",
        binary.display()
    );
}
