//! The serial port: the kernel's diagnostic channel of last resort.
//!
//! Serial works before paging, before the heap, before any driver, and before
//! anything that could render a glyph. It is the only output that is reliable
//! at the moment the kernel is least able to explain itself, which is exactly
//! when output matters most.

use core::fmt::{self, Write};
use spin::Mutex;
use uart_16550::SerialPort;

/// COM1. The port the firmware and every emulator agree on.
const COM1: u16 = 0x3F8;

/// The port, once initialised.
///
/// `None` until [`init`] runs, so that a log call made too early is a dropped
/// message rather than a write to an uninitialised device.
static SERIAL: Mutex<Option<SerialPort>> = Mutex::new(None);

/// Brings up COM1.
///
/// Safe to call more than once; later calls do nothing.
pub fn init() {
    let mut guard = SERIAL.lock();
    if guard.is_some() {
        return;
    }
    // SAFETY: COM1 is at the architecturally fixed I/O port 0x3F8 on every
    // x86 machine LokoOS supports, and this is the only code in the kernel that
    // touches it.
    let mut port = unsafe { SerialPort::new(COM1) };
    port.init();
    *guard = Some(port);
}

/// Writes formatted output to the serial port.
///
/// Interrupts are disabled for the duration. Without that, an interrupt handler
/// that logs while this lock is held would spin forever waiting for a lock that
/// only the interrupted code can release — a deadlock that appears as a silent
/// hang, which is the worst possible failure for a diagnostic channel.
pub fn write_fmt(args: fmt::Arguments<'_>) {
    x86_64::instructions::interrupts::without_interrupts(|| {
        if let Some(port) = SERIAL.lock().as_mut() {
            // A failed write to a diagnostic channel has nowhere to be
            // reported, so it is deliberately dropped rather than escalated.
            let _ = port.write_fmt(args);
        }
    });
}

/// Writes to the serial port without taking the lock.
///
/// # Safety
///
/// The caller must be certain that no other context can be inside [`write_fmt`]
/// — in practice, only the panic handler after all other cores are stopped.
/// This exists so that a panic *while holding the serial lock* still produces
/// output instead of hanging.
pub unsafe fn write_fmt_unlocked(args: fmt::Arguments<'_>) {
    // SAFETY: the caller guarantees exclusive access; see above.
    let mut port = unsafe { SerialPort::new(COM1) };
    let _ = port.write_fmt(args);
}
