//! Booting a LokoOS ISO under QEMU and checking that it came up.
//!
//! This is the test that matters most, because it is the only one that
//! exercises the code no unit test can reach: the bootloader finding the
//! kernel, loading it, building page tables, switching `cr3`, and the kernel
//! surviving on the other side.
//!
//! ## How success is decided
//!
//! Not by the exit status. The kernel halts rather than exiting, so QEMU runs
//! until it is stopped. Success is decided by what appears on the serial port:
//! the kernel narrates each boot stage, and this waits for the line it prints
//! once initialisation is complete.
//!
//! Failure is decided the same way, and deliberately covers more than "the
//! success line never arrived". A kernel that panics, faults, or refuses its
//! boot info says so on the serial port, and matching those lines turns a
//! sixty-second timeout into an immediate, specific report.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Lines the kernel prints on the way up, in the order it prints them.
///
/// Reporting the last one reached is what makes a failed boot diagnosable
/// rather than just red: "got to stage 7" localises the problem to one
/// subsystem.
const BOOT_STAGES: &[(&str, &str)] = &[
    ("LokoOS kernel", "kernel entry reached"),
    ("stage 5: kernel entered", "boot info accepted"),
    ("stage 6: descriptor tables", "GDT, TSS and IDT loaded"),
    ("stage 7: physical memory", "frame allocator built"),
    ("stage 8: kernel heap", "heap initialised"),
];

/// The line that means the kernel finished initialising.
const SUCCESS_MARKER: &str = "kernel initialised successfully and is halting";

/// Lines that mean the boot has already failed, with what to say about each.
const FAILURE_MARKERS: &[(&str, &str)] = &[
    ("*** LokoOS kernel panic ***", "the kernel panicked"),
    (
        "*** LokoOS stopped ***",
        "the kernel hit a fault it could not continue from",
    ),
    (
        "LokoOS could not start",
        "the kernel rejected what the bootloader handed it",
    ),
    ("double fault", "a fault occurred while handling a fault"),
];

/// How long to wait before giving up.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);

/// Candidate OVMF firmware pairs, newest layout first.
///
/// Distributions disagree about where OVMF lives and whether it is split into
/// code and variable stores. Probing is shorter than documenting the matrix.
const OVMF_CANDIDATES: &[(&str, &str)] = &[
    (
        "/usr/share/OVMF/OVMF_CODE_4M.fd",
        "/usr/share/OVMF/OVMF_VARS_4M.fd",
    ),
    (
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/OVMF/OVMF_VARS.fd",
    ),
    (
        "/usr/share/edk2/ovmf/OVMF_CODE.fd",
        "/usr/share/edk2/ovmf/OVMF_VARS.fd",
    ),
    (
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
        "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
    ),
    (
        "/usr/share/qemu/edk2-x86_64-code.fd",
        "/usr/share/qemu/edk2-i386-vars.fd",
    ),
];

/// Where the UEFI firmware lives.
struct Firmware {
    code: PathBuf,
    vars: PathBuf,
}

/// Finds OVMF, or explains how to install it.
///
/// `LOKO_OVMF_CODE` and `LOKO_OVMF_VARS` override the search, for a machine
/// with firmware somewhere unusual.
fn find_firmware() -> Result<Firmware, String> {
    if let (Ok(code), Ok(vars)) = (
        std::env::var("LOKO_OVMF_CODE"),
        std::env::var("LOKO_OVMF_VARS"),
    ) {
        return Ok(Firmware {
            code: PathBuf::from(code),
            vars: PathBuf::from(vars),
        });
    }

    for (code, vars) in OVMF_CANDIDATES {
        let (code, vars) = (Path::new(code), Path::new(vars));
        if code.exists() && vars.exists() {
            return Ok(Firmware {
                code: code.to_path_buf(),
                vars: vars.to_path_buf(),
            });
        }
    }

    Err("could not find OVMF, the UEFI firmware QEMU needs.\n\n\
         On Debian or Ubuntu:  sudo apt-get install -y ovmf\n\
         On Fedora:            sudo dnf install -y edk2-ovmf\n\
         On Arch:              sudo pacman -S edk2-ovmf\n\n\
         Set LOKO_OVMF_CODE and LOKO_OVMF_VARS to point at it directly if it is\n\
         installed somewhere this does not look."
        .to_string())
}

/// What the boot produced.
pub struct Outcome {
    /// Everything the kernel wrote to the serial port.
    pub serial: String,
    /// How far up the boot sequence it got.
    pub stages_reached: usize,
    /// Whether the success marker appeared.
    pub succeeded: bool,
    /// The failure marker that stopped it, if one did.
    pub failure: Option<&'static str>,
}

