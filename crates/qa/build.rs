// Compiles the Cocoa shim on macOS. clang and ar come with the Xcode command line tools.
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = std::env::var("OUT_DIR").unwrap();
    let src = "src/platform/macos/shim.m";
    println!("cargo:rerun-if-changed={src}");
    let obj = format!("{out}/shim.o");
    // Cross builds (Intel to Apple silicon and back) need the target's name for clang.
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64".to_string(),
        Ok(a) => a.to_string(),
        Err(_) => "x86_64".to_string(),
    };
    let ok = Command::new("clang").args(["-c", "-arch", &arch, "-fobjc-arc", "-O2", "-Wall", "-mmacosx-version-min=11.0", src, "-o", &obj]).status().map(|s| s.success()).unwrap_or(false);
    assert!(ok, "clang failed to compile {src}");
    let lib = format!("{out}/libqashim.a");
    let ok = Command::new("ar").args(["crs", &lib, &obj]).status().map(|s| s.success()).unwrap_or(false);
    assert!(ok, "ar failed");
    println!("cargo:rustc-link-search=native={out}");
    println!("cargo:rustc-link-lib=static=qashim");
    for f in ["Cocoa", "QuartzCore", "CoreFoundation"] {
        println!("cargo:rustc-link-lib=framework={f}");
    }
}
