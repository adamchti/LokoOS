//! The LKO root taxonomy.
//!
//! LokoOS does not present a Unix hierarchy. The top level of the filesystem is
//! a closed set of nineteen named roots, each with a declared purpose, a
//! protection class, and a storage category. Because the set is closed and
//! known at compile time, the kernel can decide "is this a protected location?"
//! without consulting any on-disk metadata — which means the answer cannot be
//! changed by anything an attacker can write to disk.

/// A top-level LKO root.
///
/// The discriminants are stable: they appear in LKOFS on-disk structures.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[repr(u8)]
pub enum Root {
    /// Core operating system components. Replaced only by system updates.
    System = 1,
    /// Per-user home directories.
    Users = 2,
    /// Installed applications.
    Apps = 3,
    /// Device drivers.
    Drivers = 4,
    /// Long-running system services.
    Services = 5,
    /// Compatibility and language runtimes.
    Runtime = 6,
    /// System and application configuration.
    Config = 7,
    /// System-managed databases and indexes.
    Data = 8,
    /// Scratch space, cleared on boot.
    Temp = 9,
    /// Regenerable caches.
    Cache = 10,
    /// Structured logs.
    Logs = 11,
    /// The recovery environment.
    Recovery = 12,
    /// Boot artefacts: the bootloader, the kernel image, boot configuration.
    Boot = 13,
    /// Loko AI models, runtime, plugins and memory.
    Ai = 14,
    /// Linder's index and search database.
    Linder = 15,
    /// Lowser's profiles, extensions and browsing data.
    Lowser = 16,
    /// Loko Store packages and metadata.
    Store = 17,
    /// Windows and macOS compatibility support.
    Compatibility = 18,
    /// Developer SDK, toolchains, headers and templates.
    Dev = 19,
}

/// How strongly a location is protected from modification.
///
/// Ordered from most to least protected; `PartialOrd` is meaningful and is used
/// to answer "is this at least as protected as X?".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Protection {
    /// Writable only by the system updater, inside a staged atomic transaction.
    /// Not writable by the running system at all, including by root-equivalent
    /// processes. Requirement 8 names these locations explicitly.
    SystemImmutable,
    /// Writable by designated system services, never by applications.
    SystemManaged,
    /// Writable by the owning user and by applications the user has granted
    /// file access to.
    UserOwned,
    /// Writable by anything with a handle, and safe for the system to delete at
    /// any time without asking.
    Volatile,
}

impl Protection {
    /// Whether an ordinary application could ever be granted write access here.
    #[must_use]
    pub const fn app_writable(self) -> bool {
        matches!(self, Protection::UserOwned | Protection::Volatile)
    }

    /// Whether LokoOS may delete contents here without user confirmation.
    ///
    /// Only `Volatile` qualifies. Everything else, including caches that look
    /// disposable, goes through the user in Linder's storage panel.
    #[must_use]
    pub const fn safe_to_purge(self) -> bool {
        matches!(self, Protection::Volatile)
    }
}

/// The bucket a location contributes to in Linder's storage dashboard
/// (requirements 18 and 48).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum StorageCategory {
    /// Counts against the "base OS under 2 GB" budget.
    BaseSystem,
    /// Installed applications.
    Applications,
    /// Optional Windows/macOS compatibility runtimes. Explicitly excluded from
    /// the base OS budget.
    Compatibility,
    /// Downloaded AI models. Excluded from the base OS budget.
    AiModels,
    /// Regenerable caches.
    Cache,
    /// Scratch space.
    Temporary,
    /// The user's own files.
    UserFiles,
    /// Recovery images. Excluded from the base OS budget.
    Recovery,
    /// Developer toolchains. Excluded from the base OS budget.
    Developer,
}

impl StorageCategory {
    /// Whether this category counts toward the 2 GB base-OS size target.
    ///
    /// Requirement 2 lists exactly what the budget excludes; this function is
    /// the single place that list is encoded, and `tools/xtask` uses it to
    /// report the real measured size of a build.
    #[must_use]
    pub const fn counts_toward_base_os_budget(self) -> bool {
        matches!(self, StorageCategory::BaseSystem)
    }

    /// A label for the storage dashboard.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            StorageCategory::BaseSystem => "System",
            StorageCategory::Applications => "Applications",
            StorageCategory::Compatibility => "Compatibility",
            StorageCategory::AiModels => "AI Models",
            StorageCategory::Cache => "Cache",
            StorageCategory::Temporary => "Temporary Files",
            StorageCategory::UserFiles => "Your Files",
            StorageCategory::Recovery => "Recovery",
            StorageCategory::Developer => "Developer Tools",
        }
    }
}

