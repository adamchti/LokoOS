//! The LKOFS access policy.
//!
//! One function, [`evaluate`], answers every "may this subject do this to this
//! path?" question in LokoOS. It is pure: no I/O, no clock, no globals. That
//! makes it exhaustively testable, and it means the kernel, the installer, the
//! recovery environment and Linder's preview pane all reach the same verdict
//! for the same inputs — which is the only way a permission model stays
//! coherent as a system grows.
//!
//! The policy is deliberately written as an ordered list of rules where the
//! first match wins, and the most restrictive rules come first. Reading it
//! top to bottom is reading the security model.

use crate::path::LkoPathRef;
use crate::root::{Protection, Root, StorageCategory};
use loko_abi::Error;

/// What kind of principal is asking.
///
/// This is not a privilege *level* — LokoOS has no single ladder of
/// privilege. It is a statement of role, and each role is constrained in
/// different directions. A driver has authority over hardware that the shell
/// does not have, and no authority at all over user documents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum SubjectClass {
    /// The kernel itself. Present so that kernel-internal callers go through
    /// the same function rather than around it.
    Kernel,
    /// Loko Update, running a staged, verified update transaction. The only
    /// principal that may write to a [`Protection::SystemImmutable`] location,
    /// and only while a transaction is open.
    SystemUpdater,
    /// A LokoOS system service, such as the indexer or the network manager.
    SystemService,
    /// A device driver.
    Driver,
    /// A first-party shell component acting directly on the user's behalf:
    /// Linder, Loko Settings, Loko Terminal.
    UserShell,
    /// An installed application that has been granted file access.
    UserApp,
    /// An installed application with no file grant at all. Confined to its own
    /// container.
    SandboxedApp,
    /// Loko AI acting on the user's behalf. Reads within granted scope freely;
    /// every write goes through the user. See requirement 16.
    AiAgent,
    /// The Windows or macOS compatibility runtime, acting for a foreign
    /// application. Never trusted with system locations, because the code it
    /// is executing was not written for LokoOS's security model.
    CompatibilityRuntime,
}

/// What the subject wants to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Operation {
    /// Read metadata: existence, size, type, timestamps.
    Inspect,
    /// Read contents, or list a directory.
    Read,
    /// Execute a file, or traverse into a directory.
    Execute,
    /// Modify contents in place.
    Write,
    /// Create a new entry.
    Create,
    /// Remove an entry.
    Delete,
    /// Move or rename an entry.
    Rename,
    /// Change permissions or ownership.
    ChangePermissions,
}

impl Operation {
    /// Whether this operation changes persistent state.
    #[must_use]
    pub const fn is_mutating(self) -> bool {
        matches!(
            self,
            Operation::Write
                | Operation::Create
                | Operation::Delete
                | Operation::Rename
                | Operation::ChangePermissions
        )
    }

    /// Whether this operation can destroy data the user cannot get back.
    ///
    /// These are the operations requirement 16 says must never happen silently
    /// on Loko AI's initiative.
    #[must_use]
    pub const fn is_destructive(self) -> bool {
        matches!(
            self,
            Operation::Delete | Operation::Write | Operation::Rename
        )
    }
}

/// Why access was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DenyReason {
    /// The location is part of LokoOS and is replaced only by system updates.
    ProtectedLocation,
    /// The subject is confined to a container and this path is outside it.
    OutsideSandbox,
    /// The path belongs to a different user.
    CrossUser,
    /// The subject holds no grant covering this path.
    NoGrant,
    /// This role is never permitted this operation anywhere.
    RoleForbidden,
}

impl DenyReason {
    /// The ABI error a denial surfaces as.
    #[must_use]
    pub const fn as_error(self) -> Error {
        match self {
            DenyReason::ProtectedLocation => Error::ProtectedLocation,
            _ => Error::PermissionDenied,
        }
    }

