//! Kernel logging.
//!
//! Structured enough to be filtered and machine-read (requirement 63), small
//! enough to work before the heap exists. Every line carries a level and a
//! subsystem, so `LKO/Logs/kernel.log` can be filtered without parsing prose.

use core::fmt;
use core::sync::atomic::{AtomicU8, Ordering};

/// How important a message is.
///
/// Every level is reachable through the macros below and through
/// `Settings → Developer → Logs`; the compiler cannot see that, because the
/// macros are what construct them.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Level {
    /// Detail useful only when chasing a specific problem.
    Trace = 0,
    /// Normal internal progress.
    Debug = 1,
    /// Something a user or administrator would want to know.
    Info = 2,
    /// Something is wrong but the system is continuing.
    Warn = 3,
    /// Something failed.
    Error = 4,
}

impl Level {
    /// A fixed-width tag, so that log lines align in a terminal.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

/// The current threshold. Messages below it are discarded.
static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Sets the minimum level that will be emitted.
pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Whether a message at `level` would be emitted.
///
/// Checked before formatting, so a filtered-out log line costs one atomic load
/// rather than a full `format_args!` evaluation.
#[must_use]
pub fn enabled(level: Level) -> bool {
    level as u8 >= LEVEL.load(Ordering::Relaxed)
}

/// Emits one log line. Called by the [`log`] macro; not meant to be used
/// directly.
pub fn emit(level: Level, subsystem: &str, args: fmt::Arguments<'_>) {
    crate::serial::write_fmt(format_args!(
        "[{}] {:<10} {}\n",
        level.tag(),
        subsystem,
        args
    ));
}

/// Writes a log line.
///
/// ```ignore
/// log!(Level::Info, "memory", "{} MiB usable", mib);
/// ```
#[macro_export]
macro_rules! log {
    ($level:expr, $subsystem:expr, $($arg:tt)*) => {{
        let level = $level;
        if $crate::log::enabled(level) {
            $crate::log::emit(level, $subsystem, format_args!($($arg)*));
        }
    }};
}

/// Logs at [`Level::Info`].
#[macro_export]
macro_rules! info {
    ($subsystem:expr, $($arg:tt)*) => {
        $crate::log!($crate::log::Level::Info, $subsystem, $($arg)*)
    };
}

/// Logs at [`Level::Warn`].
#[macro_export]
macro_rules! warn {
    ($subsystem:expr, $($arg:tt)*) => {
        $crate::log!($crate::log::Level::Warn, $subsystem, $($arg)*)
    };
}

/// Logs at [`Level::Error`].
#[macro_export]
macro_rules! error {
    ($subsystem:expr, $($arg:tt)*) => {
        $crate::log!($crate::log::Level::Error, $subsystem, $($arg)*)
    };
}

/// Logs at [`Level::Debug`].
#[macro_export]
macro_rules! debug {
    ($subsystem:expr, $($arg:tt)*) => {
        $crate::log!($crate::log::Level::Debug, $subsystem, $($arg)*)
    };
}