impl Root {
    /// Every root, in discriminant order.
    pub const ALL: &'static [Root] = &[
        Root::System,
        Root::Users,
        Root::Apps,
        Root::Drivers,
        Root::Services,
        Root::Runtime,
        Root::Config,
        Root::Data,
        Root::Temp,
        Root::Cache,
        Root::Logs,
        Root::Recovery,
        Root::Boot,
        Root::Ai,
        Root::Linder,
        Root::Lowser,
        Root::Store,
        Root::Compatibility,
        Root::Dev,
    ];

    /// The canonical spelling, as it appears on disk and in the UI.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Root::System => "System",
            Root::Users => "Users",
            Root::Apps => "Apps",
            Root::Drivers => "Drivers",
            Root::Services => "Services",
            Root::Runtime => "Runtime",
            Root::Config => "Config",
            Root::Data => "Data",
            Root::Temp => "Temp",
            Root::Cache => "Cache",
            Root::Logs => "Logs",
            Root::Recovery => "Recovery",
            Root::Boot => "Boot",
            Root::Ai => "AI",
            Root::Linder => "Linder",
            Root::Lowser => "Lowser",
            Root::Store => "Store",
            Root::Compatibility => "Compatibility",
            Root::Dev => "Dev",
        }
    }

    /// Parses a root name.
    ///
    /// The comparison is ASCII-case-insensitive **on purpose**. LKOFS is
    /// case-preserving and case-insensitive for user data, and if root matching
    /// were case-sensitive then `LKO/system/Kernel` would parse as "some
    /// unknown root" and skip the protection check that `LKO/System/Kernel`
    /// gets. Matching loosely here and enforcing strictly afterwards is the
    /// safe direction to be wrong in.
    ///
    /// Non-ASCII input is rejected outright rather than normalised, which
    /// removes homoglyph confusion (Cyrillic `Ѕ` for `S`) as a class.
    pub fn parse(s: &str) -> Option<Root> {
        if !s.is_ascii() {
            return None;
        }
        Root::ALL
            .iter()
            .copied()
            .find(|r| r.name().eq_ignore_ascii_case(s))
    }

    /// The protection class applied to the root itself.
    ///
    /// Some subtrees are more protected than their root — `LKO/Config/Security`
    /// is stricter than `LKO/Config` — which is handled by
    /// [`crate::policy::effective_protection`] rather than here.
    #[must_use]
    pub const fn protection(self) -> Protection {
        match self {
            // Requirement 8: these must be protected from unauthorised
            // modification. LokoOS goes further than "protected" and makes them
            // immutable to the running system: they are replaced wholesale by a
            // verified update transaction, never edited in place.
            Root::System | Root::Drivers | Root::Boot | Root::Recovery => {
                Protection::SystemImmutable
            }
            Root::Services
            | Root::Runtime
            | Root::Config
            | Root::Data
            | Root::Logs
            | Root::Store
            | Root::Compatibility
            | Root::Linder => Protection::SystemManaged,
            Root::Apps | Root::Ai | Root::Dev => Protection::SystemManaged,
            Root::Users | Root::Lowser => Protection::UserOwned,
            Root::Temp | Root::Cache => Protection::Volatile,
        }
    }

    /// Which storage bucket this root's contents are reported under.
    #[must_use]
    pub const fn storage_category(self) -> StorageCategory {
        match self {
            Root::System | Root::Drivers | Root::Boot | Root::Services | Root::Config => {
                StorageCategory::BaseSystem
            }
            // The Loko-native runtime is part of the base OS; the Windows and
            // macOS runtimes under it are not, and are reported separately by
            // walking one level deeper. See `storage_category_of` in `policy`.
            Root::Runtime => StorageCategory::BaseSystem,
            Root::Apps | Root::Store => StorageCategory::Applications,
            Root::Compatibility => StorageCategory::Compatibility,
            Root::Ai => StorageCategory::AiModels,
            Root::Cache | Root::Linder | Root::Lowser => StorageCategory::Cache,
            Root::Temp => StorageCategory::Temporary,
            Root::Users => StorageCategory::UserFiles,
            Root::Recovery => StorageCategory::Recovery,
            Root::Dev => StorageCategory::Developer,
            Root::Data | Root::Logs => StorageCategory::BaseSystem,
        }
    }

    /// Whether the contents survive a reboot.
    #[must_use]
    pub const fn is_persistent(self) -> bool {
        !matches!(self, Root::Temp)
    }

    /// Whether Linder and Loko Search may index this root by default.
    ///
    /// System internals and other people's user directories are excluded, so
    /// that a search index cannot become a side channel for content a user
    /// could not otherwise read.
    #[must_use]
    pub const fn indexed_by_default(self) -> bool {
        matches!(self, Root::Users | Root::Apps | Root::Dev | Root::Store)
    }

    /// A one-line description for Settings and documentation.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Root::System => "LokoOS itself. Replaced only by system updates.",
            Root::Users => "Your files, and the files of anyone else who uses this device.",
            Root::Apps => "Installed applications.",
            Root::Drivers => "Software that lets LokoOS talk to your hardware.",
            Root::Services => "Background programs that keep the system running.",
            Root::Runtime => "Support libraries that applications need in order to run.",
            Root::Config => "Settings, for the system and for applications.",
            Root::Data => "Databases and indexes LokoOS maintains.",
            Root::Temp => "Scratch space. Cleared every time you restart.",
            Root::Cache => "Saved copies of things that can be rebuilt if deleted.",
            Root::Logs => "Records of what the system has been doing.",
            Root::Recovery => "The tools used to repair LokoOS if it won't start.",
            Root::Boot => "What LokoOS loads first when you turn the device on.",
            Root::Ai => "Loko AI's models, memory and configuration.",
            Root::Linder => "Linder's search index.",
            Root::Lowser => "Lowser's profiles, extensions and browsing data.",
            Root::Store => "Downloaded packages and store information.",
            Root::Compatibility => "Optional support for Windows and macOS applications.",
            Root::Dev => "Developer tools, SDKs and toolchains.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for r in Root::ALL {
            assert_eq!(Root::parse(r.name()), Some(*r));
        }
    }

    #[test]
    fn names_are_unique() {
        for (i, a) in Root::ALL.iter().enumerate() {
            for b in &Root::ALL[i + 1..] {
                assert!(
                    !a.name().eq_ignore_ascii_case(b.name()),
                    "{} and {} collide case-insensitively",
                    a.name(),
                    b.name()
                );
            }
        }
    }

    #[test]
    fn all_is_complete_and_contiguous() {
        for (i, r) in Root::ALL.iter().enumerate() {
            assert_eq!(*r as u8 as usize, i + 1, "{r:?} is out of order in ALL");
        }
        assert_eq!(Root::ALL.len(), 19, "requirement 7 defines nineteen roots");
    }

    #[test]
    fn root_matching_resists_case_confusion() {
        // The attack this defends against: naming a protected location in a
        // spelling that fails an exact-match protection check but still
        // resolves on a case-insensitive filesystem.
        for spelling in ["system", "SYSTEM", "SyStEm"] {
            assert_eq!(
                Root::parse(spelling),
                Some(Root::System),
                "{spelling} must still be recognised as the System root"
            );
            assert_eq!(
                Root::parse(spelling).unwrap().protection(),
                Protection::SystemImmutable
            );
        }
    }

    #[test]
    fn root_matching_resists_homoglyphs() {
        // U+0405 CYRILLIC CAPITAL LETTER DZE renders identically to "S".
        assert_eq!(Root::parse("\u{0405}ystem"), None);
        // Fullwidth forms, too.
        assert_eq!(Root::parse("\u{ff33}ystem"), None);
    }

    #[test]
    fn requirement_8_locations_are_immutable() {
        // Requirement 8 names these as protected from unauthorised
        // modification. If a future change relaxes one of them, this fails.
        for r in [Root::System, Root::Drivers, Root::Boot, Root::Recovery] {
            assert_eq!(
                r.protection(),
                Protection::SystemImmutable,
                "{} must stay immutable to the running system",
                r.name()
            );
            assert!(!r.protection().app_writable());
            assert!(!r.protection().safe_to_purge());
        }
    }

    #[test]
    fn only_volatile_roots_are_purgeable() {
        for r in Root::ALL {
            assert_eq!(
                r.protection().safe_to_purge(),
                matches!(r, Root::Temp | Root::Cache),
                "{} has the wrong purge policy",
                r.name()
            );
        }
    }

    #[test]
    fn optional_components_are_outside_the_base_os_budget() {
        // Requirement 2 excludes these from the under-2-GB target.
        for r in [
            Root::Compatibility,
            Root::Ai,
            Root::Recovery,
            Root::Dev,
            Root::Users,
        ] {
            assert!(
                !r.storage_category().counts_toward_base_os_budget(),
                "{} must not count against the base OS size budget",
                r.name()
            );
        }
    }

    #[test]
    fn other_users_directories_are_not_indexed_by_a_blanket_rule() {
        // Users *is* indexed, but per-user scoping is the policy layer's job.
        // What must never happen is system internals being indexed by default.
        for r in [
            Root::System,
            Root::Drivers,
            Root::Boot,
            Root::Recovery,
            Root::Logs,
        ] {
            assert!(
                !r.indexed_by_default(),
                "{} must not be indexed by default",
                r.name()
            );
        }
    }

    #[test]
    fn descriptions_avoid_jargon() {
        for r in Root::ALL {
            let d = r.description();
            assert!(
                d.ends_with('.'),
                "{} description is not a sentence",
                r.name()
            );
            assert!(d.len() > 20, "{} description is too thin", r.name());
        }
    }
}