    /// A plain-language explanation for the user.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            DenyReason::ProtectedLocation => {
                "This is part of LokoOS itself. Only a system update can change it."
            }
            DenyReason::OutsideSandbox => {
                "This app can only reach its own files. You can give it wider access in Settings."
            }
            DenyReason::CrossUser => "This belongs to someone else who uses this device.",
            DenyReason::NoGrant => "This app hasn't been given access to this location yet.",
            DenyReason::RoleForbidden => "This part of LokoOS isn't allowed to do that.",
        }
    }
}

/// What a confirmation prompt must tell the user before the action proceeds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Confirmation {
    /// An AI-initiated change to the user's own files. The prompt must name the
    /// files and offer a review step, per requirement 16.
    AiInitiatedChange,
    /// A foreign (Windows/macOS) application writing to the user's files.
    ForeignApplicationWrite,
    /// A permission or ownership change.
    PermissionChange,
}

/// The verdict.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Proceed.
    Allow,
    /// Proceed only after the user says yes to the given prompt.
    ///
    /// Callers must not treat this as `Allow`. The `#[must_use]` on
    /// [`Decision::is_allowed`] makes the distinction hard to drop by accident.
    Confirm(Confirmation),
    /// Refuse.
    Deny(DenyReason),
}

impl Decision {
    /// Whether the operation may proceed **right now**, with no further steps.
    ///
    /// [`Decision::Confirm`] is deliberately not allowed: a caller that wants
    /// to treat "ask the user" as "yes" has to say so explicitly.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Decision::Allow)
    }

    /// The ABI error to return, if this decision blocks the operation.
    pub const fn as_error(self) -> Option<Error> {
        match self {
            Decision::Allow => None,
            Decision::Confirm(_) => Some(Error::ConfirmationRequired),
            Decision::Deny(reason) => Some(reason.as_error()),
        }
    }
}

/// Who is asking, and what they have been granted.
#[derive(Clone, Copy, Debug)]
pub struct Subject<'a> {
    /// The subject's role.
    pub class: SubjectClass,
    /// The application identifier, e.g. `com.example.editor`. `None` for
    /// kernel and system roles.
    pub app_id: Option<&'a str>,
    /// The user this subject acts for. `None` for system roles that act for no
    /// particular user.
    pub user: Option<&'a str>,
    /// Paths the user has explicitly granted this subject access to.
    ///
    /// Grants are subtrees. A grant on `LKO/Users/Adam/Documents` covers
    /// everything beneath it and nothing above it.
    pub grants: &'a [LkoPathRef<'a>],
    /// Whether a verified update transaction is currently open.
    ///
    /// Only meaningful for [`SubjectClass::SystemUpdater`]. Outside a
    /// transaction, even the updater cannot write to protected locations, so a
    /// compromised updater process sitting idle has no standing authority.
    pub update_transaction_open: bool,
}

impl<'a> Subject<'a> {
    /// A subject with no grants at all.
    #[must_use]
    pub const fn new(class: SubjectClass) -> Self {
        Subject {
            class,
            app_id: None,
            user: None,
            grants: &[],
            update_transaction_open: false,
        }
    }

    /// Whether any grant covers `path`.
    #[must_use]
    pub fn has_grant_covering(&self, path: &LkoPathRef<'_>) -> bool {
        self.grants.iter().any(|g| path.is_within(g))
    }
}

