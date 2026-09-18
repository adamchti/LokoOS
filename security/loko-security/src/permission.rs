//! Application permissions.
//!
//! LokoOS treats a permission as a promise to the user, not a checkbox on an
//! installer. Three rules follow from that, and they are encoded here rather
//! than left to each subsystem to remember:
//!
//! * An application can only ever *use* a permission it *declared*. Declaring
//!   is public and inspectable in the Loko Store; asking for something you
//!   never declared is refused outright rather than prompted for.
//! * A permission that can be used invisibly — the microphone, the camera, the
//!   screen — can never be granted permanently by a single click. The strongest
//!   grant available is "while I'm using this app".
//! * Revocation is immediate and total. There is no "takes effect on next
//!   launch", because that is a window in which the promise is not kept.

/// A capability an application can ask the user for.
///
/// This is the list in requirement 58, plus the AI permission from requirement
/// 16. Discriminants are stable: they are written into installed application
/// records.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[repr(u16)]
#[non_exhaustive]
pub enum Permission {
    /// Read and write the user's files outside the app's own container.
    Files = 1,
    /// Capture from a camera.
    Camera = 2,
    /// Capture from a microphone.
    Microphone = 3,
    /// Reach the network.
    Network = 4,
    /// Post notifications.
    Notifications = 5,
    /// Read the clipboard. Writing is unrestricted; reading is not.
    Clipboard = 6,
    /// Read the device's location.
    Location = 7,
    /// Send requests to Loko AI on the user's behalf.
    Ai = 8,
    /// Read or change system settings.
    SystemSettings = 9,
    /// Use Bluetooth.
    Bluetooth = 10,
    /// Capture the contents of the screen.
    ScreenCapture = 11,
    /// Start automatically when the user signs in.
    RunAtLogin = 12,
}

impl Permission {
    /// Every permission.
    pub const ALL: &'static [Permission] = &[
        Permission::Files,
        Permission::Camera,
        Permission::Microphone,
        Permission::Network,
        Permission::Notifications,
        Permission::Clipboard,
        Permission::Location,
        Permission::Ai,
        Permission::SystemSettings,
        Permission::Bluetooth,
        Permission::ScreenCapture,
        Permission::RunAtLogin,
    ];

    /// The stable identifier used in a `.loko` manifest.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Permission::Files => "files",
            Permission::Camera => "camera",
            Permission::Microphone => "microphone",
            Permission::Network => "network",
            Permission::Notifications => "notifications",
            Permission::Clipboard => "clipboard",
            Permission::Location => "location",
            Permission::Ai => "ai",
            Permission::SystemSettings => "system-settings",
            Permission::Bluetooth => "bluetooth",
            Permission::ScreenCapture => "screen-capture",
            Permission::RunAtLogin => "run-at-login",
        }
    }

    /// Parses a manifest identifier.
    pub fn parse(id: &str) -> Option<Permission> {
        Permission::ALL.iter().copied().find(|p| p.id() == id)
    }

    /// The prompt title, written as a question in the user's terms.
    #[must_use]
    pub const fn prompt(self) -> &'static str {
        match self {
            Permission::Files => "Let this app open your files?",
            Permission::Camera => "Let this app use your camera?",
            Permission::Microphone => "Let this app use your microphone?",
            Permission::Network => "Let this app connect to the internet?",
            Permission::Notifications => "Let this app send you notifications?",
            Permission::Clipboard => "Let this app read what you've copied?",
            Permission::Location => "Let this app know where you are?",
            Permission::Ai => "Let this app ask Loko AI on your behalf?",
            Permission::SystemSettings => "Let this app change your settings?",
            Permission::Bluetooth => "Let this app use Bluetooth?",
            Permission::ScreenCapture => "Let this app see what's on your screen?",
            Permission::RunAtLogin => "Let this app start when you sign in?",
        }
    }

    /// What granting actually allows, in one sentence.
    ///
    /// Shown under the prompt. A permission dialog the user cannot evaluate is
    /// a permission dialog the user clicks through.
    #[must_use]
    pub const fn consequence(self) -> &'static str {
        match self {
            Permission::Files => {
                "It will be able to read and change files you choose to open with it."
            }
            Permission::Camera => "It will be able to take photos and video while you're using it.",
            Permission::Microphone => "It will be able to record sound while you're using it.",
            Permission::Network => {
                "It will be able to send and receive data, including anything you put into it."
            }
            Permission::Notifications => "It will be able to show you messages and alerts.",
            Permission::Clipboard => {
                "It will be able to read anything you copy, including passwords."
            }
            Permission::Location => "It will be able to see roughly where this device is.",
            Permission::Ai => {
                "It will be able to send text to Loko AI. Where that text goes depends on your AI settings."
            }
            Permission::SystemSettings => {
                "It will be able to change how this device is set up."
            }
            Permission::Bluetooth => "It will be able to find and connect to nearby devices.",
            Permission::ScreenCapture => {
                "It will be able to see everything on your screen, including other apps."
            }
            Permission::RunAtLogin => "It will start on its own every time you sign in.",
        }
    }

    /// Whether the permission can be exercised without the user noticing.
    ///
    /// These get the strictest treatment: no permanent grant, and an indicator
    /// while in use.
    #[must_use]
    pub const fn is_covert_capable(self) -> bool {
        matches!(
            self,
            Permission::Camera
                | Permission::Microphone
                | Permission::Location
                | Permission::ScreenCapture
                | Permission::Clipboard
        )
    }

    /// Whether LokoOS shows a live indicator while this is in use.
    #[must_use]
    pub const fn shows_indicator_while_active(self) -> bool {
        matches!(
            self,
            Permission::Camera | Permission::Microphone | Permission::ScreenCapture
        )
    }

    /// The strongest grant a user can give in a single prompt.
    ///
    /// Anything that can be used invisibly tops out at "while in use". Making
    /// a permanent grant of the microphone require a deliberate trip to
    /// Settings is friction on purpose: it is the difference between a decision
    /// and a reflex.
    #[must_use]
    pub const fn strongest_prompt_grant(self) -> Grant {
        if self.is_covert_capable() {
            Grant::WhileInUse
        } else {
            Grant::Always
        }
    }
}

