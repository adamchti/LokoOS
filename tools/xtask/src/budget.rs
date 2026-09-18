//! Measuring LokoOS against its size budget.
//!
//! Requirement 2 sets a target of under 2 GB for the base operating system, and
//! lists what that figure excludes: optional applications, the Windows and
//! macOS compatibility runtimes, AI models, user files, caches, recovery
//! images, developer toolchains and language packs.
//!
//! The only way that target means anything is if it is measured. This module
//! measures real build artefacts. It does not estimate, and it does not report
//! a figure for anything that has not been built — an unbuilt subsystem shows
//! as "not built yet", so the total is always honest about how much of the
//! system it covers.

use std::fs;
use std::path::Path;

/// The budget from requirement 2.
pub const BASE_OS_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Subsystems that count against the budget, and whether they exist yet.
///
/// Listed exhaustively, including the ones that do not exist, so that the
/// report shows how much of the base OS is actually accounted for rather than
/// implying that what has been built is all there is.
const BASE_OS_COMPONENTS: &[(&str, &str)] = &[
    ("bootloader", "built"),
    ("kernel", "built"),
    ("drivers", "not built yet"),
    ("core services", "not built yet"),
    ("graphics stack", "not built yet"),
    ("compositor", "not built yet"),
    ("window manager", "not built yet"),
    ("desktop shell", "not built yet"),
    ("Linder", "not built yet"),
    ("Lowser", "not built yet"),
    ("Loko AI runtime", "not built yet"),
    ("Loko Settings", "not built yet"),
    ("Loko Terminal", "not built yet"),
    ("Loko Store client", "not built yet"),
    ("Loko Update", "not built yet"),
    ("fonts and icons", "not built yet"),
];

/// Formats a byte count the way a person reads one.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("GiB", 1024 * 1024 * 1024),
        ("MiB", 1024 * 1024),
        ("KiB", 1024),
        ("bytes", 1),
    ];
    for (unit, scale) in UNITS {
        if bytes >= scale {
            if scale == 1 {
                return format!("{bytes} {unit}");
            }
            // One decimal place: enough to see a change, not enough to imply
            // precision the measurement does not have.
            let whole = bytes / scale;
            let tenths = (bytes % scale) * 10 / scale;
            return format!("{whole}.{tenths} {unit}");
        }
    }
    format!("{bytes} bytes")
}

/// Prints the size report.
pub fn report(artifacts: &[(&str, &Path)]) -> Result<(), String> {
    let mut measured = 0u64;

    println!("\nLokoOS base OS size, measured against the 2 GB target\n");
    println!("{:<22} {:>14}   Status", "Component", "Size");
    println!("{:<22} {:>14}   ------", "---------", "----");

    for (name, status) in BASE_OS_COMPONENTS {
        match artifacts.iter().find(|(a, _)| a == name) {
            Some((_, path)) => {
                let size = fs::metadata(path)
                    .map_err(|e| format!("could not measure {}: {e}", path.display()))?
                    .len();
                measured += size;
                println!("{name:<22} {:>14}   {status}", human_size(size));
            }
            None => println!("{name:<22} {:>14}   {status}", "-"),
        }
    }

    let built = BASE_OS_COMPONENTS
        .iter()
        .filter(|(_, s)| *s == "built")
        .count();
    let total = BASE_OS_COMPONENTS.len();

    println!("\n{:<22} {:>14}", "Measured total", human_size(measured));
    println!("{:<22} {:>14}", "Budget", human_size(BASE_OS_BUDGET_BYTES));
    println!(
        "{:<22} {:>13.2}%",
        "Budget used",
        measured as f64 / BASE_OS_BUDGET_BYTES as f64 * 100.0
    );
    println!(
        "\n{built} of {total} base-OS components exist. This figure covers only what has\n\
         been built; it is not a projection of the finished system's size."
    );
    println!(
        "\nExcluded from this budget, per requirement 2: optional applications, the\n\
         Windows and macOS compatibility runtimes, AI models, user files, browser and\n\
         application caches, recovery images, developer toolchains and language packs."
    );

    if measured > BASE_OS_BUDGET_BYTES {
        return Err("the measured base OS already exceeds the 2 GB budget".to_string());
    }
    Ok(())
}
