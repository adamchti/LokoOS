//! System-call dispatch.
//!
//! The ABI is fully specified in `loko-abi`; this is the kernel side of it.
//!
//! ## Status
//!
//! **Dispatch and argument decoding are implemented. Almost every call returns
//! [`Error::NotImplemented`]**, because the subsystems behind them — the
//! scheduler, the storage stack, the address-space manager — do not exist yet.
//!
//! That is deliberate and visible. An unimplemented call returns a distinct
//! error that says so, rather than a plausible-looking success, so that
//! anything built on top of this fails immediately and obviously instead of
//! appearing to work.

// Nothing calls into this module yet: the `syscall` entry stub that would
// invoke `dispatch` needs the MSR setup and the per-thread kernel stack that
// come with the scheduler. The dispatch logic is written and reviewable now so
// that the entry stub, when it lands, has something correct to call.
#![allow(dead_code)]

use loko_abi::{Error, RawReturn, SyscallNo};

/// The six argument registers, as the entry stub captured them.
#[derive(Clone, Copy, Debug, Default)]
pub struct Args {
    /// First argument. For most calls, a handle.
    pub a0: u64,
    /// Second argument.
    pub a1: u64,
    /// Third argument.
    pub a2: u64,
    /// Fourth argument.
    pub a3: u64,
    /// Fifth argument.
    pub a4: u64,
    /// Sixth argument.
    pub a5: u64,
}

/// Dispatches one system call.
///
/// Returns the raw value to place in the return register. Never panics: a
/// malformed call from userland is an error code, not a kernel fault.
#[must_use]
pub fn dispatch(number: u16, args: Args) -> RawReturn {
    let Some(call) = SyscallNo::from_number(number) else {
        // An unknown number is the most likely shape of a hostile or corrupt
        // call, so it is rejected before anything looks at the arguments.
        return RawReturn::err(Error::NoSuchSyscall);
    };

    let result = match call {
        SyscallNo::LogWrite => log_write(args),
        SyscallNo::ClockMonotonic => clock_monotonic(),
        // Everything else is specified but not yet built.
        _ => Err(Error::NotImplemented),
    };

    result.into()
}

/// `LogWrite`: append a line to the system log on the caller's behalf.
///
/// Not yet implemented: it needs a way to read the caller's memory safely,
/// which needs an address-space manager.
fn log_write(_args: Args) -> Result<usize, Error> {
    Err(Error::NotImplemented)
}

/// `ClockMonotonic`: nanoseconds since boot.
///
/// Not yet implemented: it needs a calibrated timer source, which needs ACPI
/// parsing and either the TSC's invariant frequency or the HPET.
fn clock_monotonic() -> Result<usize, Error> {
    Err(Error::NotImplemented)
}
