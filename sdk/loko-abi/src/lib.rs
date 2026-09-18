//! # LokoOS ABI
//!
//! The contract between userland and the LokoOS kernel. Everything in this
//! crate is layout-stable: changing a discriminant, a struct field order, or a
//! syscall number is a breaking change to every binary ever compiled against
//! LokoOS, and must go through the versioning process in
//! `documentation/architecture/adr/0005-abi-stability.md`.
//!
//! ## Design: handles, not paths
//!
//! LokoOS syscalls do not take paths. A process cannot name a resource it was
//! not given, so there is no ambient authority to confuse-deputy its way
//! around. A process receives [`Handle`]s at startup from its manifest, and
//! derives narrower handles from them with [`SyscallNo::HandleDerive`].
//!
//! This is the mechanism that makes the permission model in `loko-security`
//! enforceable rather than advisory: revoking a permission closes handles.
//!
//! ## Status
//!
//! **Specified and compiling. Not yet wired to a running kernel.** The kernel
//! currently implements the dispatch table shell only; see
//! `kernel/loko-kernel/src/syscall.rs` for which numbers are live.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

pub mod error;
pub mod handle;
pub mod syscall;

pub use error::{Error, Result};
pub use handle::{Handle, HandleRights, HandleType};
pub use syscall::SyscallNo;

/// ABI major version. Bumped only for incompatible changes.
///
/// A binary records the version it was built against in its `.loko` manifest;
/// the loader refuses to start a binary whose major version the running kernel
/// does not implement, with a message the user can act on rather than a fault.
pub const ABI_VERSION_MAJOR: u16 = 0;

/// ABI minor version. Bumped when syscalls are added.
///
/// A binary built against a lower minor version always runs on a higher one.
pub const ABI_VERSION_MINOR: u16 = 1;

/// The size of a memory page as seen by the ABI, in bytes.
///
/// LokoOS may use larger pages internally for its own mappings, but every
/// length and alignment crossing the syscall boundary is expressed in terms of
/// this value so that userland does not have to probe for it.
pub const PAGE_SIZE: usize = 4096;

/// Maximum length in bytes of a single LKO path component.
pub const MAX_COMPONENT_LEN: usize = 255;

/// Maximum total length in bytes of an LKO path.
pub const MAX_PATH_LEN: usize = 4096;

/// The raw register-level form of a syscall return value.
///
/// LokoOS returns a single `isize`: non-negative values are success payloads
/// (a handle number, a byte count, a length), negative values are the negated
/// [`Error`] discriminant. This is a deliberate copy of a pattern that has
/// survived forty years of real use, because it needs no out-parameter and no
/// errno-style thread-local.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct RawReturn(pub isize);

impl RawReturn {
    /// Interprets the raw value as a `Result`.
    pub fn decode(self) -> Result<usize> {
        if self.0 >= 0 {
            Ok(self.0 as usize)
        } else {
            Err(Error::from_raw(self.0))
        }
    }

    /// Builds a raw return value from a success payload.
    ///
    /// # Panics
    ///
    /// Panics if `value` does not fit in a non-negative `isize`. Callers inside
    /// the kernel must clamp before calling; a payload that large is a bug.
    #[must_use]
    pub fn ok(value: usize) -> Self {
        assert!(
            value <= isize::MAX as usize,
            "syscall success payload must fit in a non-negative isize"
        );
        RawReturn(value as isize)
    }

    /// Builds a raw return value from an error.
    #[must_use]
    pub fn err(error: Error) -> Self {
        RawReturn(error.to_raw())
    }
}

impl From<Result<usize>> for RawReturn {
    fn from(value: Result<usize>) -> Self {
        match value {
            Ok(v) => RawReturn::ok(v),
            Err(e) => RawReturn::err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_return_round_trips_success() {
        assert_eq!(RawReturn::ok(0).decode(), Ok(0));
        assert_eq!(RawReturn::ok(4096).decode(), Ok(4096));
    }

    #[test]
    fn raw_return_round_trips_every_error() {
        for error in Error::ALL {
            let decoded = RawReturn::err(*error).decode();
            assert_eq!(decoded, Err(*error), "error {error:?} did not round-trip");
        }
    }

    #[test]
    fn success_and_error_spaces_do_not_overlap() {
        // An error must never decode as a success, or a failed syscall would be
        // read as a valid handle number.
        for error in Error::ALL {
            assert!(
                RawReturn::err(*error).0 < 0,
                "error {error:?} encoded as a non-negative value"
            );
        }
    }
}
