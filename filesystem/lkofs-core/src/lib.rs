//! # LKOFS core
//!
//! The pure-logic half of the LokoOS filesystem: the shape of an LKO path, the
//! nineteen roots and what each one is for, and the policy that decides who may
//! do what to which location.
//!
//! There is no I/O in this crate, deliberately. Storage drivers, journalling,
//! encryption and the on-disk format live in `filesystem/lkofs-store`; this
//! crate is the part that has to be *right*, so it is the part that is kept
//! small enough to hold in your head and exhaustively testable on a host
//! machine with no hardware.
//!
//! ## Example
//!
//! ```
//! use lkofs_core::{evaluate, Decision, DenyReason, LkoPathRef, Operation, Subject, SubjectClass};
//!
//! let path = LkoPathRef::parse("LKO/System/Kernel/loko-kernel")?;
//! let shell = Subject::new(SubjectClass::UserShell);
//!
//! // LokoOS is readable by the person who owns the machine...
//! assert!(evaluate(&shell, &path, Operation::Read).is_allowed());
//!
//! // ...but not writable by anything other than a verified update.
//! assert_eq!(
//!     evaluate(&shell, &path, Operation::Write),
//!     Decision::Deny(DenyReason::ProtectedLocation),
//! );
//! # Ok::<(), lkofs_core::PathError>(())
//! ```
//!
//! ## Status
//!
//! **Implemented and tested.** This crate is complete for what it claims to
//! do. It is not yet called by a running kernel, because the kernel does not
//! yet have a storage stack to call it from — see
//! `documentation/STATUS.md` for where that sits.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod path;
pub mod policy;
pub mod root;

pub use path::{validate_component, LkoPathRef, PathError, LKO_PREFIX};
pub use policy::{
    effective_protection, evaluate, storage_category_of, Confirmation, Decision, DenyReason,
    Operation, Subject, SubjectClass,
};
pub use root::{Protection, Root, StorageCategory};

#[cfg(feature = "alloc")]
pub use path::LkoPath;

/// The LKO paths LokoOS creates during installation, in creation order.
///
/// Used by the installer to lay out a fresh volume and by `cargo xtask` to
/// verify that the tree an image actually contains matches the one documented
/// in requirement 7. Keeping it here rather than in the installer means the
/// layout is checked by this crate's tests.
pub const INITIAL_LAYOUT: &[&str] = &[
    "LKO/System",
    "LKO/System/Kernel",
    "LKO/System/Core",
    "LKO/System/Desktop",
    "LKO/System/WindowManager",
    "LKO/System/Security",
    "LKO/System/Networking",
    "LKO/System/Graphics",
    "LKO/System/SystemServices",
    "LKO/Users",
    "LKO/Apps",
    "LKO/Drivers",
    "LKO/Drivers/Graphics",
    "LKO/Drivers/Network",
    "LKO/Drivers/Audio",
    "LKO/Drivers/Storage",
    "LKO/Drivers/USB",
    "LKO/Drivers/Input",
    "LKO/Services",
    "LKO/Services/Network",
    "LKO/Services/Update",
    "LKO/Services/Security",
    "LKO/Services/Audio",
    "LKO/Services/Bluetooth",
    "LKO/Services/AI",
    "LKO/Runtime",
    "LKO/Runtime/Loko",
    "LKO/Config",
    "LKO/Config/System",
    "LKO/Config/Users",
    "LKO/Config/Network",
    "LKO/Config/Security",
    "LKO/Config/Appearance",
    "LKO/Config/Applications",
    "LKO/Data",
    "LKO/Data/Applications",
    "LKO/Data/Packages",
    "LKO/Data/Index",
    "LKO/Data/Database",
    "LKO/Temp",
    "LKO/Cache",
    "LKO/Logs",
    "LKO/Recovery",
    "LKO/Boot",
    "LKO/AI",
    "LKO/AI/Models",
    "LKO/AI/Runtime",
    "LKO/AI/Plugins",
    "LKO/AI/Memory",
    "LKO/AI/Config",
    "LKO/Linder",
    "LKO/Linder/Index",
    "LKO/Linder/Database",
    "LKO/Linder/Search",
    "LKO/Linder/AI",
    "LKO/Linder/Config",
    "LKO/Lowser",
    "LKO/Lowser/Profiles",
    "LKO/Lowser/Extensions",
    "LKO/Lowser/Cache",
    "LKO/Lowser/Downloads",
    "LKO/Lowser/Data",
    "LKO/Lowser/Config",
    "LKO/Store",
    "LKO/Store/Packages",
    "LKO/Store/Downloads",
    "LKO/Store/Metadata",
    "LKO/Store/Cache",
    "LKO/Compatibility",
    "LKO/Compatibility/Shared",
    "LKO/Dev",
];

/// The directories created inside a new user's home.
pub const USER_HOME_LAYOUT: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    "Pictures",
    "Videos",
    "Music",
    "Projects",
    "AppData",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_initial_layout_is_all_valid_paths() {
        for entry in INITIAL_LAYOUT {
            LkoPathRef::parse(entry)
                .unwrap_or_else(|e| panic!("layout entry {entry} is invalid: {e}"));
        }
    }

    #[test]
    fn the_initial_layout_creates_parents_before_children() {
        // The installer walks this list in order, so a child must never appear
        // before its parent.
        for (i, entry) in INITIAL_LAYOUT.iter().enumerate() {
            let path = LkoPathRef::parse(entry).unwrap();
            let Some(parent) = path.parent() else {
                continue;
            };
            if parent == LkoPathRef::ROOT {
                continue;
            }
            let parent_string = parent.to_string();
            let parent_index = INITIAL_LAYOUT[..i]
                .iter()
                .position(|e| LkoPathRef::parse(e).unwrap().to_string() == parent_string);
            assert!(
                parent_index.is_some(),
                "{entry} appears before its parent {parent_string}"
            );
        }
    }

    #[test]
    fn every_root_from_requirement_7_is_created() {
        for root in Root::ALL {
            let expected = alloc::format!("LKO/{}", root.name());
            assert!(
                INITIAL_LAYOUT.iter().any(|e| *e == expected),
                "the installer never creates {expected}"
            );
        }
    }

    #[test]
    fn the_layout_has_no_duplicates() {
        for (i, a) in INITIAL_LAYOUT.iter().enumerate() {
            for b in &INITIAL_LAYOUT[i + 1..] {
                assert_ne!(a, b, "{a} is listed twice");
            }
        }
    }

    #[test]
    fn no_optional_component_directories_are_created_up_front() {
        // Requirement 2: the base OS stays under 2 GB by not shipping the
        // optional runtimes at all. The installer must not even create their
        // directories, so that a missing runtime is distinguishable from an
        // empty one.
        for absent in [
            "LKO/Runtime/Windows",
            "LKO/Runtime/Mac",
            "LKO/Compatibility/Windows",
        ] {
            assert!(
                !INITIAL_LAYOUT.contains(&absent),
                "{absent} must be created by the runtime installer, not by the OS installer"
            );
        }
    }

    #[test]
    fn a_new_user_home_is_valid_and_sandbox_compatible() {
        let home = LkoPath::parse("LKO/Users/Adam").unwrap();
        for dir in USER_HOME_LAYOUT {
            let path = home
                .join(dir)
                .unwrap_or_else(|e| panic!("user directory {dir} is invalid: {e}"));
            assert!(path.as_ref().is_within(&home.as_ref()));
        }
        assert!(
            USER_HOME_LAYOUT.contains(&"AppData"),
            "sandboxed apps need AppData to exist or they have nowhere to write"
        );
    }
}
