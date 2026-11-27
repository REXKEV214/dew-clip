use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_swift_bridge();
    }
    tauri_build::build()
}

/// The native Translation and URLSession bridges share a static Swift library.
fn build_swift_bridge() {
    const SOURCES: [&str; 2] = ["swift/Translate.swift", "swift/UpdateCheck.swift"];
    for source in SOURCES {
        println!("cargo:rerun-if-changed={source}");
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        Ok(other) => panic!("unsupported target arch {other}"),
        Err(_) => panic!("CARGO_CFG_TARGET_ARCH not set"),
    };
    // Same floor as the app (tauri.conf.json minimumSystemVersion); the code
    // itself checks for macOS 26 at run time.
    let target = format!("{arch}-apple-macos14.0");
    let library = out_dir.join("libdewclip_translate.a");

    let status = Command::new("xcrun")
        .args(["swiftc", "-emit-library", "-static", "-parse-as-library", "-O"])
        .args(["-swift-version", "5", "-target", &target, "-module-name", "DewClipTranslate"])
        .arg("-module-cache-path")
        .arg(out_dir.join("swift-module-cache"))
        .args(SOURCES)
        .arg("-o")
        .arg(&library)
        .status()
        .expect("failed to run swiftc (Xcode command line tools needed)");
    assert!(status.success(), "swiftc failed for the native bridges");

    let sdk = String::from_utf8(
        Command::new("xcrun")
            .args(["--sdk", "macosx", "--show-sdk-path"])
            .output()
            .expect("xcrun --show-sdk-path")
            .stdout,
    )
    .unwrap();
    let sdk = sdk.trim();

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=dewclip_translate");
    // The Swift runtime ships with macOS, in /usr/lib/swift.
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    // Not on macOS 14, so linked weakly: the app still starts there, and the
    // run-time check keeps it from being touched.
    for framework in ["Translation", "_Translation_SwiftUI"] {
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,{framework}");
    }
}