/// Boots `iso` under QEMU and waits for the kernel to finish initialising.
pub fn boot_test(iso: &Path, out_dir: &Path, timeout: Duration) -> Result<Outcome, String> {
    if !command_exists("qemu-system-x86_64") {
        return Err("qemu-system-x86_64 is not on PATH.\n\n\
                    On Debian or Ubuntu:  sudo apt-get install -y qemu-system-x86\n\
                    On Fedora:            sudo dnf install -y qemu-system-x86\n\
                    On macOS:             brew install qemu"
            .to_string());
    }
    let firmware = find_firmware()?;

    fs::create_dir_all(out_dir)
        .map_err(|e| format!("could not create {}: {e}", out_dir.display()))?;

    // OVMF writes to its variable store, so it needs a private writable copy.
    // Pointing QEMU at the system one would either fail on a read-only file or
    // leave the machine with modified firmware variables.
    let vars = out_dir.join("OVMF_VARS.local.fd");
    fs::copy(&firmware.vars, &vars).map_err(|e| {
        format!(
            "could not copy firmware variables from {}: {e}",
            firmware.vars.display()
        )
    })?;

    let serial_log = out_dir.join("serial.log");
    let _ = fs::remove_file(&serial_log);

    println!("  firmware  {}", firmware.code.display());
    println!("  image     {}", iso.display());
    println!("  serial    {}", serial_log.display());

    let mut child = spawn_qemu(iso, &firmware.code, &vars, &serial_log)?;
    let outcome = watch(&mut child, &serial_log, timeout);

    // The kernel halts rather than exiting, so QEMU is always still running.
    let _ = child.kill();
    let _ = child.wait();

    outcome
}

fn command_exists(binary: &str) -> bool {
    Command::new(binary)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn spawn_qemu(iso: &Path, code: &Path, vars: &Path, serial_log: &Path) -> Result<Child, String> {
    let mut command = Command::new("qemu-system-x86_64");
    command
        .args(["-machine", "q35"])
        // `max` rather than the default model: the bootloader sets EFER.NXE and
        // maps data pages no-execute, and a CPU model without NX would fault on
        // the first such mapping for a reason that has nothing to do with the
        // code being tested.
        .args(["-cpu", "max"])
        .args(["-m", "512M"])
        .args([
            "-drive",
            &format!(
                "if=pflash,format=raw,unit=0,readonly=on,file={}",
                code.display()
            ),
        ])
        .args([
            "-drive",
            &format!("if=pflash,format=raw,unit=1,file={}", vars.display()),
        ])
        .args([
            "-drive",
            &format!("file={},format=raw,media=cdrom,readonly=on", iso.display()),
        ])
        .args(["-serial", &format!("file:{}", serial_log.display())])
        .args(["-display", "none"])
        // A triple fault should end the run, not silently restart it and make
        // the serial log look like two boots.
        .arg("-no-reboot")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    command
        .spawn()
        .map_err(|e| format!("could not start QEMU: {e}"))
}

/// Polls the serial log until the kernel succeeds, fails, or time runs out.
fn watch(child: &mut Child, serial_log: &Path, timeout: Duration) -> Result<Outcome, String> {
    let started = Instant::now();
    let mut reported = 0usize;

    loop {
        let serial = fs::read_to_string(serial_log).unwrap_or_default();

        // Report progress as it happens, so a hung boot still shows how far it
        // got before the log stopped moving.
        let reached = BOOT_STAGES
            .iter()
            .take_while(|(marker, _)| serial.contains(marker))
            .count();
        while reported < reached {
            println!("  ok  {}", BOOT_STAGES[reported].1);
            reported += 1;
        }

        if serial.contains(SUCCESS_MARKER) {
            return Ok(Outcome {
                serial,
                stages_reached: reached,
                succeeded: true,
                failure: None,
            });
        }

        if let Some((_, description)) = FAILURE_MARKERS
            .iter()
            .find(|(marker, _)| serial.contains(marker))
        {
            return Ok(Outcome {
                serial,
                stages_reached: reached,
                succeeded: false,
                failure: Some(description),
            });
        }

        // QEMU exiting on its own means the guest triple-faulted or the
        // firmware gave up. Read the log once more before reporting.
        if let Ok(Some(status)) = child.try_wait() {
            let serial = fs::read_to_string(serial_log).unwrap_or_default();
            return Ok(Outcome {
                serial,
                stages_reached: reached,
                succeeded: false,
                failure: Some(if status.success() {
                    "QEMU exited before the kernel finished starting"
                } else {
                    "QEMU exited with an error, which usually means a triple fault"
                }),
            });
        }

        if started.elapsed() > timeout {
            return Ok(Outcome {
                serial,
                stages_reached: reached,
                succeeded: false,
                failure: Some("timed out with no further output"),
            });
        }

        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Runs the boot test and prints a verdict. Returns an error if it did not boot.
pub fn run(iso: &Path, out_dir: &Path) -> Result<(), String> {
    let timeout = std::env::var("LOKO_BOOT_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_TIMEOUT);

    println!("\nBooting LokoOS under QEMU");
    let outcome = boot_test(iso, out_dir, timeout)?;

    println!("\n--- serial output ---");
    if outcome.serial.trim().is_empty() {
        println!("(nothing: the kernel never reached its first log line)");
    } else {
        for line in outcome.serial.lines() {
            println!("  {line}");
        }
    }
    println!("--- end of serial output ---\n");

    if outcome.succeeded {
        println!(
            "LokoOS booted. All {} stages reached, and the kernel finished\n\
             initialising and halted as designed.",
            BOOT_STAGES.len()
        );
        return Ok(());
    }

    let reached = if outcome.stages_reached == 0 {
        "nothing at all".to_string()
    } else {
        format!(
            "{} ({} of {} stages)",
            BOOT_STAGES[outcome.stages_reached - 1].1,
            outcome.stages_reached,
            BOOT_STAGES.len()
        )
    };

    Err(format!(
        "LokoOS did not finish booting.\n\
         \x20 Reached: {reached}\n\
         \x20 Stopped: {}\n\n\
         The serial output above is everything the kernel managed to say.",
        outcome.failure.unwrap_or("for an unknown reason")
    ))
}
