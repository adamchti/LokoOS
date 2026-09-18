//! System-call numbers and their contracts.
//!
//! LokoOS keeps the syscall surface small on purpose. Every syscall is a piece
//! of the trusted computing base that can never be removed, and a small,
//! handle-oriented surface is one that can actually be audited. Higher-level
//! facilities — search, indexing, AI, the package manager — are userland
//! services reached over [`SyscallNo::ChannelSend`], not kernel entry points.

/// A LokoOS system-call number.
///
/// Numbers are permanently assigned. A removed syscall keeps its number
/// reserved and returns [`crate::Error::NoSuchSyscall`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
#[non_exhaustive]
pub enum SyscallNo {
    // -- Process and thread lifecycle -------------------------------------
    /// Terminate the calling thread. Never returns.
    ThreadExit = 1,
    /// Yield the remainder of this thread's time slice.
    ThreadYield = 2,
    /// Sleep for a number of nanoseconds.
    ThreadSleep = 3,
    /// Create a thread in the calling process.
    ThreadCreate = 4,

    // -- Handles ------------------------------------------------------------
    /// Close a handle. The object is destroyed when the last handle closes.
    HandleClose = 16,
    /// Create a new handle to the same object with equal or fewer rights.
    HandleDerive = 17,
    /// Read a handle's type and rights.
    HandleQuery = 18,

    // -- LKOFS --------------------------------------------------------------
    /// Open an object relative to a directory handle.
    ///
    /// There is no absolute-path form. The root directory handle is granted at
    /// process start according to the application manifest, and a process that
    /// was not granted one cannot reach the filesystem at all.
    FsOpenAt = 32,
    /// Read bytes from a file handle at an offset.
    FsRead = 33,
    /// Write bytes to a file handle at an offset.
    FsWrite = 34,
    /// Read metadata for a handle.
    FsStat = 35,
    /// List directory entries.
    FsReadDir = 36,
    /// Create a directory relative to a directory handle.
    FsCreateDir = 37,
    /// Unlink an entry relative to a directory handle.
    FsUnlink = 38,
    /// Rename within or between directory handles.
    FsRename = 39,
    /// Flush pending writes to stable storage.
    FsSync = 40,

    // -- Memory -------------------------------------------------------------
    /// Map anonymous memory into the calling address space.
    VmMap = 48,
    /// Unmap a range.
    VmUnmap = 49,
    /// Change the protection of a mapped range.
    VmProtect = 50,

    // -- IPC ----------------------------------------------------------------
    /// Create a connected pair of channel endpoints.
    ChannelCreate = 64,
    /// Send a message, optionally transferring handles with it.
    ChannelSend = 65,
    /// Receive a message.
    ChannelRecv = 66,

    // -- Time and events ----------------------------------------------------
    /// Read a monotonic clock in nanoseconds since boot.
    ClockMonotonic = 80,
    /// Wait until any of a set of handles is signalled.
    WaitMany = 81,

    // -- Diagnostics --------------------------------------------------------
    /// Write a line to the system log on behalf of the calling process.
    ///
    /// Available to every process without any capability, because a process
    /// that cannot report a problem cannot be debugged. Rate limited, and
    /// attributed to the caller so it cannot be used to forge system messages.
    LogWrite = 96,
}

