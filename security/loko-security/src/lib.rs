//! # LokoOS security policy
//!
//! Two decisions live here, both as pure functions over explicit inputs:
//!
//! * [`permission::Record::check`] — may this application use this capability
//!   right now?
//! * [`trust::evaluate_install`] — may this package be installed?
//!
//! Neither touches the filesystem, the clock, or any global state. That is what
//! makes them testable, and being testable is what makes it reasonable to
//! believe they are right. A security decision nobody can exercise is a
//! security decision nobody has checked.
//!
//! The third decision — may this subject touch this path? — lives in
//! `lkofs-core`, next to the path model it depends on.
//!
//! ## Status
//!
//! **Implemented and tested.** Not yet enforced by a running system: there is
//! no application loader to consult it. The policy being finished before the
//! machinery that enforces it is deliberate; the alternative is discovering the
//! model is wrong after ten subsystems already depend on it.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

pub mod permission;
pub mod trust;

pub use permission::{
    DenyReason as PermissionDenyReason, Grant, Outcome, Permission, Record, State,
};
pub use trust::{
    evaluate_install, Decision as InstallDecision, Intent, Package, Refusal, Signature, Source,
    Version, Warning,
};

/// The permissions Loko AI itself holds, from requirement 16.
///
/// Loko AI is an application like any other as far as this crate is concerned;
/// it is listed here so that the set is written down once and shown in
/// `Settings → Privacy → Loko AI` from the same source the enforcement uses.
pub const LOKO_AI_PERMISSIONS: &[Permission] = &[
    Permission::Files,
    Permission::Network,
    Permission::Notifications,
    Permission::Clipboard,
    Permission::SystemSettings,
    Permission::Microphone,
    Permission::Camera,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loko_ai_starts_with_nothing_granted() {
        // Requirement 16: Loko AI's access is something the user turns on, not
        // something it has by being part of the system.
        let record = Record::new(LOKO_AI_PERMISSIONS);
        for permission in LOKO_AI_PERMISSIONS {
            assert_eq!(
                record.state(*permission),
                State::NotAsked,
                "Loko AI starts with {permission:?} already decided"
            );
            assert_eq!(record.check(*permission, true, true), Outcome::Prompt);
        }
    }

    #[test]
    fn loko_ai_cannot_permanently_hold_the_microphone_from_a_prompt() {
        let mut record = Record::new(LOKO_AI_PERMISSIONS);
        record.decide(Permission::Microphone, Grant::Always);
        assert_eq!(
            record.state(Permission::Microphone),
            State::Decided(Grant::WhileInUse)
        );
    }

    #[test]
    fn loko_ai_is_not_given_location_or_screen_capture() {
        // Nothing in requirement 14's list of what Loko AI does needs either,
        // and a system assistant that can see the screen is a different product
        // with a different consent conversation.
        assert!(!LOKO_AI_PERMISSIONS.contains(&Permission::Location));
        assert!(!LOKO_AI_PERMISSIONS.contains(&Permission::ScreenCapture));
    }
}
