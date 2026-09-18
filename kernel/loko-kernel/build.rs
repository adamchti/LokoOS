//! Points the linker at `linker.ld` and re-runs when it changes.

use std::path::PathBuf;

fn main() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("linker.ld");

    println!("cargo:rustc-link-arg-bins=-T{}", script.display());
    // Without this, a stale link script silently keeps being used after an
    // edit, and the resulting image is wrong in ways that only show up at boot.
    println!("cargo:rerun-if-changed={}", script.display());
    println!("cargo:rerun-if-changed=build.rs");
}