impl SyscallNo {
    /// Every implemented or reserved syscall number, in ascending order.
    pub const ALL: &'static [SyscallNo] = &[
        SyscallNo::ThreadExit,
        SyscallNo::ThreadYield,
        SyscallNo::ThreadSleep,
        SyscallNo::ThreadCreate,
        SyscallNo::HandleClose,
        SyscallNo::HandleDerive,
        SyscallNo::HandleQuery,
        SyscallNo::FsOpenAt,
        SyscallNo::FsRead,
        SyscallNo::FsWrite,
        SyscallNo::FsStat,
        SyscallNo::FsReadDir,
        SyscallNo::FsCreateDir,
        SyscallNo::FsUnlink,
        SyscallNo::FsRename,
        SyscallNo::FsSync,
        SyscallNo::VmMap,
        SyscallNo::VmUnmap,
        SyscallNo::VmProtect,
        SyscallNo::ChannelCreate,
        SyscallNo::ChannelSend,
        SyscallNo::ChannelRecv,
        SyscallNo::ClockMonotonic,
        SyscallNo::WaitMany,
        SyscallNo::LogWrite,
    ];

    /// The numeric value passed in the syscall register.
    #[must_use]
    pub const fn number(self) -> u16 {
        self as u16
    }

    /// Decodes a raw syscall number.
    pub fn from_number(n: u16) -> Option<SyscallNo> {
        SyscallNo::ALL.iter().copied().find(|s| s.number() == n)
    }

    /// The rights the handle in argument 0 must carry for this call.
    ///
    /// `None` means the call takes no handle, or validates rights itself
    /// because they depend on the other arguments.
    pub const fn required_rights(self) -> Option<crate::HandleRights> {
        use crate::HandleRights as R;
        Some(match self {
            SyscallNo::HandleDerive => R::DERIVE,
            SyscallNo::HandleQuery => R::INSPECT,
            SyscallNo::FsOpenAt => R::EXECUTE,
            SyscallNo::FsRead => R::READ,
            SyscallNo::FsWrite => R::WRITE,
            SyscallNo::FsStat => R::INSPECT,
            SyscallNo::FsReadDir => R::READ,
            SyscallNo::FsCreateDir => R::CREATE,
            SyscallNo::FsUnlink => R::DELETE,
            SyscallNo::FsRename => R::DELETE,
            SyscallNo::FsSync => R::WRITE,
            SyscallNo::VmUnmap | SyscallNo::VmProtect => R::WRITE,
            SyscallNo::ChannelSend => R::WRITE,
            SyscallNo::ChannelRecv => R::READ,
            // HandleClose takes a handle but needs no rights: you may always
            // drop authority you hold.
            _ => return None,
        })
    }

    /// Whether this call can change persistent state.
    ///
    /// Used by the audit log to decide what to record by default, and by the
    /// Loko AI permission gate (requirement 16) to decide what needs an
    /// explicit user confirmation before an automated action proceeds.
    #[must_use]
    pub const fn is_mutating(self) -> bool {
        matches!(
            self,
            SyscallNo::FsWrite
                | SyscallNo::FsCreateDir
                | SyscallNo::FsUnlink
                | SyscallNo::FsRename
                | SyscallNo::FsSync
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HandleRights;

    #[test]
    fn numbers_are_unique() {
        for (i, a) in SyscallNo::ALL.iter().enumerate() {
            for b in &SyscallNo::ALL[i + 1..] {
                assert_ne!(
                    a.number(),
                    b.number(),
                    "{a:?} and {b:?} share syscall number {}",
                    a.number()
                );
            }
        }
    }

    #[test]
    fn numbers_are_ascending_in_all() {
        // ALL is used to generate documentation, so order is meaningful.
        let mut previous = 0u16;
        for s in SyscallNo::ALL {
            assert!(
                s.number() > previous,
                "{s:?} is out of order in SyscallNo::ALL"
            );
            previous = s.number();
        }
    }

    #[test]
    fn zero_is_not_a_syscall() {
        // A jump through a zeroed table entry must fail, not dispatch.
        assert_eq!(SyscallNo::from_number(0), None);
    }

    #[test]
    fn decoding_round_trips() {
        for s in SyscallNo::ALL {
            assert_eq!(SyscallNo::from_number(s.number()), Some(*s));
        }
    }

    #[test]
    fn unassigned_numbers_do_not_decode() {
        for n in [5u16, 15, 31, 47, 63, 79, 95, 200, u16::MAX] {
            assert_eq!(SyscallNo::from_number(n), None, "{n} should be unassigned");
        }
    }

    #[test]
    fn every_mutating_fs_call_requires_a_mutating_right() {
        for s in SyscallNo::ALL.iter().filter(|s| s.is_mutating()) {
            let rights = s
                .required_rights()
                .unwrap_or_else(|| panic!("{s:?} mutates but declares no required rights"));
            assert!(
                rights.is_mutating(),
                "{s:?} mutates but only requires {rights:?}"
            );
        }
    }

    #[test]
    fn reads_never_require_write_rights() {
        for s in [
            SyscallNo::FsRead,
            SyscallNo::FsStat,
            SyscallNo::FsReadDir,
            SyscallNo::HandleQuery,
        ] {
            let rights = s.required_rights().unwrap();
            assert!(
                !rights.contains(HandleRights::WRITE),
                "{s:?} should not require WRITE"
            );
        }
    }

    #[test]
    fn closing_a_handle_needs_no_rights() {
        assert_eq!(SyscallNo::HandleClose.required_rights(), None);
    }
}
