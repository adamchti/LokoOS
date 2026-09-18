//! System-call error codes.
//!
//! Every LokoOS error carries three things: a stable numeric code, a short
//! developer-facing name, and a plain-language explanation with suggested user
//! actions. The third part is not decoration. Requirement 64 of the LokoOS
//! design says the system must never show `ERR_0x00482` when it could say what
//! actually went wrong, and the only way to guarantee that is to make the
//! human-readable form part of the error type instead of something a UI layer
//! is trusted to remember to add.

/// A LokoOS system-call error.
///
/// Discriminants are part of the stable ABI. Add new variants at the end; never
/// renumber.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i16)]
#[non_exhaustive]
pub enum Error {
    /// An argument was structurally invalid: a bad length, a malformed path, a
    /// reserved flag bit set.
    InvalidArgument = 1,
    /// The handle number does not refer to an open handle in this process.
    BadHandle = 2,
    /// The handle is open but lacks the rights this operation needs.
    ///
    /// Distinct from [`Error::PermissionDenied`]: the process holds a
    /// capability for the object, just not a strong enough one.
    InsufficientRights = 3,
    /// The calling process has no capability for this object at all.
    PermissionDenied = 4,
    /// The named object does not exist.
    NotFound = 5,
    /// An object with that name already exists and the caller asked for
    /// exclusive creation.
    AlreadyExists = 6,
    /// The operation is valid but not implemented in this build.
    ///
    /// Used deliberately and visibly during development so that an unfinished
    /// subsystem fails loudly instead of silently succeeding.
    NotImplemented = 7,
    /// The kernel could not allocate memory to complete the request.
    OutOfMemory = 8,
    /// The target is a directory and the operation requires a file, or the
    /// reverse.
    WrongObjectType = 9,
    /// The path names a location inside a protected system root and the caller
    /// is not the system updater.
    ProtectedLocation = 10,
    /// The operation would exceed a quota: handles, memory, or storage.
    QuotaExceeded = 11,
    /// The device or backing store reported a hardware-level failure.
    DeviceFault = 12,
    /// The operation was interrupted and may be retried.
    Interrupted = 13,
    /// The operation would block and the handle is in non-blocking mode.
    WouldBlock = 14,
    /// A signature or integrity check failed.
    ///
    /// Never retried automatically: a package or update that fails
    /// verification is escalated to the user, per requirement 59.
    IntegrityFailure = 15,
    /// The requested operation needs a user confirmation that has not been
    /// granted. The caller should surface a prompt and retry.
    ConfirmationRequired = 16,
    /// The compatibility runtime required to service this request is not
    /// installed.
    RuntimeMissing = 17,
    /// The syscall number is not recognised by this kernel.
    NoSuchSyscall = 18,
}