/// How far a grant extends.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Grant {
    /// Refused. Distinct from never having been asked: LokoOS does not ask
    /// again after a refusal unless the user goes to Settings.
    Denied,
    /// This one time only.
    Once,
    /// Whenever the app is in the foreground.
    WhileInUse,
    /// At any time, including in the background.
    Always,
}

impl Grant {
    /// Whether this grant permits use right now, given whether the application
    /// is in the foreground.
    #[must_use]
    pub const fn permits(self, in_foreground: bool) -> bool {
        match self {
            Grant::Denied => false,
            Grant::Once | Grant::WhileInUse => in_foreground,
            Grant::Always => true,
        }
    }

    /// The label on the button that produces this grant.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Grant::Denied => "Don't Allow",
            Grant::Once => "Allow Once",
            Grant::WhileInUse => "While Using the App",
            Grant::Always => "Allow",
        }
    }
}

/// The state of one permission for one application.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Declared in the manifest, never asked about.
    NotAsked,
    /// Asked and answered.
    Decided(Grant),
}

/// What should happen when an application tries to use a permission.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Proceed.
    Allow,
    /// Show the prompt for this permission, then act on the answer.
    Prompt,
    /// Refuse, quietly. The application already asked and was told no.
    Deny(DenyReason),
}

/// Why a permission use was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DenyReason {
    /// The application never declared this permission, so it cannot be granted
    /// even by the user. Fixing it requires a new version of the app.
    NotDeclared,
    /// The user said no.
    UserRefused,
    /// Granted only while in use, and the app is in the background.
    NotInForeground,
    /// A system-wide switch is off, e.g. the camera is disabled for everything.
    DisabledSystemWide,
}

impl DenyReason {
    /// The ABI error this surfaces as.
    #[must_use]
    pub const fn as_error(self) -> loko_abi::Error {
        loko_abi::Error::PermissionDenied
    }

    /// A plain-language explanation.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            DenyReason::NotDeclared => {
                "This app didn't say it would need this, so LokoOS won't let it. The app needs to be updated."
            }
            DenyReason::UserRefused => {
                "You've turned this off for this app. You can change it in Settings."
            }
            DenyReason::NotInForeground => {
                "This app can only do this while you're using it."
            }
            DenyReason::DisabledSystemWide => {
                "This is turned off for every app on this device. You can change it in Settings."
            }
        }
    }
}

/// One application's permission record.
///
/// Fixed-capacity so the kernel and the service supervisor can hold one without
/// allocating.
#[derive(Clone, Copy, Debug)]
pub struct Record {
    declared: [bool; Permission::ALL.len()],
    state: [State; Permission::ALL.len()],
}

