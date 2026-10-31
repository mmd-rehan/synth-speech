//! ggml's Metal code checks the macOS version at run time when built for an
//! older macOS than the SDK. Those checks live in clang's runtime library, which
//! Rust does not link by default, so link it here.

use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let macos = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos");
    if !macos || env::var_os("CARGO_FEATURE_METAL").is_none() {
        return;
    }
    let output = Command::new("clang").arg("--print-runtime-dir").output();
    let Ok(output) = output else {
        println!("cargo:warning=clang was not found, so linking may fail");
        return;
    };
    let dir = String::from_utf8_lossy(&output.stdout);
    println!("cargo:rustc-link-search=native={}", dir.trim());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
