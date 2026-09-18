//! # The LokoOS build driver
//!
//! `cargo xtask <command>`. Everything a contributor needs to do to this
//! repository, in one place, so that CI and a laptop run the same commands.
//!
//! Run `cargo xtask help` for the list.
//!
//! ## Why this exists rather than a shell script
//!
//! Three targets are in play — the host, `x86_64-unknown-none` for the kernel,
//! and `x86_64-unknown-uefi` for the bootloader — and each needs different
//! flags. A driver that knows which crate belongs to which target is shorter
//! and harder to get wrong than three near-identical command lines that drift.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod budget;
mod iso;
mod qemu;
mod toolchain;

/// Crates that are freestanding and must never be built for the host.
const BARE_METAL_CRATES: &[&str] = &["loko-kernel", "loko-boot"];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let (command, flags) = match args.split_first() {
        Some((c, rest)) => (c.as_str(), rest),
        None => ("help", &[] as &[String]),
    };
    let release = flags.iter().any(|f| f == "--release");

    let result = match command {
        "build" => build_all(release),
        "kernel" => build_kernel(release).map(|_| ()),
        "bootloader" => build_bootloader(release).map(|_| ()),
        "image" => image(release).map(|_| ()),
        "iso" => build_iso(release).map(|_| ()),
        "boot-test" => boot_test(release),
        "size" => size(release),
        "test" => test(),
        "clippy" => clippy(),
        "fmt" => fmt(flags.iter().any(|f| f == "--check")),
        "check" => check_everything(),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!(
            "unknown command `{other}`. Run `cargo xtask help` for the list."
        )),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("\nxtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "\
LokoOS build driver

USAGE:
    cargo xtask <command> [--release]

COMMANDS:
    build        Build the kernel and the bootloader
    kernel       Build the kernel only
    bootloader   Build the bootloader only
    image        Build both and lay out an EFI system partition in build/esp
    iso          Build a bootable hybrid ISO in build/
    boot-test    Build the ISO and boot it under QEMU, checking the serial log
    size         Build in release and report against the 2 GB base-OS budget
    test         Run the host test suite
    clippy       Lint every crate, for its own target
    fmt          Format every crate (--check to verify without writing)
    check        test + clippy + fmt --check + build. What CI runs.
    help         This message

NOTES:
    The kernel and bootloader are cross-compiled and are excluded from host
    commands automatically. On a Windows machine without Visual Studio, the
    driver selects the GNU toolchain so that linking works without an external
    linker; set LOKO_TOOLCHAIN to override."
    );
}

/// The repository root, derived from this crate's location rather than from the
/// current directory, so `cargo xtask` works from any subdirectory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask lives at <root>/tools/xtask")
        .to_path_buf()
}

/// Runs a cargo command, echoing it first so that a failure can be reproduced
/// by copying the line out of the log.
fn cargo(args: &[&str]) -> Result<(), String> {
    let toolchain = toolchain::select()?;
    let root = workspace_root();

    let mut full: Vec<String> = Vec::new();
    if let Some(name) = &toolchain {
        full.push(format!("+{name}"));
    }
    full.extend(args.iter().map(|s| (*s).to_string()));

    println!("\n> cargo {}", full.join(" "));

    let status = Command::new("cargo")
        .args(&full)
        .current_dir(&root)
        .status()
        .map_err(|e| format!("could not run cargo: {e}. Is Rust installed and on PATH?"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", full.join(" ")))
    }
}

/// Flags shared by both bare-metal targets.
///
/// `build-std` is used unconditionally rather than relying on a precompiled
/// `rust-std` for these targets. rustup does ship one, but requiring it means a
/// contributor's first build fails with a rustup error instead of just working,
/// and `rust-src` is needed for the toolchain anyway.
const BUILD_STD: &[&str] = &[
    "-Zbuild-std=core,compiler_builtins,alloc",
    "-Zbuild-std-features=compiler-builtins-mem",
];

fn profile_args(release: bool) -> Vec<&'static str> {
    if release {
        vec!["--release"]
    } else {
        vec![]
    }
}

fn profile_dir(release: bool) -> &'static str {
    if release {
        "release"
    } else {
        "debug"
    }
}

fn build_kernel(release: bool) -> Result<PathBuf, String> {
    let mut args = vec![
        "build",
        "-p",
        "loko-kernel",
        "--target",
        "x86_64-unknown-none",
    ];
    args.extend(BUILD_STD);
    args.extend(profile_args(release));
    cargo(&args)?;

    let path = workspace_root()
        .join("target/x86_64-unknown-none")
        .join(profile_dir(release))
        .join("loko-kernel");
    if !path.exists() {
        return Err(format!("the kernel did not appear at {}", path.display()));
    }
    Ok(path)
}

fn build_bootloader(release: bool) -> Result<PathBuf, String> {
    let mut args = vec![
        "build",
        "-p",
        "loko-boot",
        "--target",
        "x86_64-unknown-uefi",
    ];
    args.extend(BUILD_STD);
    args.extend(profile_args(release));
    cargo(&args)?;

    let path = workspace_root()
        .join("target/x86_64-unknown-uefi")
        .join(profile_dir(release))
        .join("loko-boot.efi");
    if !path.exists() {
        return Err(format!(
            "the bootloader did not appear at {}",
            path.display()
        ));
    }
    Ok(path)
}