impl Default for Record {
    fn default() -> Self {
        Record::new(&[])
    }
}

impl Record {
    /// Builds a record from the permissions a manifest declares.
    #[must_use]
    pub fn new(declared: &[Permission]) -> Self {
        let mut record = Record {
            declared: [false; Permission::ALL.len()],
            state: [State::NotAsked; Permission::ALL.len()],
        };
        for permission in declared {
            record.declared[Self::index(*permission)] = true;
        }
        record
    }

    const fn index(permission: Permission) -> usize {
        permission as usize - 1
    }

    /// Whether the manifest declared this permission.
    #[must_use]
    pub fn declares(&self, permission: Permission) -> bool {
        self.declared[Self::index(permission)]
    }

    /// The current state.
    #[must_use]
    pub fn state(&self, permission: Permission) -> State {
        self.state[Self::index(permission)]
    }

    /// Records the user's answer to a prompt.
    ///
    /// A grant stronger than [`Permission::strongest_prompt_grant`] is clamped
    /// rather than rejected, so that a UI bug downgrades to the safe answer
    /// instead of quietly handing out a permanent grant to the microphone.
    pub fn decide(&mut self, permission: Permission, grant: Grant) {
        if !self.declares(permission) {
            return;
        }
        let clamped = grant.min(permission.strongest_prompt_grant());
        self.state[Self::index(permission)] = State::Decided(clamped);
    }

    /// Sets a grant from Loko Settings, where the user is choosing
    /// deliberately rather than answering an interruption.
    ///
    /// This is the only path that can grant `Always` for a covert-capable
    /// permission.
    pub fn set_from_settings(&mut self, permission: Permission, grant: Grant) {
        if !self.declares(permission) {
            return;
        }
        self.state[Self::index(permission)] = State::Decided(grant);
    }

    /// Withdraws a permission immediately.
    ///
    /// Requirement 58: the user can revoke. Revocation sets the state to
    /// refused rather than back to "not asked", so the app does not get to
    /// prompt again the moment it is denied.
    pub fn revoke(&mut self, permission: Permission) {
        self.state[Self::index(permission)] = State::Decided(Grant::Denied);
    }

    /// Clears a one-time grant. Called when the application exits.
    pub fn end_session(&mut self) {
        for index in 0..self.state.len() {
            if self.state[index] == State::Decided(Grant::Once) {
                self.state[index] = State::NotAsked;
            }
        }
    }

