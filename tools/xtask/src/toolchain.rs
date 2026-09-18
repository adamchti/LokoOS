//! Picking a Rust toolchain that can actually link.
//!
//! On Windows the default host target is `x86_64-pc-windows-msvc`, which needs
//! `link.exe` and the Windows SDK from Visual Studio. Nothing in LokoOS is
//! Windows-specific — the host build exists only to run tests and this driver —
//! so on a machine without Visual Studio the right answer is to use the GNU
//! host target, where rustup's `rust-mingw` component supplies the import
//! libraries and `rust-lld` does the linking. The flags that make that work are
//! in `.cargo/config.toml`.
//!
//! Elsewhere, and on Windows machines that do have Visual Studio, this returns
//! `None` and the toolchain pinned in `rust-toolchain.toml` is used unchanged.

use std::env;
use std::process::{Command, Stdio};

/// The GNU-host toolchain used as a fallback on Windows.
const WINDOWS_GNU_TOOLCHAIN: &str = "nightly-x86_64-pc-windows-gnu";

/// The toolchain to pass to cargo as `+name`, or `None` for the pinned default.
pub fn select() -> Result<Option<String>, String> {
    // An explicit choice always wins, including the empty string to mean "use
    // the pinned default and stop guessing".
    if let Ok(name) = env::var("LOKO_TOOLCHAIN") {
        return Ok(if name.is_empty() { None } else { Some(name) });
    }

    if !cfg!(windows) {
        return Ok(None);
    }
    if msvc_linker_available() {
        return Ok(None);
    }
    if !toolchain_installed(WINDOWS_GNU_TOOLCHAIN) {
        return Err(format!(
            "this machine has no MSVC linker (link.exe), so the default Windows\n\
             toolchain cannot link anything, and the fallback toolchain is not installed.\n\n\
             Either install the Visual Studio Build Tools with the C++ workload, or run:\n\n\
             \x20   rustup toolchain install {WINDOWS_GNU_TOOLCHAIN} --profile minimal -c rust-src\n\n\
             Set LOKO_TOOLCHAIN to override this choice."
        ));
    }
    Ok(Some(WINDOWS_GNU_TOOLCHAIN.to_string()))
}

/// Whether `link.exe` can be executed.
///
/// Running it is a more honest test than looking for a directory: a Visual
/// Studio installation can be present but missing the C++ workload, in which
/// case the directory exists and the linker does not.
fn msvc_linker_available() -> bool {
    Command::new("link.exe")
        .arg("/?")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Whether rustup has the named toolchain.
///
/// `rustup toolchain list` annotates lines with parenthesised notes that have
/// changed spelling across rustup versions — `(default)`, `(active, default)`,
/// `(override)`. Only the first whitespace-delimited token is the name, so that
/// is all this compares.
fn toolchain_installed(name: &str) -> bool {
    let Ok(output) = Command::new("rustup").args(["toolchain", "list"]).output() else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .any(|token| token == name)
}
