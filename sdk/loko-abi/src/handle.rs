//! Handles: the unit of authority in LokoOS.
//!
//! A [`Handle`] is an opaque, per-process index into a kernel-side table. It
//! names an object *and* carries the rights the holder has over it. A process
//! can only weaken a handle ([`HandleRights::derive`]), never strengthen it, so
//! authority in LokoOS only ever flows downhill.
//!
//! This is what makes revocation meaningful. When a user turns off "Files" for
//! an app in Loko Settings, the kernel closes that app's filesystem handles.
//! There is no cached path string the app can fall back on, because paths were
//! never authority in the first place.

use bitflags::bitflags;

/// An opaque per-process handle number.
///
/// Handle `0` is never valid, so a zeroed structure is not accidentally a
/// reference to a real object.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[repr(transparent)]
pub struct Handle(pub u32);

impl Handle {
    /// The reserved invalid handle.
    pub const INVALID: Handle = Handle(0);

    /// Whether this handle could refer to an object.
    ///
    /// A `true` result does not mean the handle is open — only the kernel knows
    /// that. It means the value is not the reserved sentinel.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

impl Default for Handle {
    fn default() -> Self {
        Handle::INVALID
    }
}

/// What kind of kernel object a handle refers to.
///
/// Carried alongside the handle in the kernel table, and returned by
/// [`crate::SyscallNo::HandleQuery`] so that a process can reason about handles
/// it was passed without having to try operations and catch failures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
#[non_exhaustive]
pub enum HandleType {
    /// A directory in LKOFS. Can be opened relative to, listed, watched.
    Directory = 1,
    /// A file in LKOFS.
    File = 2,
    /// A running process.
    Process = 3,
    /// A thread within a process.
    Thread = 4,
    /// A region of address space.
    VirtualMemory = 5,
    /// A bidirectional message channel to another process.
    Channel = 6,
    /// A one-shot or repeating timer.
    Timer = 7,
    /// A registered interrupt source, held by a driver.
    Interrupt = 8,
    /// A device exposed by the HAL.
    Device = 9,
    /// A permission grant that can be presented to a service.
    Grant = 10,
}

bitflags! {
    /// The rights a handle conveys.
    ///
    /// Rights are checked by the kernel on every syscall, not once at open
    /// time. A handle whose rights were narrowed by a parent process cannot be
    /// widened by the child under any circumstances.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
    #[repr(transparent)]
    pub struct HandleRights: u32 {
        /// Read the object's contents.
        const READ          = 1 << 0;
        /// Modify the object's contents.
        const WRITE         = 1 << 1;
        /// Execute, or for a directory, traverse into it.
        const EXECUTE       = 1 << 2;
        /// Read the object's metadata (size, timestamps, type).
        const INSPECT       = 1 << 3;
        /// Change the object's metadata, including permissions.
        const ADMINISTER    = 1 << 4;
        /// Create new objects inside this one. Directories only.
        const CREATE        = 1 << 5;
        /// Remove objects from inside this one. Directories only.
        const DELETE        = 1 << 6;
        /// Subscribe to change notifications.
        const WATCH         = 1 << 7;
        /// Pass this handle to another process over a channel.
        ///
        /// Withholding this right is how a service hands out authority that
        /// cannot be re-delegated.
        const TRANSFER      = 1 << 8;
        /// Create a new handle to the same object with equal or fewer rights.
        const DERIVE        = 1 << 9;

        /// A read-only view: the common case for opening a document.
        const RO = Self::READ.bits() | Self::INSPECT.bits();
        /// A read-write view of an existing object.
        const RW = Self::RO.bits() | Self::WRITE.bits();
        /// Full authority over a directory subtree.
        const DIR_FULL = Self::RW.bits()
            | Self::EXECUTE.bits()
            | Self::CREATE.bits()
            | Self::DELETE.bits()
            | Self::WATCH.bits()
            | Self::DERIVE.bits();
    }
}

impl HandleRights {
    /// Narrows these rights to `requested`.
    ///
    /// Returns `None` if `requested` asks for anything this handle does not
    /// already have, or if this handle may not be derived from at all. The
    /// asymmetry is the point: there is no code path that returns wider rights
    /// than it was given.
    pub fn derive(self, requested: HandleRights) -> Option<HandleRights> {
        if !self.contains(HandleRights::DERIVE) {
            return None;
        }
        if !self.contains(requested) {
            return None;
        }
        Some(requested)
    }