    /// Decides what happens when the application tries to use `permission`.
    #[must_use]
    pub fn check(
        &self,
        permission: Permission,
        in_foreground: bool,
        enabled_system_wide: bool,
    ) -> Outcome {
        // Undeclared first: a permission the app never asked for in public is
        // not something the user should be interrupted about in private.
        if !self.declares(permission) {
            return Outcome::Deny(DenyReason::NotDeclared);
        }
        if !enabled_system_wide {
            return Outcome::Deny(DenyReason::DisabledSystemWide);
        }
        match self.state(permission) {
            State::NotAsked => Outcome::Prompt,
            State::Decided(Grant::Denied) => Outcome::Deny(DenyReason::UserRefused),
            State::Decided(grant) => {
                if grant.permits(in_foreground) {
                    Outcome::Allow
                } else {
                    Outcome::Deny(DenyReason::NotInForeground)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_are_unique() {
        for p in Permission::ALL {
            assert_eq!(Permission::parse(p.id()), Some(*p));
        }
        for (i, a) in Permission::ALL.iter().enumerate() {
            for b in &Permission::ALL[i + 1..] {
                assert_ne!(a.id(), b.id());
            }
        }
    }

    #[test]
    fn discriminants_are_contiguous_from_one() {
        // `Record` indexes arrays by `discriminant - 1`.
        for (i, p) in Permission::ALL.iter().enumerate() {
            assert_eq!(*p as usize, i + 1, "{p:?} breaks the index assumption");
        }
    }

    #[test]
    fn an_undeclared_permission_can_never_be_granted() {
        let mut record = Record::new(&[Permission::Network]);
        // Not declared, so even an explicit settings change does nothing.
        record.set_from_settings(Permission::Camera, Grant::Always);
        record.decide(Permission::Camera, Grant::Always);
        assert_eq!(
            record.check(Permission::Camera, true, true),
            Outcome::Deny(DenyReason::NotDeclared)
        );
    }

    #[test]
    fn a_prompt_cannot_permanently_grant_a_covert_permission() {
        let mut record = Record::new(Permission::ALL);
        for permission in Permission::ALL.iter().filter(|p| p.is_covert_capable()) {
            record.decide(*permission, Grant::Always);
            assert_eq!(
                record.state(*permission),
                State::Decided(Grant::WhileInUse),
                "{permission:?} was permanently granted from a prompt"
            );
        }
    }

    #[test]
    fn settings_can_grant_what_a_prompt_cannot() {
        let mut record = Record::new(&[Permission::Microphone]);
        record.set_from_settings(Permission::Microphone, Grant::Always);
        assert_eq!(
            record.state(Permission::Microphone),
            State::Decided(Grant::Always)
        );
        assert_eq!(
            record.check(Permission::Microphone, false, true),
            Outcome::Allow
        );
    }

    #[test]
    fn while_in_use_means_while_in_use() {
        let mut record = Record::new(&[Permission::Camera]);
        record.decide(Permission::Camera, Grant::WhileInUse);
        assert_eq!(record.check(Permission::Camera, true, true), Outcome::Allow);
        assert_eq!(
            record.check(Permission::Camera, false, true),
            Outcome::Deny(DenyReason::NotInForeground)
        );
    }

    #[test]
    fn revocation_takes_effect_immediately_and_does_not_reprompt() {
        let mut record = Record::new(&[Permission::Files]);
        record.decide(Permission::Files, Grant::Always);
        assert_eq!(record.check(Permission::Files, true, true), Outcome::Allow);

        record.revoke(Permission::Files);
        assert_eq!(
            record.check(Permission::Files, true, true),
            Outcome::Deny(DenyReason::UserRefused),
            "a revoked permission must not fall back to prompting"
        );
    }

    #[test]
    fn a_one_time_grant_does_not_survive_the_session() {
        let mut record = Record::new(&[Permission::Location]);
        record.decide(Permission::Location, Grant::Once);
        assert_eq!(
            record.check(Permission::Location, true, true),
            Outcome::Allow
        );

        record.end_session();
        assert_eq!(
            record.check(Permission::Location, true, true),
            Outcome::Prompt,
            "a one-time grant must be gone after the app exits"
        );
    }

    #[test]
    fn ending_a_session_does_not_forget_a_refusal() {
        let mut record = Record::new(&[Permission::Location]);
        record.decide(Permission::Location, Grant::Denied);
        record.end_session();
        assert_eq!(
            record.check(Permission::Location, true, true),
            Outcome::Deny(DenyReason::UserRefused),
            "clearing one-time grants must not clear refusals"
        );
    }

    #[test]
    fn a_system_wide_switch_overrides_every_grant() {
        let mut record = Record::new(&[Permission::Camera]);
        record.set_from_settings(Permission::Camera, Grant::Always);
        assert_eq!(
            record.check(Permission::Camera, true, false),
            Outcome::Deny(DenyReason::DisabledSystemWide)
        );
    }

    #[test]
    fn covert_permissions_that_capture_show_an_indicator() {
        for p in [
            Permission::Camera,
            Permission::Microphone,
            Permission::ScreenCapture,
        ] {
            assert!(p.is_covert_capable());
            assert!(
                p.shows_indicator_while_active(),
                "{p:?} can capture invisibly and must show an indicator"
            );
        }
    }

    #[test]
    fn every_permission_explains_itself_without_jargon() {
        for p in Permission::ALL {
            assert!(p.prompt().ends_with('?'), "{p:?} prompt is not a question");
            let c = p.consequence();
            assert!(c.ends_with('.'), "{p:?} consequence is not a sentence");
            assert!(c.len() > 25, "{p:?} consequence is too thin to evaluate");
        }
    }

    #[test]
    fn every_denial_explains_how_to_change_it() {
        for reason in [
            DenyReason::NotDeclared,
            DenyReason::UserRefused,
            DenyReason::NotInForeground,
            DenyReason::DisabledSystemWide,
        ] {
            assert!(reason.explanation().ends_with('.'));
            assert_eq!(reason.as_error(), loko_abi::Error::PermissionDenied);
        }
    }

    #[test]
    fn grants_are_ordered_from_weakest_to_strongest() {
        // `decide` clamps with `min`, which depends on this ordering.
        assert!(Grant::Denied < Grant::Once);
        assert!(Grant::Once < Grant::WhileInUse);
        assert!(Grant::WhileInUse < Grant::Always);
    }
}