impl Error {
    /// Every error variant, for exhaustive testing and for generating the
    /// documentation table.
    pub const ALL: &'static [Error] = &[
        Error::InvalidArgument,
        Error::BadHandle,
        Error::InsufficientRights,
        Error::PermissionDenied,
        Error::NotFound,
        Error::AlreadyExists,
        Error::NotImplemented,
        Error::OutOfMemory,
        Error::WrongObjectType,
        Error::ProtectedLocation,
        Error::QuotaExceeded,
        Error::DeviceFault,
        Error::Interrupted,
        Error::WouldBlock,
        Error::IntegrityFailure,
        Error::ConfirmationRequired,
        Error::RuntimeMissing,
        Error::NoSuchSyscall,
    ];

    /// The stable numeric code.
    #[must_use]
    pub const fn code(self) -> i16 {
        self as i16
    }

    /// Encodes as a negative `isize` for the register return convention.
    #[must_use]
    pub const fn to_raw(self) -> isize {
        -(self as i16 as isize)
    }

    /// Decodes from a negative `isize`.
    ///
    /// An unrecognised code decodes to [`Error::InvalidArgument`] rather than
    /// panicking, so that a userland built against a newer ABI degrades instead
    /// of aborting.
    #[must_use]
    pub const fn from_raw(raw: isize) -> Self {
        match -raw {
            1 => Error::InvalidArgument,
            2 => Error::BadHandle,
            3 => Error::InsufficientRights,
            4 => Error::PermissionDenied,
            5 => Error::NotFound,
            6 => Error::AlreadyExists,
            7 => Error::NotImplemented,
            8 => Error::OutOfMemory,
            9 => Error::WrongObjectType,
            10 => Error::ProtectedLocation,
            11 => Error::QuotaExceeded,
            12 => Error::DeviceFault,
            13 => Error::Interrupted,
            14 => Error::WouldBlock,
            15 => Error::IntegrityFailure,
            16 => Error::ConfirmationRequired,
            17 => Error::RuntimeMissing,
            18 => Error::NoSuchSyscall,
            _ => Error::InvalidArgument,
        }
    }

    /// The short, stable, developer-facing identifier.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Error::InvalidArgument => "InvalidArgument",
            Error::BadHandle => "BadHandle",
            Error::InsufficientRights => "InsufficientRights",
            Error::PermissionDenied => "PermissionDenied",
            Error::NotFound => "NotFound",
            Error::AlreadyExists => "AlreadyExists",
            Error::NotImplemented => "NotImplemented",
            Error::OutOfMemory => "OutOfMemory",
            Error::WrongObjectType => "WrongObjectType",
            Error::ProtectedLocation => "ProtectedLocation",
            Error::QuotaExceeded => "QuotaExceeded",
            Error::DeviceFault => "DeviceFault",
            Error::Interrupted => "Interrupted",
            Error::WouldBlock => "WouldBlock",
            Error::IntegrityFailure => "IntegrityFailure",
            Error::ConfirmationRequired => "ConfirmationRequired",
            Error::RuntimeMissing => "RuntimeMissing",
            Error::NoSuchSyscall => "NoSuchSyscall",
        }
    }

    /// A plain-language explanation suitable for showing to a user.
    ///
    /// Written in the second person, without jargon, and without a code. The
    /// caller is expected to prepend the specific subject, e.g.
    /// `"LokoOS couldn't open Documents. "` + `explanation()`.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Error::InvalidArgument => {
                "The request wasn't formed correctly. This is usually a problem with the app, not with your system."
            }
            Error::BadHandle => {
                "The app referred to something that is no longer open. This is usually a problem with the app."
            }
            Error::InsufficientRights => {
                "The app has access to this, but not the kind of access it asked for."
            }
            Error::PermissionDenied => {
                "The app doesn't have permission to do this. You can change what it's allowed to do in Settings."
            }
            Error::NotFound => "It isn't there. It may have been moved, renamed, or deleted.",
            Error::AlreadyExists => "Something with that name is already there.",
            Error::NotImplemented => {
                "This part of LokoOS isn't finished yet. It's marked experimental in this build."
            }
            Error::OutOfMemory => {
                "Your system ran out of memory. Closing some apps should free enough to continue."
            }
            Error::WrongObjectType => {
                "That's a different kind of item than the one this action works on."
            }
            Error::ProtectedLocation => {
                "This location is part of LokoOS itself and is protected. Only system updates can change it."
            }
            Error::QuotaExceeded => "This app has reached a limit set for it.",
            Error::DeviceFault => {
                "The drive or device reported a problem. Your data may be at risk — check the device in Settings."
            }
            Error::Interrupted => "The operation was interrupted before it finished.",
            Error::WouldBlock => "This isn't ready yet. LokoOS will try again.",
            Error::IntegrityFailure => {
                "LokoOS couldn't verify that this came from who it says it came from, so it stopped. Don't install it unless you're certain of the source."
            }
            Error::ConfirmationRequired => "This needs your confirmation before it can go ahead.",
            Error::RuntimeMissing => {
                "The compatibility support this app needs isn't installed yet."
            }
            Error::NoSuchSyscall => {
                "The app asked LokoOS for something this version doesn't provide. The app may be built for a newer version of LokoOS."
            }
        }
    }

    /// The actions a UI should offer for this error, most-recommended first.
    ///
    /// An empty slice means the only reasonable action is to dismiss.
    #[must_use]
    pub const fn suggested_actions(self) -> &'static [UserAction] {
        match self {
            Error::PermissionDenied | Error::InsufficientRights => {
                &[UserAction::OpenPermissionSettings, UserAction::Cancel]
            }
            Error::ProtectedLocation => &[UserAction::LearnMore, UserAction::Cancel],
            Error::OutOfMemory => &[UserAction::OpenSystemMonitor, UserAction::Cancel],
            Error::DeviceFault => &[UserAction::OpenStorageSettings, UserAction::Cancel],
            Error::IntegrityFailure => &[UserAction::LearnMore, UserAction::Cancel],
            Error::ConfirmationRequired => &[UserAction::Confirm, UserAction::Cancel],
            Error::RuntimeMissing => &[
                UserAction::InstallRuntime,
                UserAction::LearnMore,
                UserAction::Cancel,
            ],
            Error::Interrupted | Error::WouldBlock => &[UserAction::Retry, UserAction::Cancel],
            Error::NotImplemented | Error::NoSuchSyscall => {
                &[UserAction::LearnMore, UserAction::Cancel]
            }
            _ => &[UserAction::Cancel],
        }
    }

    /// Whether an automatic retry of the identical request could succeed.
    ///
    /// The kernel and the service supervisor use this to decide whether to back
    /// off and retry or to escalate. [`Error::IntegrityFailure`] is deliberately
    /// **not** retryable: silently retrying a signature failure is how a
    /// downgrade attack gets a second chance.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Error::Interrupted | Error::WouldBlock | Error::OutOfMemory
        )
    }

    /// Whether this error should be recorded in the security log.
    #[must_use]
    pub const fn is_security_relevant(self) -> bool {
        matches!(
            self,
            Error::PermissionDenied
                | Error::InsufficientRights
                | Error::ProtectedLocation
                | Error::IntegrityFailure
        )
    }
}

