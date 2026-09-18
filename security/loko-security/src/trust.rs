//! Package trust: deciding whether a `.loko` package may be installed.
//!
//! Requirement 59 asks for signatures, integrity verification, publisher
//! identity and a warning on anything unsigned. The decision those inputs feed
//! is written here as one function, [`evaluate_install`], for the same reason
//! the filesystem policy is one function: a trust decision that is spread
//! across an installer, a store client and an update service is a trust
//! decision with three subtly different answers.
//!
//! Two rules are absolute and are not user-overridable:
//!
//! * A package whose contents do not match its signature is never installed.
//!   Not with a warning, not with a checkbox. A failed integrity check means
//!   the bytes are not what the publisher signed, and no amount of user
//!   confidence changes that.
//! * A package cannot claim to be from LokoOS itself unless it is signed by the
//!   LokoOS root. Otherwise the most valuable label in the system is free.

/// How a package's signature checked out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Signature {
    /// Signed by the LokoOS release key.
    LokoOfficial,
    /// Signed by a publisher whose identity the Loko Store has verified.
    VerifiedPublisher,
    /// Signed by a key the user has previously chosen to trust.
    KnownPublisher,
    /// Signed by a key nobody has vouched for. The signature is valid — it
    /// proves the package has not changed since it was signed — but it says
    /// nothing about who signed it.
    UnknownPublisher,
    /// No signature at all.
    Unsigned,
    /// A signature that does not match the contents.
    Invalid,
}

impl Signature {
    /// Whether the bytes are provably unchanged since signing.
    #[must_use]
    pub const fn proves_integrity(self) -> bool {
        matches!(
            self,
            Signature::LokoOfficial
                | Signature::VerifiedPublisher
                | Signature::KnownPublisher
                | Signature::UnknownPublisher
        )
    }

    /// Whether anyone has vouched for who the publisher is.
    #[must_use]
    pub const fn proves_identity(self) -> bool {
        matches!(
            self,
            Signature::LokoOfficial | Signature::VerifiedPublisher | Signature::KnownPublisher
        )
    }
}

/// Where a package came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// The Loko Store.
    LokoStore,
    /// A system update served by Loko Update.
    SystemUpdate,
    /// A file the user chose themselves.
    LocalFile,
    /// A URL the user or another application supplied.
    Download,
}

/// What the package says it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Package<'a> {
    /// Reverse-DNS identifier, e.g. `com.example.editor`.
    pub id: &'a str,
    /// The version being offered.
    pub version: Version,
    /// How its signature checked out.
    pub signature: Signature,
    /// Where it came from.
    pub source: Source,
    /// Whether the package declares itself a LokoOS system component.
    pub claims_system_component: bool,
}

/// A semantic version, as requirement 66 specifies.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version {
    /// Incompatible changes.
    pub major: u16,
    /// Backwards-compatible additions.
    pub minor: u16,
    /// Fixes.
    pub patch: u16,
}

impl Version {
    /// A version.
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Version {
            major,
            minor,
            patch,
        }
    }
}

/// What the system is being asked to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Intent {
    /// Nothing with this id is installed.
    FreshInstall,
    /// Replacing an installed version.
    Update {
        /// What is installed now.
        installed: Version,
    },
}

/// The decision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Install without saying anything.
    Install,
    /// Install, after the user acknowledges a warning.
    WarnThenInstall(Warning),
    /// Do not install.
    Refuse(Refusal),
}

impl Decision {
    /// Whether installation proceeds with no further interaction.
    #[must_use]
    pub const fn is_silent(self) -> bool {
        matches!(self, Decision::Install)
    }

    /// Whether installation can proceed at all.
    #[must_use]
    pub const fn can_proceed(self) -> bool {
        !matches!(self, Decision::Refuse(_))
    }
}

/// Something the user must acknowledge first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Warning {
    /// Valid signature, but nobody has vouched for the signer.
    UnknownPublisher,
    /// No signature at all.
    Unsigned,
    /// Downloaded rather than obtained from the Store.
    FromOutsideTheStore,
}

impl Warning {
    /// The heading on the dialog.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Warning::UnknownPublisher => "LokoOS doesn't know who made this.",
            Warning::Unsigned => "This app isn't signed.",
            Warning::FromOutsideTheStore => "This app didn't come from the Loko Store.",
        }
    }

    /// The body, explaining the actual risk.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Warning::UnknownPublisher => {
                "The app hasn't been changed since it was made, but nobody has confirmed who made it. Only install it if you trust where you got it from."
            }
            Warning::Unsigned => {
                "There's no way to tell who made this app or whether it has been changed since. Only install it if you trust where you got it from."
            }
            Warning::FromOutsideTheStore => {
                "Apps from the Loko Store are checked before they're listed. This one hasn't been. Only install it if you trust where you got it from."
            }
        }
    }
}

