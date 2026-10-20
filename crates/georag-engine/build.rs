//! Compiles the build flavour into the binary: the target triple plus
//! `-cuda` or `-vulkan` when that feature is on. The updater selects release
//! assets by this string and never infers it from hardware.

use std::env;

fn main() {
    let target = env::var("TARGET").expect("cargo sets TARGET");
    let suffix = if env::var_os("CARGO_FEATURE_CUDA").is_some() {
        "-cuda"
    } else if env::var_os("CARGO_FEATURE_VULKAN").is_some() {
        "-vulkan"
    } else {
        ""
    };
    println!("cargo:rustc-env=GEORAG_FLAVOUR={target}{suffix}");
    println!("cargo:rerun-if-changed=build.rs");
}