    /// Whether these rights permit an operation needing `required`.
    #[must_use]
    pub const fn allows(self, required: HandleRights) -> bool {
        self.contains(required)
    }

    /// Whether these rights can modify anything.
    ///
    /// Used to decide whether an action needs a user confirmation prompt.
    #[must_use]
    pub fn is_mutating(self) -> bool {
        self.intersects(
            HandleRights::WRITE
                | HandleRights::ADMINISTER
                | HandleRights::CREATE
                | HandleRights::DELETE,
        )
    }
}

/// The rights every handle type may legally carry.
///
/// Requesting `CREATE` on a file handle is a programming error, not a
/// permission failure, and is rejected as [`crate::Error::InvalidArgument`] so
/// that it shows up during development rather than as a confusing denial.
#[must_use]
pub fn legal_rights_for(kind: HandleType) -> HandleRights {
    let base = HandleRights::INSPECT | HandleRights::TRANSFER | HandleRights::DERIVE;
    match kind {
        HandleType::Directory => base | HandleRights::DIR_FULL,
        HandleType::File => {
            base | HandleRights::READ
                | HandleRights::WRITE
                | HandleRights::EXECUTE
                | HandleRights::WATCH
                | HandleRights::ADMINISTER
        }
        HandleType::Process | HandleType::Thread => {
            base | HandleRights::READ | HandleRights::WRITE | HandleRights::ADMINISTER
        }
        HandleType::VirtualMemory => {
            base | HandleRights::READ | HandleRights::WRITE | HandleRights::EXECUTE
        }
        HandleType::Channel => base | HandleRights::READ | HandleRights::WRITE,
        HandleType::Timer => base | HandleRights::READ | HandleRights::WATCH,
        HandleType::Interrupt => base | HandleRights::READ | HandleRights::WATCH,
        HandleType::Device => {
            base | HandleRights::READ | HandleRights::WRITE | HandleRights::ADMINISTER
        }
        // A grant is presented, never read or written. It deliberately cannot
        // be derived: narrowing a permission grant would let a process forge a
        // plausible-looking weaker grant it was never issued.
        HandleType::Grant => HandleRights::INSPECT | HandleRights::TRANSFER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_handle_is_never_valid() {
        assert!(!Handle::INVALID.is_valid());
        assert!(!Handle::default().is_valid());
        assert!(Handle(1).is_valid());
    }

    #[test]
    fn derive_can_only_narrow() {
        let full = HandleRights::DIR_FULL;
        assert_eq!(
            full.derive(HandleRights::RO),
            Some(HandleRights::RO),
            "narrowing to read-only must succeed"
        );
        assert_eq!(
            HandleRights::RO.derive(HandleRights::RW),
            None,
            "a read-only handle must not be able to derive a writable one"
        );
    }

    #[test]
    fn derive_requires_the_derive_right() {
        let no_derive = HandleRights::RW;
        assert!(!no_derive.contains(HandleRights::DERIVE));
        assert_eq!(no_derive.derive(HandleRights::RO), None);
    }

    #[test]
    fn derive_is_idempotent_at_the_same_level() {
        let r = HandleRights::DIR_FULL;
        assert_eq!(r.derive(r), Some(r));
    }

    #[test]
    fn grants_cannot_be_derived_from() {
        let rights = legal_rights_for(HandleType::Grant);
        assert!(!rights.contains(HandleRights::DERIVE));
        assert!(!rights.is_mutating());
    }

    #[test]
    fn only_directories_may_create_or_delete() {
        for kind in [
            HandleType::File,
            HandleType::Process,
            HandleType::Channel,
            HandleType::Timer,
            HandleType::Device,
            HandleType::Grant,
        ] {
            let rights = legal_rights_for(kind);
            assert!(
                !rights.intersects(HandleRights::CREATE | HandleRights::DELETE),
                "{kind:?} must not be able to carry CREATE or DELETE"
            );
        }
        assert!(legal_rights_for(HandleType::Directory).contains(HandleRights::CREATE));
    }

    #[test]
    fn read_only_is_not_mutating() {
        assert!(!HandleRights::RO.is_mutating());
        assert!(HandleRights::RW.is_mutating());
        assert!(HandleRights::DIR_FULL.is_mutating());
    }
}
