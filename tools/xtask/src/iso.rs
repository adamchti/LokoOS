//! Building a bootable LokoOS ISO.
//!
//! The output is a hybrid image: an ISO 9660 filesystem with an El Torito EFI
//! boot entry, and the same FAT EFI system partition appended as a GPT
//! partition. Firmware booting from optical media finds it through El Torito;
//! firmware booting from a USB stick written with `dd` finds it through the
//! partition table. One file, both paths.
//!
//! There is no BIOS or legacy boot entry. LokoOS is UEFI-only (requirement 6),
//! and an image that advertises a boot path it does not have is worse than one
//! that does not advertise it.
//!
//! ## External tools
//!
//! This needs `mkfs.vfat`, `mtools` and `xorriso`. Writing FAT and ISO 9660
//! encoders inside the build driver would be a lot of code with no payoff:
//! both formats are frozen, and these tools are the reference implementations.
//! When they are missing, [`check_tools`] says exactly which ones and how to
//! get them, rather than failing somewhere deep inside a pipeline.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::budget::human_size;

/// The volume label. Uppercase and short, because ISO 9660 and FAT both care.
const VOLUME_LABEL: &str = "LOKOOS";

/// Slack added to the EFI system partition beyond the payload.
///
/// Leaves room for a boot configuration file and a recovery kernel later
/// without changing the image layout.
const ESP_SLACK_BYTES: u64 = 6 * 1024 * 1024;

/// The smallest EFI system partition worth making.
const ESP_MINIMUM_BYTES: u64 = 16 * 1024 * 1024;

/// A tool this module shells out to.
struct Tool {
    /// The executable name.
    binary: &'static str,
    /// The distribution package that provides it.
    package: &'static str,
}

const REQUIRED_TOOLS: &[Tool] = &[
    Tool {
        binary: "mkfs.vfat",
        package: "dosfstools",
    },
    Tool {
        binary: "mmd",
        package: "mtools",
    },
    Tool {
        binary: "mcopy",
        package: "mtools",
    },
    Tool {
        binary: "xorriso",
        package: "xorriso",
    },
];

/// Fails with an actionable message if any required tool is missing.
pub fn check_tools() -> Result<(), String> {
    let missing: Vec<&Tool> = REQUIRED_TOOLS
        .iter()
        .filter(|t| !tool_present(t.binary))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }

    let names: Vec<&str> = missing.iter().map(|t| t.binary).collect();
    let mut packages: Vec<&str> = missing.iter().map(|t| t.package).collect();
    packages.sort_unstable();
    packages.dedup();

    Err(format!(
        "building an ISO needs these tools, which are not on PATH: {}\n\n\
         On Debian or Ubuntu:  sudo apt-get install -y {}\n\
         On Fedora:            sudo dnf install -y {}\n\
         On macOS:             brew install {}\n\n\
         None of them exist for Windows in a usable form, which is why LokoOS\n\
         builds its ISO in CI. Push the branch and the `iso` workflow produces\n\
         one; see .github/workflows/iso.yml.",
        names.join(", "),
        packages.join(" "),
        packages.join(" "),
        packages.join(" "),
    ))
}