/// Why installation is refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The contents do not match the signature.
    IntegrityFailure,
    /// The package claims to be a LokoOS component but is not signed by LokoOS.
    ForgedSystemComponent,
    /// A system component from anywhere other than Loko Update.
    SystemComponentFromWrongSource,
    /// The offered version is older than what is installed.
    Downgrade,
    /// Same version, already installed.
    AlreadyInstalled,
}

impl Refusal {
    /// The ABI error this surfaces as.
    #[must_use]
    pub const fn as_error(self) -> loko_abi::Error {
        match self {
            Refusal::IntegrityFailure
            | Refusal::ForgedSystemComponent
            | Refusal::SystemComponentFromWrongSource => loko_abi::Error::IntegrityFailure,
            Refusal::Downgrade => loko_abi::Error::PermissionDenied,
            Refusal::AlreadyInstalled => loko_abi::Error::AlreadyExists,
        }
    }

    /// What to tell the user.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Refusal::IntegrityFailure => {
                "This file has been changed since it was signed, so LokoOS won't install it. Try downloading it again."
            }
            Refusal::ForgedSystemComponent => {
                "This claims to be part of LokoOS, but it isn't. LokoOS won't install it."
            }
            Refusal::SystemComponentFromWrongSource => {
                "Parts of LokoOS can only be installed through system updates."
            }
            Refusal::Downgrade => {
                "This is an older version than the one you have. Uninstall the current version first if you really want to go back."
            }
            Refusal::AlreadyInstalled => "You already have this version.",
        }
    }
}