fn build_all(release: bool) -> Result<(), String> {
    let bootloader = build_bootloader(release)?;
    let kernel = build_kernel(release)?;
    println!("\nbootloader  {}", describe(&bootloader));
    println!("kernel      {}", describe(&kernel));
    Ok(())
}

fn describe(path: &Path) -> String {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    format!("{}  ({})", path.display(), budget::human_size(size))
}

/// Lays out an EFI system partition tree, returning its path.
///
/// This produces a *directory*, not a disk image. [`build_iso`] turns it into
/// one. Copying the tree onto an already-formatted FAT partition is enough to
/// boot on real firmware, which is why it stays a step of its own.
fn image(release: bool) -> Result<PathBuf, String> {
    let bootloader = build_bootloader(release)?;
    let kernel = build_kernel(release)?;

    let esp = workspace_root().join("build/esp");
    let boot_dir = esp.join("EFI/BOOT");
    let loko_dir = esp.join("EFI/LOKO");

    for dir in [&boot_dir, &loko_dir] {
        fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }

    // BOOTX64.EFI is the name firmware looks for when nothing is registered in
    // NVRAM, which is the case on a freshly installed machine and in every
    // emulator.
    let targets = [
        (&bootloader, boot_dir.join("BOOTX64.EFI")),
        (&bootloader, loko_dir.join("loko-boot.efi")),
        (&kernel, loko_dir.join("loko-kernel")),
    ];
    for (source, destination) in &targets {
        fs::copy(source, destination).map_err(|e| {
            format!(
                "could not copy {} to {}: {e}",
                source.display(),
                destination.display()
            )
        })?;
    }

    println!("\nEFI system partition laid out at {}", esp.display());
    println!("  EFI/BOOT/BOOTX64.EFI   the bootloader, under the name firmware looks for");
    println!("  EFI/LOKO/loko-boot.efi the same binary, under its own name");
    println!("  EFI/LOKO/loko-kernel   the kernel");
    println!(
        "\nThis is a directory tree, not a disk image. `cargo xtask iso` turns it\n\
         into a bootable hybrid ISO; copying it onto an already-formatted EFI\n\
         system partition works too."
    );
    Ok(esp)
}

/// Builds a bootable hybrid ISO from the EFI system partition tree.
fn build_iso(release: bool) -> Result<PathBuf, String> {
    let esp = image(release)?;
    let out_dir = workspace_root().join("build");

    println!("\nBuilding a bootable ISO");
    let iso = iso::build(&esp, &out_dir, env!("CARGO_PKG_VERSION"))?;

    let size = fs::metadata(&iso).map(|m| m.len()).unwrap_or(0);
    println!(
        "\nISO written to {}  ({})",
        iso.display(),
        budget::human_size(size)
    );
    println!(
        "\nBoot it with:\n\
         \x20   qemu-system-x86_64 -cpu max -m 512M -cdrom {} -serial stdio -display none\n\
         or write it to a USB stick with dd. It is a hybrid image, so both work.",
        iso.display()
    );
    Ok(iso)
}

/// Builds the ISO and boots it under QEMU, failing if the kernel does not come
/// up.
///
/// Kept out of `check` on purpose: it needs QEMU and OVMF, which most
/// development machines do not have, and a gate that cannot run locally is a
/// gate people learn to ignore. CI runs it on every push.
fn boot_test(release: bool) -> Result<(), String> {
    let iso = build_iso(release)?;
    qemu::run(&iso, &workspace_root().join("build/boot-test"))
}

fn size(release: bool) -> Result<(), String> {
    let bootloader = build_bootloader(release)?;
    let kernel = build_kernel(release)?;
    budget::report(&[("bootloader", &bootloader), ("kernel", &kernel)])
}

/// Host commands exclude the freestanding crates, which cannot link for the
/// host: they have no `main`, and they define their own panic handler and
/// allocator.
fn host_exclusions() -> Vec<String> {
    BARE_METAL_CRATES
        .iter()
        .flat_map(|c| ["--exclude".to_string(), (*c).to_string()])
        .collect()
}

fn test() -> Result<(), String> {
    let exclusions = host_exclusions();
    let mut args = vec!["test", "--workspace"];
    args.extend(exclusions.iter().map(String::as_str));
    cargo(&args)
}

fn clippy() -> Result<(), String> {
    let exclusions = host_exclusions();
    let mut args = vec!["clippy", "--workspace", "--all-targets"];
    args.extend(exclusions.iter().map(String::as_str));
    args.push("--");
    args.push("-Dwarnings");
    cargo(&args)?;

    // The freestanding crates get linted for their real targets, or their
    // `#[cfg]`-gated code is never looked at.
    for (package, target) in [
        ("loko-kernel", "x86_64-unknown-none"),
        ("loko-boot", "x86_64-unknown-uefi"),
    ] {
        let mut args = vec!["clippy", "-p", package, "--target", target];
        args.extend(BUILD_STD);
        args.push("--");
        args.push("-Dwarnings");
        cargo(&args)?;
    }
    Ok(())
}

fn fmt(check: bool) -> Result<(), String> {
    let mut args = vec!["fmt", "--all"];
    if check {
        args.push("--check");
    }
    cargo(&args)
}

fn check_everything() -> Result<(), String> {
    fmt(true)?;
    clippy()?;
    test()?;
    build_all(false)?;
    println!("\nEverything passed.");
    Ok(())
}