fn tool_present(binary: &str) -> bool {
    // `--help` rather than `--version`: not every mtools command implements
    // `--version`, but all of them respond to `--help`. The exit status is
    // ignored; what matters is whether the process could be started at all.
    Command::new(binary)
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Runs a command, turning a non-zero exit into a readable error.
fn run(binary: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(binary)
        .args(args)
        // mtools refuses to touch an image whose sector count is not a whole
        // number of tracks, which is every image not built to a 1980s floppy
        // geometry. The check protects real floppy drives from a class of
        // problem that has not existed for decades; for a file on disk it is
        // noise, and every distribution build script disables it.
        .env("MTOOLS_SKIP_CHECK", "1")
        .output()
        .map_err(|e| format!("could not run `{binary}`: {e}"))?;

    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "`{} {}` failed ({})\n{}{}",
        binary,
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim(),
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

/// Total size of every file under `dir`.
fn tree_size(dir: &Path) -> Result<u64, String> {
    let mut total = 0;
    for entry in fs::read_dir(dir).map_err(|e| format!("could not read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("could not read {}: {e}", dir.display()))?;
        let meta = entry
            .metadata()
            .map_err(|e| format!("could not stat {}: {e}", entry.path().display()))?;
        total += if meta.is_dir() {
            tree_size(&entry.path())?
        } else {
            meta.len()
        };
    }
    Ok(total)
}

/// Copies `dir` into the FAT image at `fat_path`, under the FAT path `prefix`.
///
/// Directories are created before their contents, because `mcopy` will not
/// create a parent directory for you.
fn copy_tree_into_fat(fat_path: &Path, dir: &Path, prefix: &str) -> Result<(), String> {
    let image = fat_path.to_string_lossy().to_string();

    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("could not read {}: {e}", dir.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    // Deterministic order, so two builds of the same tree produce byte-identical
    // images. A build that is not reproducible cannot be audited.
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let target = format!("{prefix}/{name}");
        let path = entry.path();

        if path.is_dir() {
            run("mmd", &["-i", &image, &target])?;
            copy_tree_into_fat(fat_path, &path, &target)?;
        } else {
            let source = path.to_string_lossy().to_string();
            run("mcopy", &["-i", &image, &source, &target])?;
        }
    }
    Ok(())
}

/// Builds `esp.img`: a FAT EFI system partition holding `esp_tree`.
fn build_esp_image(esp_tree: &Path, out_dir: &Path) -> Result<PathBuf, String> {
    let payload = tree_size(esp_tree)?;
    let size = (payload * 2 + ESP_SLACK_BYTES).max(ESP_MINIMUM_BYTES);
    // FAT wants a whole number of sectors and xorriso wants the appended
    // partition aligned; rounding to 1 MiB satisfies both.
    let size = size.div_ceil(1024 * 1024) * 1024 * 1024;

    let path = out_dir.join("esp.img");
    // `mkfs.vfat -C` creates the file and refuses to overwrite one that is
    // already there, so a second run in the same build directory would fail.
    // Removing it first makes the command idempotent, which matters because
    // `boot-test` builds the ISO again on top of whatever `iso` left behind.
    let _ = fs::remove_file(&path);
    let image = path.to_string_lossy().to_string();
    let blocks = (size / 1024).to_string();

    run(
        "mkfs.vfat",
        &[
            // FAT16, not FAT32. FAT32 needs at least 65,525 clusters, which
            // would force this image to about 33 MiB to carry a 2 MiB payload.
            // UEFI requires firmware to support FAT12, FAT16 and FAT32 on
            // removable media, and every distribution ships a FAT16 El Torito
            // EFI image for exactly this reason.
            "-F",
            "16",
            "-n",
            VOLUME_LABEL,
            // A fixed volume ID, which mkfs.fat requires to be hexadecimal.
            // Without it the current time is stamped in and two builds of
            // identical inputs would differ.
            "-i",
            "10C0050F",
            "-C",
            &image,
            &blocks,
        ],
    )?;

    copy_tree_into_fat(&path, esp_tree, "::")?;

    println!(
        "  EFI system partition  {} image, {} payload, FAT16",
        human_size(size),
        human_size(payload)
    );
    Ok(path)
}

/// Builds the ISO. Returns its path.
pub fn build(esp_tree: &Path, out_dir: &Path, version: &str) -> Result<PathBuf, String> {
    check_tools()?;

    fs::create_dir_all(out_dir)
        .map_err(|e| format!("could not create {}: {e}", out_dir.display()))?;

    let esp_image = build_esp_image(esp_tree, out_dir)?;

    // The staging tree becomes the ISO 9660 filesystem. It carries the EFI
    // system partition under boot/, which is what firmware boots, and a plain
    // copy of the EFI directory, so that a mounted ISO shows something a person
    // can inspect rather than one opaque blob.
    let iso_root = out_dir.join("iso_root");
    if iso_root.exists() {
        fs::remove_dir_all(&iso_root)
            .map_err(|e| format!("could not clear {}: {e}", iso_root.display()))?;
    }
    copy_tree(esp_tree, &iso_root)?;

    let staged_esp = iso_root.join("boot/esp.img");
    fs::create_dir_all(staged_esp.parent().expect("boot/ has a parent"))
        .map_err(|e| format!("could not create the boot directory: {e}"))?;
    fs::copy(&esp_image, &staged_esp)
        .map_err(|e| format!("could not stage the EFI system partition: {e}"))?;

    let iso = out_dir.join(format!("lokoos-{version}-x86_64.iso"));
    // Same reason as esp.img: a rebuild must not trip over the last one.
    let _ = fs::remove_file(&iso);
    let iso_string = iso.to_string_lossy().to_string();
    let root_string = iso_root.to_string_lossy().to_string();

    run(
        "xorriso",
        &[
            "-as",
            "mkisofs",
            // Rock Ridge and Joliet, so long filenames survive on Linux and on
            // Windows when the ISO is mounted rather than booted.
            "-R",
            "-J",
            "-joliet-long",
            "-V",
            VOLUME_LABEL,
            // The El Torito EFI boot entry. `--efi-boot` expands to an
            // alternative boot entry with no emulation, which is what UEFI
            // firmware looks for on optical media.
            "--efi-boot",
            "boot/esp.img",
            // And the same image again as a real partition, so that writing the
            // ISO to a USB stick with `dd` produces something firmware will
            // boot. `--efi-boot-image` means "use the El Torito image as the
            // partition contents" rather than storing a second copy.
            "-efi-boot-part",
            "--efi-boot-image",
            // A protective MBR, which is what puts the 0x55AA signature in the
            // first sector. Without it the image boots from optical media and
            // silently does not boot from USB.
            "--protective-msdos-label",
            "-o",
            &iso_string,
            &root_string,
        ],
    )?;

    verify(&iso)?;
    Ok(iso)
}

/// Recursively copies a directory.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("could not create {}: {e}", to.display()))?;
    for entry in
        fs::read_dir(from).map_err(|e| format!("could not read {}: {e}", from.display()))?
    {
        let entry = entry.map_err(|e| format!("could not read {}: {e}", from.display()))?;
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &destination)?;
        } else {
            fs::copy(&source, &destination).map_err(|e| {
                format!(
                    "could not copy {} to {}: {e}",
                    source.display(),
                    destination.display()
                )
            })?;
        }
    }
    Ok(())
}