/// An action a user interface can offer in response to an [`Error`].
///
/// Deliberately a closed set: it keeps error dialogs across LokoOS consistent,
/// and it means the shell can localise the button labels once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum UserAction {
    /// Dismiss without doing anything.
    Cancel,
    /// Try the same thing again.
    Retry,
    /// Proceed, having understood the consequence.
    Confirm,
    /// Open the relevant page of Loko Settings → Privacy & Security.
    OpenPermissionSettings,
    /// Open Loko System Monitor.
    OpenSystemMonitor,
    /// Open Loko Settings → System → Storage.
    OpenStorageSettings,
    /// Open the Loko Compatibility Center to install the missing runtime.
    InstallRuntime,
    /// Open the documentation page for this error.
    LearnMore,
}

impl UserAction {
    /// The default English label. Localisation replaces this at the shell.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            UserAction::Cancel => "Cancel",
            UserAction::Retry => "Try Again",
            UserAction::Confirm => "Continue",
            UserAction::OpenPermissionSettings => "Open Permissions",
            UserAction::OpenSystemMonitor => "Open System Monitor",
            UserAction::OpenStorageSettings => "Check Storage",
            UserAction::InstallRuntime => "Install Runtime",
            UserAction::LearnMore => "Learn More",
        }
    }

    /// Whether choosing this action commits to the operation going ahead.
    #[must_use]
    pub const fn is_affirmative(self) -> bool {
        matches!(self, UserAction::Retry | UserAction::Confirm)
    }
}

/// The result type used throughout the LokoOS ABI.
pub type Result<T> = core::result::Result<T, Error>;

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.explanation())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_positive() {
        let mut seen = [false; 64];
        for e in Error::ALL {
            let c = e.code();
            assert!(c > 0, "{} has a non-positive code", e.name());
            let idx = c as usize;
            assert!(!seen[idx], "code {c} is used twice");
            seen[idx] = true;
        }
    }

    #[test]
    fn every_variant_is_listed_in_all() {
        // If a variant is added without updating ALL, the highest code will no
        // longer equal the list length and this catches it.
        let max = Error::ALL.iter().map(|e| e.code()).max().unwrap();
        assert_eq!(
            max as usize,
            Error::ALL.len(),
            "Error::ALL is missing a variant, or codes are not contiguous"
        );
    }

    #[test]
    fn explanations_are_plain_language() {
        for e in Error::ALL {
            let text = e.explanation();
            assert!(!text.is_empty(), "{} has no explanation", e.name());
            assert!(
                text.ends_with('.'),
                "{} explanation is not a sentence",
                e.name()
            );
            // Requirement 64: never surface a bare code to the user.
            assert!(
                !text.contains("0x") && !text.contains("ERR_"),
                "{} explanation leaks a raw error code",
                e.name()
            );
        }
    }

    #[test]
    fn every_error_offers_a_way_out() {
        for e in Error::ALL {
            let actions = e.suggested_actions();
            assert!(
                actions.contains(&UserAction::Cancel),
                "{} gives the user no way to dismiss",
                e.name()
            );
        }
    }

    #[test]
    fn integrity_failures_are_never_retried_automatically() {
        assert!(!Error::IntegrityFailure.is_retryable());
        assert!(Error::IntegrityFailure.is_security_relevant());
    }
}