/// The protection class actually in force at `path`.
///
/// Usually the root's own class, but some subtrees are stricter than their
/// root. Requirement 8 lists `Security` among the locations to protect;
/// `Security` is not a root, so it is handled here.
#[must_use]
pub fn effective_protection(path: &LkoPathRef<'_>) -> Protection {
    let Some(root) = path.root() else {
        // The volume root itself: creating a twentieth root is not a thing
        // applications may do.
        return Protection::SystemImmutable;
    };

    let first = path.tail().next();

    match (root, first) {
        // `LKO/Config/Security` holds firewall rules, credential policy and
        // signing trust anchors. It is as sensitive as anything under System.
        (Root::Config, Some(c)) if c.eq_ignore_ascii_case("Security") => {
            Protection::SystemImmutable
        }
        (Root::Services, Some(c)) if c.eq_ignore_ascii_case("Security") => {
            Protection::SystemImmutable
        }
        // Loko AI's memory is the user's, not the system's: they must be able
        // to read and erase it. Requirement 15 makes this a privacy control.
        (Root::Ai, Some(c)) if c.eq_ignore_ascii_case("Memory") => Protection::UserOwned,
        // Model weights and the AI runtime are system-managed, as elsewhere.
        _ => root.protection(),
    }
}

/// The storage bucket `path` contributes to.
///
/// Refines [`Root::storage_category`] where a root spans more than one bucket.
/// `LKO/Runtime/Loko` is part of the base OS; `LKO/Runtime/Windows` is an
/// optional download and must not count against the 2 GB budget.
#[must_use]
pub fn storage_category_of(path: &LkoPathRef<'_>) -> StorageCategory {
    let Some(root) = path.root() else {
        return StorageCategory::BaseSystem;
    };
    if root == Root::Runtime {
        return match path.tail().next() {
            Some(c) if c.eq_ignore_ascii_case("Windows") || c.eq_ignore_ascii_case("Mac") => {
                StorageCategory::Compatibility
            }
            _ => StorageCategory::BaseSystem,
        };
    }
    root.storage_category()
}