/// Decides whether `package` may be installed.
///
/// Rules are ordered most to least severe; the first that applies wins.
#[must_use]
pub fn evaluate_install(package: &Package<'_>, intent: Intent, developer_mode: bool) -> Decision {
    // Rule 1. A broken signature ends the conversation. Not overridable, and
    // deliberately checked before anything that could produce a softer answer.
    if package.signature == Signature::Invalid {
        return Decision::Refuse(Refusal::IntegrityFailure);
    }

    // Rule 2. Only LokoOS may claim to be LokoOS.
    if package.claims_system_component {
        if package.signature != Signature::LokoOfficial {
            return Decision::Refuse(Refusal::ForgedSystemComponent);
        }
        if package.source != Source::SystemUpdate {
            return Decision::Refuse(Refusal::SystemComponentFromWrongSource);
        }
    }

    // Rule 3. Version arithmetic. A downgrade is refused because the older
    // version is older for a reason, and the usual reason is a fixed
    // vulnerability.
    if let Intent::Update { installed } = intent {
        if package.version < installed {
            return Decision::Refuse(Refusal::Downgrade);
        }
        if package.version == installed {
            return Decision::Refuse(Refusal::AlreadyInstalled);
        }
    }

    // Rule 4. Unsigned packages. Allowed only in Developer Mode, and even then
    // with a warning: a developer installing their own build should still be
    // told when they are installing something nobody signed.
    if package.signature == Signature::Unsigned {
        if !developer_mode {
            return Decision::Refuse(Refusal::IntegrityFailure);
        }
        return Decision::WarnThenInstall(Warning::Unsigned);
    }

    // Rule 5. Valid signature, unknown signer.
    if !package.signature.proves_identity() {
        return Decision::WarnThenInstall(Warning::UnknownPublisher);
    }

    // Rule 6. Known publisher, but not from the Store.
    if matches!(package.source, Source::Download | Source::LocalFile) {
        return Decision::WarnThenInstall(Warning::FromOutsideTheStore);
    }

    Decision::Install
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(signature: Signature, source: Source) -> Package<'static> {
        Package {
            id: "com.example.editor",
            version: Version::new(1, 2, 0),
            signature,
            source,
            claims_system_component: false,
        }
    }

    #[test]
    fn a_broken_signature_is_never_installable() {
        let bad = package(Signature::Invalid, Source::LokoStore);
        // Not in developer mode, not as an update, not from any source.
        for source in [
            Source::LokoStore,
            Source::SystemUpdate,
            Source::LocalFile,
            Source::Download,
        ] {
            for developer_mode in [false, true] {
                let mut p = bad;
                p.source = source;
                assert_eq!(
                    evaluate_install(&p, Intent::FreshInstall, developer_mode),
                    Decision::Refuse(Refusal::IntegrityFailure),
                    "a tampered package was accepted from {source:?} (dev mode {developer_mode})"
                );
            }
        }
    }

    #[test]
    fn only_lokoos_may_claim_to_be_lokoos() {
        for signature in [
            Signature::VerifiedPublisher,
            Signature::KnownPublisher,
            Signature::UnknownPublisher,
            Signature::Unsigned,
        ] {
            let mut p = package(signature, Source::SystemUpdate);
            p.claims_system_component = true;
            assert_eq!(
                evaluate_install(&p, Intent::FreshInstall, true),
                Decision::Refuse(Refusal::ForgedSystemComponent),
                "{signature:?} was able to impersonate a LokoOS component"
            );
        }
    }

    #[test]
    fn system_components_only_arrive_through_updates() {
        for source in [Source::LokoStore, Source::LocalFile, Source::Download] {
            let mut p = package(Signature::LokoOfficial, source);
            p.claims_system_component = true;
            assert_eq!(
                evaluate_install(&p, Intent::FreshInstall, true),
                Decision::Refuse(Refusal::SystemComponentFromWrongSource)
            );
        }
        let mut good = package(Signature::LokoOfficial, Source::SystemUpdate);
        good.claims_system_component = true;
        assert_eq!(
            evaluate_install(&good, Intent::FreshInstall, false),
            Decision::Install
        );
    }

    #[test]
    fn downgrades_are_refused() {
        let p = package(Signature::VerifiedPublisher, Source::LokoStore);
        assert_eq!(
            evaluate_install(
                &p,
                Intent::Update {
                    installed: Version::new(1, 3, 0)
                },
                false
            ),
            Decision::Refuse(Refusal::Downgrade)
        );
        assert_eq!(
            evaluate_install(
                &p,
                Intent::Update {
                    installed: Version::new(1, 2, 0)
                },
                false
            ),
            Decision::Refuse(Refusal::AlreadyInstalled)
        );
        assert_eq!(
            evaluate_install(
                &p,
                Intent::Update {
                    installed: Version::new(1, 1, 9)
                },
                false
            ),
            Decision::Install
        );
    }

    #[test]
    fn version_ordering_is_semantic_not_lexical() {
        // The bug this guards against: comparing "1.10.0" and "1.9.0" as text.
        assert!(Version::new(1, 10, 0) > Version::new(1, 9, 0));
        assert!(Version::new(2, 0, 0) > Version::new(1, 99, 99));
        assert!(Version::new(1, 0, 10) > Version::new(1, 0, 9));
    }

    #[test]
    fn unsigned_packages_need_developer_mode_and_still_warn() {
        let p = package(Signature::Unsigned, Source::LocalFile);
        assert_eq!(
            evaluate_install(&p, Intent::FreshInstall, false),
            Decision::Refuse(Refusal::IntegrityFailure)
        );
        assert_eq!(
            evaluate_install(&p, Intent::FreshInstall, true),
            Decision::WarnThenInstall(Warning::Unsigned),
            "even a developer should be told when nothing is signed"
        );
    }

    #[test]
    fn a_valid_signature_from_an_unknown_signer_warns_but_installs() {
        let p = package(Signature::UnknownPublisher, Source::Download);
        assert_eq!(
            evaluate_install(&p, Intent::FreshInstall, false),
            Decision::WarnThenInstall(Warning::UnknownPublisher)
        );
    }

    #[test]
    fn store_installs_from_verified_publishers_are_silent() {
        let p = package(Signature::VerifiedPublisher, Source::LokoStore);
        let decision = evaluate_install(&p, Intent::FreshInstall, false);
        assert!(decision.is_silent());
        assert!(decision.can_proceed());
    }

    #[test]
    fn sideloading_a_known_publisher_still_says_where_it_came_from() {
        for source in [Source::Download, Source::LocalFile] {
            let p = package(Signature::KnownPublisher, source);
            assert_eq!(
                evaluate_install(&p, Intent::FreshInstall, false),
                Decision::WarnThenInstall(Warning::FromOutsideTheStore)
            );
        }
    }

    #[test]
    fn signature_levels_say_what_they_actually_prove() {
        // An unknown-publisher signature proves the bytes are unchanged but
        // says nothing about who signed them. Conflating those is how a
        // "signed = safe" indicator becomes misleading.
        assert!(Signature::UnknownPublisher.proves_integrity());
        assert!(!Signature::UnknownPublisher.proves_identity());
        assert!(!Signature::Unsigned.proves_integrity());
        assert!(!Signature::Invalid.proves_integrity());
        assert!(Signature::LokoOfficial.proves_identity());
    }

    #[test]
    fn every_warning_and_refusal_is_written_for_a_person() {
        for w in [
            Warning::UnknownPublisher,
            Warning::Unsigned,
            Warning::FromOutsideTheStore,
        ] {
            assert!(w.heading().ends_with('.'));
            assert!(w.detail().len() > 50, "{w:?} does not explain the risk");
        }
        for r in [
            Refusal::IntegrityFailure,
            Refusal::ForgedSystemComponent,
            Refusal::SystemComponentFromWrongSource,
            Refusal::Downgrade,
            Refusal::AlreadyInstalled,
        ] {
            assert!(r.explanation().ends_with('.'));
            assert!(!r.explanation().contains("0x"));
        }
    }
}