/// Checks that what came out is actually a bootable ISO.
///
/// Cheap, and it catches the case where xorriso reports success but writes
/// something unusable. The primary volume descriptor lives in sector 16 and
/// starts with the string `CD001`.
fn verify(iso: &Path) -> Result<(), String> {
    let data = fs::read(iso).map_err(|e| format!("could not read {}: {e}", iso.display()))?;

    const PVD_OFFSET: usize = 16 * 2048;
    if data.len() < PVD_OFFSET + 6 {
        return Err(format!(
            "{} is too small to be an ISO ({} bytes)",
            iso.display(),
            data.len()
        ));
    }
    if &data[PVD_OFFSET + 1..PVD_OFFSET + 6] != b"CD001" {
        return Err(format!(
            "{} has no ISO 9660 volume descriptor: xorriso reported success but produced something else",
            iso.display()
        ));
    }
    // A hybrid image carries a partition table in the first sector. Without it
    // the ISO boots from optical media but not from a USB stick, which is a
    // silent half-failure worth catching here rather than in someone's hands.
    if data[510] != 0x55 || data[511] != 0xAA {
        return Err(format!(
            "{} has no partition table signature, so it would not boot from USB",
            iso.display()
        ));
    }

    println!("  verified: ISO 9660 volume descriptor and partition table signature both present");
    Ok(())
}