/// Decides whether `subject` may perform `operation` on `path`.
///
/// Rules are evaluated in order; the first that applies wins.
#[must_use]
pub fn evaluate(subject: &Subject<'_>, path: &LkoPathRef<'_>, operation: Operation) -> Decision {
    // Rule 0. The kernel is the thing enforcing the policy; it is not subject
    // to it. Kept explicit so that the set of principals with unlimited
    // authority is one line long and visible.
    if subject.class == SubjectClass::Kernel {
        return Decision::Allow;
    }

    let protection = effective_protection(path);

    // Rule 1. Protected locations. Nothing writes here except a verified update
    // transaction — not the shell, not a service, not a driver, not the user.
    // This is what makes "LokoOS itself cannot be tampered with while running"
    // a property rather than an aspiration.
    if protection == Protection::SystemImmutable && operation.is_mutating() {
        let updating =
            subject.class == SubjectClass::SystemUpdater && subject.update_transaction_open;
        if !updating {
            return Decision::Deny(DenyReason::ProtectedLocation);
        }
        return Decision::Allow;
    }

    // Rule 2. Compatibility runtimes never touch system locations at all, even
    // to read. A Windows application enumerating LokoOS internals is a
    // fingerprinting surface with no legitimate use.
    if subject.class == SubjectClass::CompatibilityRuntime
        && matches!(
            path.root(),
            Some(Root::System | Root::Drivers | Root::Boot | Root::Recovery | Root::Services)
        )
    {
        return Decision::Deny(DenyReason::RoleForbidden);
    }

    // Rule 3. Drivers have authority over hardware, not over files. They may
    // read their own root and write logs; everything else is out of role.
    if subject.class == SubjectClass::Driver {
        return match (path.root(), operation.is_mutating()) {
            (Some(Root::Drivers), false) => Decision::Allow,
            (Some(Root::Logs), _) => Decision::Allow,
            (Some(Root::Temp), _) => Decision::Allow,
            _ => Decision::Deny(DenyReason::RoleForbidden),
        };
    }

    // Rule 4. Cross-user isolation. A subject acting for one user may not reach
    // another user's home directory, whatever else it has been granted.
    if path.root() == Some(Root::Users) {
        if let Some(owner) = path.tail().next() {
            let acting_for_owner = subject.user.is_some_and(|u| u.eq_ignore_ascii_case(owner));
            let system_role = matches!(
                subject.class,
                SubjectClass::SystemService | SubjectClass::SystemUpdater
            );
            if !acting_for_owner && !system_role {
                return Decision::Deny(DenyReason::CrossUser);
            }
        }
    }

    // Rule 5. Volatile scratch space is open to everyone that got this far.
    // Temp and Cache hold nothing that isn't regenerable, and forcing every app
    // to negotiate a grant for scratch space would train users to click yes.
    if matches!(path.root(), Some(Root::Temp | Root::Cache)) {
        return Decision::Allow;
    }

    // Rule 6. Sandboxed applications. Confined to their own container, with
    // read access to their own installation directory.
    if subject.class == SubjectClass::SandboxedApp {
        // Compared component-wise against the app id and user name rather than
        // against a built path string, so there is no string prefix anywhere in
        // the containment check.
        let (user, app_id) = (subject.user, subject.app_id);

        let in_own_appdata = match (path.root(), user, app_id) {
            (Some(Root::Users), Some(u), Some(app)) => {
                let mut tail = path.tail();
                matches!(tail.next(), Some(o) if o.eq_ignore_ascii_case(u))
                    && matches!(tail.next(), Some(d) if d.eq_ignore_ascii_case("AppData"))
                    && matches!(tail.next(), Some(a) if a.eq_ignore_ascii_case(app))
            }
            _ => false,
        };
        let in_own_install_dir = match (path.root(), app_id) {
            (Some(Root::Apps), Some(app)) => {
                matches!(path.tail().next(), Some(a) if a.eq_ignore_ascii_case(app))
            }
            _ => false,
        };

        // Its own container: full access. Its own installation directory: read
        // only, so that an application cannot rewrite its own installed code
        // and thereby survive an uninstall or defeat signature checking.
        let permitted = in_own_appdata || (in_own_install_dir && !operation.is_mutating());
        return if permitted {
            Decision::Allow
        } else {
            Decision::Deny(DenyReason::OutsideSandbox)
        };
    }

    // Rule 7. System-managed locations are readable by anything that got this
    // far — LokoOS is inspectable by the person who owns the machine — but
    // writable only by system roles.
    if protection == Protection::SystemManaged && operation.is_mutating() {
        let system_role = matches!(
            subject.class,
            SubjectClass::SystemService | SubjectClass::SystemUpdater | SubjectClass::UserShell
        );
        if !system_role {
            return Decision::Deny(DenyReason::NoGrant);
        }
    }

    // Rule 8. User-owned locations need an explicit grant, except for the
    // shell, which is the user operating their own machine directly.
    if protection == Protection::UserOwned
        && subject.class != SubjectClass::UserShell
        && !subject.has_grant_covering(path)
    {
        return Decision::Deny(DenyReason::NoGrant);
    }

    // Rule 9. Changing permissions is always a decision the user makes.
    if operation == Operation::ChangePermissions {
        return Decision::Confirm(Confirmation::PermissionChange);
    }

    // Rule 10. Requirement 16: Loko AI never destroys or overwrites anything
    // on its own initiative. It may read freely within what it was granted, and
    // it may create new files — but changing or removing something that already
    // exists goes to the user, with the files named.
    if subject.class == SubjectClass::AiAgent && operation.is_destructive() {
        return Decision::Confirm(Confirmation::AiInitiatedChange);
    }

    // Rule 11. Foreign applications get the same treatment for writes to user
    // data. Their code predates LokoOS's model and cannot be assumed to respect
    // it, so the user stays in the loop.
    if subject.class == SubjectClass::CompatibilityRuntime && operation.is_mutating() {
        return Decision::Confirm(Confirmation::ForeignApplicationWrite);
    }

    Decision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::LkoPathRef;

    fn p(s: &str) -> LkoPathRef<'_> {
        LkoPathRef::parse(s).expect("test path should parse")
    }

    const ALL_OPS: &[Operation] = &[
        Operation::Inspect,
        Operation::Read,
        Operation::Execute,
        Operation::Write,
        Operation::Create,
        Operation::Delete,
        Operation::Rename,
        Operation::ChangePermissions,
    ];

    fn app<'a>(id: &'a str, user: &'a str, grants: &'a [LkoPathRef<'a>]) -> Subject<'a> {
        Subject {
            class: SubjectClass::UserApp,
            app_id: Some(id),
            user: Some(user),
            grants,
            update_transaction_open: false,
        }
    }

    // -- protected locations ------------------------------------------------

    #[test]
    fn nothing_writes_to_protected_roots_except_an_open_update_transaction() {
        let targets = [
            "LKO/System/Kernel/loko-kernel",
            "LKO/Drivers/Graphics/amdgpu.loko",
            "LKO/Boot/loko-boot.efi",
            "LKO/Recovery/recovery.img",
            "LKO/Config/Security/trust-anchors.db",
        ];
        let roles = [
            SubjectClass::SystemService,
            SubjectClass::UserShell,
            SubjectClass::UserApp,
            SubjectClass::SandboxedApp,
            SubjectClass::AiAgent,
            SubjectClass::Driver,
            SubjectClass::CompatibilityRuntime,
        ];
        for target in targets {
            for role in roles {
                let subject = Subject {
                    class: role,
                    app_id: Some("com.example.app"),
                    user: Some("Adam"),
                    grants: &[],
                    // Even while falsely claiming an update transaction is
                    // open: the class check is what matters, not the flag.
                    update_transaction_open: true,
                };
                for op in ALL_OPS.iter().filter(|o| o.is_mutating()) {
                    let decision = evaluate(&subject, &p(target), *op);
                    assert!(
                        !decision.is_allowed(),
                        "{role:?} was allowed to {op:?} {target}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_updater_may_write_protected_roots_only_inside_a_transaction() {
        let path = p("LKO/System/Kernel/loko-kernel");
        let mut updater = Subject::new(SubjectClass::SystemUpdater);

        updater.update_transaction_open = false;
        assert_eq!(
            evaluate(&updater, &path, Operation::Write),
            Decision::Deny(DenyReason::ProtectedLocation),
            "an idle updater must hold no standing authority"
        );

        updater.update_transaction_open = true;
        assert_eq!(evaluate(&updater, &path, Operation::Write), Decision::Allow);
    }

    #[test]
    fn protected_roots_stay_readable() {
        // Inspectability is a feature: the user owns the machine. Only writes
        // are locked down.
        let shell = Subject::new(SubjectClass::UserShell);
        for op in [Operation::Inspect, Operation::Read, Operation::Execute] {
            assert!(
                evaluate(&shell, &p("LKO/System/Core"), op).is_allowed(),
                "{op:?} on a system path should be readable"
            );
        }
    }

    #[test]
    fn case_variants_of_protected_paths_are_still_protected() {
        let shell = Subject::new(SubjectClass::UserShell);
        for spelling in [
            "LKO/system/Kernel",
            "LKO/SYSTEM/Kernel",
            "LKO/config/security/policy.db",
            "LKO/Config/SECURITY/policy.db",
        ] {
            assert_eq!(
                evaluate(&shell, &p(spelling), Operation::Write),
                Decision::Deny(DenyReason::ProtectedLocation),
                "{spelling} must be protected regardless of case"
            );
        }
    }

    // -- sandboxing ---------------------------------------------------------

    #[test]
    fn a_sandboxed_app_reaches_only_its_own_container() {
        let subject = Subject {
            class: SubjectClass::SandboxedApp,
            app_id: Some("com.example.editor"),
            user: Some("Adam"),
            grants: &[],
            update_transaction_open: false,
        };

        // Its own AppData: full access.
        assert!(evaluate(
            &subject,
            &p("LKO/Users/Adam/AppData/com.example.editor/state.db"),
            Operation::Write
        )
        .is_allowed());

        // Its own install directory: read only.
        assert!(evaluate(
            &subject,
            &p("LKO/Apps/com.example.editor/resources/icon.png"),
            Operation::Read
        )
        .is_allowed());
        assert_eq!(
            evaluate(
                &subject,
                &p("LKO/Apps/com.example.editor/resources/icon.png"),
                Operation::Write
            ),
            Decision::Deny(DenyReason::OutsideSandbox),
            "an app must not be able to rewrite its own installed code"
        );

        // Everything else.
        for outside in [
            "LKO/Users/Adam/Documents/taxes.pdf",
            "LKO/Users/Adam/AppData/com.other.app/secrets.db",
            "LKO/Apps/com.other.app/binary",
            "LKO/Lowser/Profiles/default/cookies.db",
            "LKO/AI/Memory/conversations.db",
        ] {
            for op in ALL_OPS {
                assert!(
                    !evaluate(&subject, &p(outside), *op).is_allowed(),
                    "sandboxed app reached {outside} with {op:?}"
                );
            }
        }
    }

    #[test]
    fn a_sandboxed_app_cannot_escape_via_a_similar_prefix() {
        let subject = Subject {
            class: SubjectClass::SandboxedApp,
            app_id: Some("com.example.editor"),
            user: Some("Adam"),
            grants: &[],
            update_transaction_open: false,
        };
        // The classic escape: a sibling directory whose name starts with the
        // app's own id.
        for near_miss in [
            "LKO/Users/Adam/AppData/com.example.editor.evil/x",
            "LKO/Apps/com.example.editor2/binary",
        ] {
            assert!(
                !evaluate(&subject, &p(near_miss), Operation::Read).is_allowed(),
                "escaped into {near_miss}"
            );
        }
    }

    #[test]
    fn a_sandboxed_app_still_gets_scratch_space() {
        let subject = Subject {
            class: SubjectClass::SandboxedApp,
            app_id: Some("com.example.editor"),
            user: Some("Adam"),
            grants: &[],
            update_transaction_open: false,
        };
        assert!(evaluate(&subject, &p("LKO/Temp/work"), Operation::Create).is_allowed());
        assert!(evaluate(&subject, &p("LKO/Cache/thumbs"), Operation::Write).is_allowed());
    }

    // -- grants -------------------------------------------------------------

    #[test]
    fn a_granted_app_reaches_exactly_its_grant() {
        let docs = p("LKO/Users/Adam/Documents");
        let grants = [docs];
        let subject = app("com.example.editor", "Adam", &grants);

        assert!(evaluate(
            &subject,
            &p("LKO/Users/Adam/Documents/notes/todo.md"),
            Operation::Write
        )
        .is_allowed());

        assert_eq!(
            evaluate(
                &subject,
                &p("LKO/Users/Adam/Pictures/a.png"),
                Operation::Read
            ),
            Decision::Deny(DenyReason::NoGrant),
            "a grant on Documents must not reach Pictures"
        );
        assert_eq!(
            evaluate(&subject, &p("LKO/Users/Adam"), Operation::Read),
            Decision::Deny(DenyReason::NoGrant),
            "a grant must not reach upward"
        );
    }

    #[test]
    fn grants_do_not_cross_users() {
        let docs = p("LKO/Users/Bea/Documents");
        let grants = [docs];
        // Adam's app holding a grant that names Bea's directory: the grant is
        // covering, but the cross-user rule fires first.
        let subject = app("com.example.editor", "Adam", &grants);
        assert_eq!(
            evaluate(
                &subject,
                &p("LKO/Users/Bea/Documents/a.txt"),
                Operation::Read
            ),
            Decision::Deny(DenyReason::CrossUser)
        );
    }

    #[test]
    fn the_shell_does_not_need_a_grant_for_its_own_users_files() {
        let shell = Subject {
            class: SubjectClass::UserShell,
            app_id: None,
            user: Some("Adam"),
            grants: &[],
            update_transaction_open: false,
        };
        assert!(evaluate(
            &shell,
            &p("LKO/Users/Adam/Documents/a.txt"),
            Operation::Delete
        )
        .is_allowed());
        assert_eq!(
            evaluate(&shell, &p("LKO/Users/Bea/Documents/a.txt"), Operation::Read),
            Decision::Deny(DenyReason::CrossUser),
            "even the shell respects user boundaries"
        );
    }

    // -- Loko AI ------------------------------------------------------------

    #[test]
    fn requirement_16_ai_never_destroys_silently() {
        let docs = p("LKO/Users/Adam/Documents");
        let grants = [docs];
        let ai = Subject {
            class: SubjectClass::AiAgent,
            app_id: Some("loko.ai"),
            user: Some("Adam"),
            grants: &grants,
            update_transaction_open: false,
        };
        let target = p("LKO/Users/Adam/Documents/report.md");

        // Reading is free within the grant.
        assert!(evaluate(&ai, &target, Operation::Read).is_allowed());
        assert!(evaluate(&ai, &target, Operation::Inspect).is_allowed());

        // Anything that could lose data goes to the user.
        for op in [Operation::Delete, Operation::Write, Operation::Rename] {
            assert_eq!(
                evaluate(&ai, &target, op),
                Decision::Confirm(Confirmation::AiInitiatedChange),
                "AI {op:?} must require confirmation"
            );
        }

        // Creating something new does not destroy anything, so it proceeds:
        // "save this summary to Documents" should not need a dialog.
        assert!(evaluate(
            &ai,
            &p("LKO/Users/Adam/Documents/summary.md"),
            Operation::Create
        )
        .is_allowed());
    }

    #[test]
    fn ai_memory_belongs_to_the_user() {
        // Requirement 15: the user can inspect and erase Loko AI's memory.
        let shell = Subject {
            class: SubjectClass::UserShell,
            app_id: None,
            user: Some("Adam"),
            grants: &[],
            update_transaction_open: false,
        };
        assert_eq!(
            effective_protection(&p("LKO/AI/Memory/conversations.db")),
            Protection::UserOwned
        );
        assert!(evaluate(
            &shell,
            &p("LKO/AI/Memory/conversations.db"),
            Operation::Delete
        )
        .is_allowed());
        // Model weights are not the user's to edit in place, though.
        assert_eq!(
            effective_protection(&p("LKO/AI/Models/local-7b.gguf")),
            Protection::SystemManaged
        );
    }

    // -- roles --------------------------------------------------------------

    #[test]
    fn a_driver_has_no_authority_over_files() {
        let driver = Subject::new(SubjectClass::Driver);
        assert!(evaluate(
            &driver,
            &p("LKO/Drivers/Graphics/info.toml"),
            Operation::Read
        )
        .is_allowed());
        assert!(evaluate(&driver, &p("LKO/Logs/drivers/gpu.log"), Operation::Write).is_allowed());
        for forbidden in [
            "LKO/Users/Adam/Documents/a.txt",
            "LKO/Lowser/Profiles/default/cookies.db",
            "LKO/AI/Memory/conversations.db",
        ] {
            assert_eq!(
                evaluate(&driver, &p(forbidden), Operation::Read),
                Decision::Deny(DenyReason::RoleForbidden),
                "a driver reached {forbidden}"
            );
        }
    }

    #[test]
    fn compatibility_runtimes_cannot_see_lokoos_internals() {
        let wine = Subject {
            class: SubjectClass::CompatibilityRuntime,
            app_id: Some("windows.runtime"),
            user: Some("Adam"),
            grants: &[],
            update_transaction_open: false,
        };
        for internal in [
            "LKO/System/Core",
            "LKO/Drivers/Graphics",
            "LKO/Boot/loko-boot.efi",
            "LKO/Recovery/recovery.img",
            "LKO/Services/Update",
        ] {
            assert_eq!(
                evaluate(&wine, &p(internal), Operation::Inspect),
                Decision::Deny(DenyReason::RoleForbidden),
                "the Windows runtime could enumerate {internal}"
            );
        }
    }

    #[test]
    fn foreign_application_writes_go_through_the_user() {
        let docs = p("LKO/Users/Adam/Documents");
        let grants = [docs];
        let wine = Subject {
            class: SubjectClass::CompatibilityRuntime,
            app_id: Some("windows.runtime"),
            user: Some("Adam"),
            grants: &grants,
            update_transaction_open: false,
        };
        let target = p("LKO/Users/Adam/Documents/sheet.xlsx");
        assert!(evaluate(&wine, &target, Operation::Read).is_allowed());
        assert_eq!(
            evaluate(&wine, &target, Operation::Write),
            Decision::Confirm(Confirmation::ForeignApplicationWrite)
        );
    }

    #[test]
    fn permission_changes_always_ask() {
        let docs = p("LKO/Users/Adam/Documents");
        let grants = [docs];
        for class in [
            SubjectClass::UserShell,
            SubjectClass::UserApp,
            SubjectClass::SystemService,
        ] {
            let subject = Subject {
                class,
                app_id: Some("x"),
                user: Some("Adam"),
                grants: &grants,
                update_transaction_open: false,
            };
            assert_eq!(
                evaluate(
                    &subject,
                    &p("LKO/Users/Adam/Documents/a.txt"),
                    Operation::ChangePermissions
                ),
                Decision::Confirm(Confirmation::PermissionChange),
                "{class:?} changed permissions without asking"
            );
        }
    }

    // -- invariants ---------------------------------------------------------

    #[test]
    fn confirm_is_never_treated_as_allow() {
        assert!(!Decision::Confirm(Confirmation::AiInitiatedChange).is_allowed());
        assert_eq!(
            Decision::Confirm(Confirmation::AiInitiatedChange).as_error(),
            Some(Error::ConfirmationRequired)
        );
    }

    #[test]
    fn every_denial_maps_to_a_usable_error_and_explanation() {
        for reason in [
            DenyReason::ProtectedLocation,
            DenyReason::OutsideSandbox,
            DenyReason::CrossUser,
            DenyReason::NoGrant,
            DenyReason::RoleForbidden,
        ] {
            // Every denial must be auditable, or a probing attacker leaves no
            // trace in the security log.
            assert!(
                reason.as_error().is_security_relevant(),
                "{reason:?} maps to an error that is not logged as security-relevant"
            );
            assert!(reason.explanation().ends_with('.'));
        }
    }

    #[test]
    fn storage_split_keeps_optional_runtimes_out_of_the_base_budget() {
        assert_eq!(
            storage_category_of(&p("LKO/Runtime/Loko/libloko.so")),
            StorageCategory::BaseSystem
        );
        for optional in ["LKO/Runtime/Windows/wine", "LKO/Runtime/Mac/frameworks"] {
            assert_eq!(
                storage_category_of(&p(optional)),
                StorageCategory::Compatibility,
                "{optional} must not count against the base OS budget"
            );
            assert!(!storage_category_of(&p(optional)).counts_toward_base_os_budget());
        }
    }

    #[test]
    fn no_role_can_mutate_the_volume_root() {
        for class in [
            SubjectClass::SystemService,
            SubjectClass::UserShell,
            SubjectClass::UserApp,
            SubjectClass::AiAgent,
        ] {
            let subject = Subject::new(class);
            assert_eq!(
                evaluate(&subject, &LkoPathRef::ROOT, Operation::Create),
                Decision::Deny(DenyReason::ProtectedLocation),
                "{class:?} could create a new top-level root"
            );
        }
    }
}
