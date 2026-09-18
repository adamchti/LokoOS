//! LKO path parsing and validation.
//!
//! An [`LkoPath`] is a path that has already been proved well-formed. Code that
//! holds one does not need to re-check for `..`, embedded NULs, or a bogus
//! root, because a value of this type cannot be constructed without those
//! checks having run.
//!
//! ## Rejection over normalisation
//!
//! LKOFS **rejects** `.` and `..` rather than resolving them. Normalisation is
//! where directory-traversal bugs live: two components of a system disagree
//! about the order of "resolve symlinks" and "collapse `..`", and a path that
//! passed a check ends up pointing somewhere else. If there is exactly one
//! spelling of every location, there is nothing to disagree about.
//!
//! Callers that genuinely need to walk upward use [`LkoPath::parent`], which
//! cannot escape the root.

use crate::root::Root;
use core::fmt;
use loko_abi::{MAX_COMPONENT_LEN, MAX_PATH_LEN};

#[cfg(feature = "alloc")]
use alloc::string::String;

/// The prefix that names the LKOFS volume root.
pub const LKO_PREFIX: &str = "LKO";

/// Why a path was rejected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PathError {
    /// The path did not start with `LKO/` or `/`.
    MissingRoot,
    /// The first component was not one of the nineteen LKO roots.
    UnknownRoot,
    /// The path was longer than [`MAX_PATH_LEN`] bytes.
    TooLong,
    /// A single component was longer than [`MAX_COMPONENT_LEN`] bytes.
    ComponentTooLong,
    /// Two separators in a row, or a trailing separator followed by nothing.
    EmptyComponent,
    /// A `.` or `..` component. LKOFS does not resolve these; see the module
    /// documentation.
    RelativeComponent,
    /// A backslash appeared. LKO paths use `/` only; accepting `\` as well
    /// would create two spellings of the same location.
    BackslashSeparator,
    /// A `/` appeared inside what was supposed to be a single name. Rejected
    /// rather than silently treated as two components, so that
    /// [`LkoPath::join`] can never add more levels than the caller intended.
    SeparatorInName,
    /// A NUL, C0/C1 control character, or line separator appeared.
    ControlCharacter,
    /// A Unicode bidirectional override appeared. These can make a filename
    /// render as something other than what it is (the "Trojan Source" class),
    /// so LKOFS refuses to store them.
    BidirectionalOverride,
    /// A component ended in a space or a dot. Harmless in LKOFS itself, but
    /// such names round-trip badly through the Windows compatibility layer, so
    /// they are rejected at creation time rather than becoming unreachable
    /// later.
    TrailingSpaceOrDot,
    /// The root component contained non-ASCII text, which is never legitimate
    /// and is how homoglyph attacks on protected roots would begin.
    NonAsciiRoot,
}

impl PathError {
    /// A plain-language explanation, in the style required by requirement 64.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            PathError::MissingRoot => "That location doesn't start from the top of the drive.",
            PathError::UnknownRoot => "There's no such place on this system.",
            PathError::TooLong => "That location's name is too long.",
            PathError::ComponentTooLong => "One of the folder or file names is too long.",
            PathError::EmptyComponent => "That location has a blank folder name in it.",
            PathError::RelativeComponent => {
                "LokoOS doesn't allow \".\" or \"..\" in a location. Use the full location instead."
            }
            PathError::BackslashSeparator => "LokoOS separates folders with \"/\", not \"\\\".",
            PathError::SeparatorInName => "A name can't contain \"/\".",
            PathError::ControlCharacter => "That name contains characters LokoOS can't store.",
            PathError::BidirectionalOverride => {
                "That name contains hidden characters that would make it display as something else, so LokoOS won't store it."
            }
            PathError::TrailingSpaceOrDot => {
                "Names can't end with a space or a dot, because Windows apps wouldn't be able to open them."
            }
            PathError::NonAsciiRoot => "The top-level folder name isn't one LokoOS recognises.",
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.explanation())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for PathError {}

impl From<PathError> for loko_abi::Error {
    fn from(value: PathError) -> Self {
        match value {
            PathError::UnknownRoot => loko_abi::Error::NotFound,
            _ => loko_abi::Error::InvalidArgument,
        }
    }
}

/// Checks a single path component in isolation.
///
/// Exposed because the shell and Linder validate a name as the user types it,
/// before there is a path to validate.
pub fn validate_component(component: &str) -> Result<(), PathError> {
    if component.is_empty() {
        return Err(PathError::EmptyComponent);
    }
    if component.len() > MAX_COMPONENT_LEN {
        return Err(PathError::ComponentTooLong);
    }
    if component == "." || component == ".." {
        return Err(PathError::RelativeComponent);
    }
    if component.contains('\\') {
        return Err(PathError::BackslashSeparator);
    }
    if component.contains('/') {
        return Err(PathError::SeparatorInName);
    }
    for ch in component.chars() {
        // C0 controls (including NUL), DEL, and C1 controls.
        if ch.is_control() {
            return Err(PathError::ControlCharacter);
        }
        match ch {
            // Line and paragraph separators render as a newline in many
            // toolkits and would let a filename fake a second UI line.
            '\u{2028}' | '\u{2029}' => return Err(PathError::ControlCharacter),
            // LRE, RLE, PDF, LRO, RLO and the isolate family. These reorder
            // rendering without changing bytes.
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200F}' | '\u{200E}' => {
                return Err(PathError::BidirectionalOverride)
            }
            _ => {}
        }
    }
    // `ends_with(char)` on the last char, not the last byte, so multi-byte
    // characters are handled correctly.
    if let Some(last) = component.chars().next_back() {
        if last == ' ' || last == '.' {
            return Err(PathError::TrailingSpaceOrDot);
        }
    }
    Ok(())
}

/// A validated, borrowed LKO path.
///
/// Holds a reference to the caller's string. The string is guaranteed to be in
/// canonical form: it begins with `LKO/`, uses `/` separators, has no empty,
/// relative, or otherwise illegal components, and names a known [`Root`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LkoPathRef<'a> {
    /// Always begins with `LKO/`. Never has a trailing separator except for the
    /// volume root itself, which is stored as `LKO`.
    raw: &'a str,
    root: Option<Root>,
}

impl<'a> LkoPathRef<'a> {
    /// The volume root, `LKO`.
    pub const ROOT: LkoPathRef<'static> = LkoPathRef {
        raw: LKO_PREFIX,
        root: None,
    };

    /// Parses and validates a path.
    ///
    /// Accepts `LKO/System/Kernel` and `/System/Kernel`; both canonicalise to
    /// the former. A trailing `/` is accepted and dropped.
    pub fn parse(input: &'a str) -> Result<Self, PathError> {
        if input.len() > MAX_PATH_LEN {
            return Err(PathError::TooLong);
        }
        if input.contains('\\') {
            return Err(PathError::BackslashSeparator);
        }

        // Strip the volume prefix in either accepted spelling.
        let body = if let Some(rest) = strip_prefix_ci(input, LKO_PREFIX) {
            match rest.as_bytes().first() {
                None => return Ok(LkoPathRef::ROOT),
                Some(b'/') => &rest[1..],
                // "LKOsomething" is not the LKO root.
                Some(_) => return Err(PathError::MissingRoot),
            }
        } else if let Some(rest) = input.strip_prefix('/') {
            rest
        } else {
            return Err(PathError::MissingRoot);
        };

        // A trailing separator is cosmetic; anything after it must be empty.
        let body = body.strip_suffix('/').unwrap_or(body);
        if body.is_empty() {
            return Ok(LkoPathRef::ROOT);
        }

        let mut parts = body.split('/');
        let root_name = parts.next().expect("split always yields one item");
        validate_component(root_name)?;
        if !root_name.is_ascii() {
            return Err(PathError::NonAsciiRoot);
        }
        let root = Root::parse(root_name).ok_or(PathError::UnknownRoot)?;

        for part in parts {
            validate_component(part)?;
        }

        Ok(LkoPathRef {
            raw: input,
            root: Some(root),
        })
    }

    /// The root this path is under, or `None` for the volume root itself.
    pub const fn root(&self) -> Option<Root> {
        self.root
    }

    /// The components after the root, in order.
    ///
    /// `LKO/Users/Adam/Documents` yields `["Adam", "Documents"]`.
    pub fn tail(&self) -> impl DoubleEndedIterator<Item = &'a str> + Clone {
        let body = canonical_body(self.raw);
        let mut parts = body.split('/');
        // Drop the root component; for the volume root, `body` is empty and
        // `split` yields one empty string, which the filter removes.
        let _ = parts.next();
        parts.filter(|p| !p.is_empty())
    }

    /// All components including the root name.
    pub fn components(&self) -> impl DoubleEndedIterator<Item = &'a str> + Clone {
        canonical_body(self.raw)
            .split('/')
            .filter(|p| !p.is_empty())
    }

    /// How many components deep this path is. The volume root is depth 0.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.components().count()
    }

    /// The final component, or `None` for the volume root.
    pub fn file_name(&self) -> Option<&'a str> {
        self.components().next_back()
    }

    /// Whether `self` is `other` or is contained within it.
    ///
    /// Compared component by component, never as a string prefix.
    /// `LKO/Users/Adam2` is **not** within `LKO/Users/Adam`, and a string
    /// prefix test would get that wrong — which is exactly how sandbox escapes
    /// happen.
    ///
    /// Comparison is ASCII-case-insensitive, matching LKOFS's case-insensitive
    /// lookup, so a containment check cannot be evaded by changing case.
    #[must_use]
    pub fn is_within(&self, other: &LkoPathRef<'_>) -> bool {
        let mut mine = self.components();
        for theirs in other.components() {
            match mine.next() {
                Some(m) if m.eq_ignore_ascii_case(theirs) => {}
                _ => return false,
            }
        }
        true
    }

    /// The containing directory, or `None` for the volume root.
    ///
    /// Cannot escape `LKO`: the parent of `LKO/System` is `LKO`, and the parent
    /// of `LKO` is `None`.
    pub fn parent(&self) -> Option<LkoPathRef<'a>> {
        let body = canonical_body(self.raw);
        if body.is_empty() {
            return None;
        }
        match body.rfind('/') {
            // `LKO/System/Kernel` -> `LKO/System`
            Some(idx) => {
                let end = LKO_PREFIX.len() + 1 + idx;
                let raw = &self.raw[..raw_offset(self.raw, end)];
                Some(LkoPathRef {
                    raw,
                    root: self.root,
                })
            }
            // `LKO/System` -> `LKO`
            None => Some(LkoPathRef::ROOT),
        }
    }

    /// The path as a canonical string, always beginning with `LKO`.
    #[must_use]
    pub fn as_str(&self) -> &'a str {
        self.raw
    }

    /// Allocates an owned copy in canonical spelling.
    #[cfg(feature = "alloc")]
    #[must_use]
    pub fn to_owned_path(&self) -> LkoPath {
        let body = canonical_body(self.raw);
        let mut s = String::with_capacity(LKO_PREFIX.len() + 1 + body.len());
        s.push_str(LKO_PREFIX);
        if !body.is_empty() {
            s.push('/');
            s.push_str(body);
        }
        LkoPath {
            inner: s,
            root: self.root,
        }
    }
}

impl fmt::Display for LkoPathRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(LKO_PREFIX)?;
        let body = canonical_body(self.raw);
        if !body.is_empty() {
            f.write_str("/")?;
            f.write_str(body)?;
        }
        Ok(())
    }
}

impl fmt::Debug for LkoPathRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LkoPath({self})")
    }
}

/// A validated, owned LKO path in canonical spelling.
#[cfg(feature = "alloc")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LkoPath {
    inner: String,
    root: Option<Root>,
}

#[cfg(feature = "alloc")]
impl LkoPath {
    /// Parses and validates, producing a canonically spelled owned path.
    pub fn parse(input: &str) -> Result<Self, PathError> {
        Ok(LkoPathRef::parse(input)?.to_owned_path())
    }

    /// Borrows this path.
    #[must_use]
    pub fn as_ref(&self) -> LkoPathRef<'_> {
        LkoPathRef {
            raw: &self.inner,
            root: self.root,
        }
    }

    /// The root this path is under, or `None` for the volume root.
    pub const fn root(&self) -> Option<Root> {
        self.root
    }

    /// The canonical string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Appends a component, validating it first.
    ///
    /// Because `validate_component` rejects `..` and `/`, there is no input to
    /// this function that can make the result leave the current subtree.
    pub fn join(&self, component: &str) -> Result<LkoPath, PathError> {
        validate_component(component)?;
        let new_len = self.inner.len() + 1 + component.len();
        if new_len > MAX_PATH_LEN {
            return Err(PathError::TooLong);
        }
        let mut inner = String::with_capacity(new_len);
        inner.push_str(&self.inner);
        inner.push('/');
        inner.push_str(component);

        // Joining onto the volume root establishes the root component.
        let root = match self.root {
            Some(r) => Some(r),
            None => Some(Root::parse(component).ok_or(PathError::UnknownRoot)?),
        };
        Ok(LkoPath { inner, root })
    }
}

#[cfg(feature = "alloc")]
impl fmt::Display for LkoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

#[cfg(feature = "alloc")]
impl fmt::Debug for LkoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LkoPath({})", self.inner)
    }
}

#[cfg(feature = "alloc")]
impl core::str::FromStr for LkoPath {
    type Err = PathError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        LkoPath::parse(s)
    }
}

#[cfg(feature = "alloc")]
impl From<LkoPathRef<'_>> for LkoPath {
    fn from(value: LkoPathRef<'_>) -> Self {
        value.to_owned_path()
    }
}

#[cfg(feature = "alloc")]
impl LkoPath {
    /// The canonical string, consuming the path.
    #[must_use]
    pub fn into_string(self) -> String {
        self.inner
    }
}

// -- helpers ---------------------------------------------------------------

/// Strips `prefix` from the start of `s`, ignoring ASCII case.
///
/// `prefix` is always ASCII. The char-boundary check is not optional: slicing
/// `"é.."` at byte 3 would split a multi-byte character and panic, and this
/// function runs on unvalidated, attacker-reachable input.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    debug_assert!(prefix.is_ascii(), "strip_prefix_ci assumes an ASCII prefix");
    if s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// The part of a raw path after the volume prefix, with no leading or trailing
/// separator. Returns `""` for the volume root.
///
/// Only ever called on strings that [`LkoPathRef::parse`] has already accepted.
fn canonical_body(raw: &str) -> &str {
    let rest = if let Some(r) = strip_prefix_ci(raw, LKO_PREFIX) {
        r
    } else {
        raw
    };
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    rest.strip_suffix('/').unwrap_or(rest)
}

/// Maps an offset in canonical space back to an offset in the raw string.
///
/// The two differ only when the caller wrote the `/System/...` spelling, which
/// is one byte shorter than `LKO/System/...`.
fn raw_offset(raw: &str, canonical_offset: usize) -> usize {
    if strip_prefix_ci(raw, LKO_PREFIX).is_some() {
        canonical_offset
    } else {
        // The raw form omits "LKO", so shift left by its length.
        canonical_offset - LKO_PREFIX.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> LkoPathRef<'_> {
        LkoPathRef::parse(s).expect("should parse")
    }

    #[test]
    fn parses_canonical_form() {
        let path = p("LKO/System/Kernel");
        assert_eq!(path.root(), Some(Root::System));
        assert_eq!(path.file_name(), Some("Kernel"));
        assert_eq!(path.depth(), 2);
    }

    #[test]
    fn accepts_the_leading_slash_spelling() {
        let path = p("/Users/Adam/Documents");
        assert_eq!(path.root(), Some(Root::Users));
        assert_eq!(path.to_string(), "LKO/Users/Adam/Documents");
    }

    #[test]
    fn volume_root_parses_in_every_spelling() {
        for spelling in ["LKO", "LKO/", "/"] {
            let path = p(spelling);
            assert_eq!(path.root(), None, "{spelling} should be the volume root");
            assert_eq!(path.depth(), 0);
            assert_eq!(path.to_string(), "LKO");
        }
    }

    #[test]
    fn trailing_separator_is_dropped() {
        assert_eq!(p("LKO/Users/Adam/").to_string(), "LKO/Users/Adam");
    }

    #[test]
    fn rejects_traversal() {
        for bad in [
            "LKO/Users/../System",
            "LKO/Users/Adam/..",
            "LKO/./System",
            "/../Boot",
        ] {
            assert_eq!(
                LkoPathRef::parse(bad).unwrap_err(),
                PathError::RelativeComponent,
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_backslashes() {
        for bad in ["LKO\\System", "LKO/Users\\Adam", "\\System"] {
            assert_eq!(
                LkoPathRef::parse(bad).unwrap_err(),
                PathError::BackslashSeparator,
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_embedded_nul_and_controls() {
        assert_eq!(
            LkoPathRef::parse("LKO/Users/ad\u{0}am").unwrap_err(),
            PathError::ControlCharacter
        );
        assert_eq!(
            LkoPathRef::parse("LKO/Users/a\nb").unwrap_err(),
            PathError::ControlCharacter
        );
        assert_eq!(
            LkoPathRef::parse("LKO/Users/a\u{2028}b").unwrap_err(),
            PathError::ControlCharacter
        );
    }

    #[test]
    fn rejects_bidi_overrides() {
        // "invoice\u{202E}fdp.exe" renders as "invoiceexe.pdf".
        assert_eq!(
            LkoPathRef::parse("LKO/Users/Adam/invoice\u{202E}fdp.exe").unwrap_err(),
            PathError::BidirectionalOverride
        );
        assert_eq!(
            LkoPathRef::parse("LKO/Users/Adam/a\u{2066}b").unwrap_err(),
            PathError::BidirectionalOverride
        );
    }

    #[test]
    fn rejects_empty_components() {
        assert_eq!(
            LkoPathRef::parse("LKO/Users//Adam").unwrap_err(),
            PathError::EmptyComponent
        );
    }

    #[test]
    fn rejects_unknown_roots() {
        assert_eq!(
            LkoPathRef::parse("LKO/Windows/System32").unwrap_err(),
            PathError::UnknownRoot
        );
    }

    #[test]
    fn rejects_missing_root() {
        for bad in ["System/Kernel", "LKOSystem", "C:/Windows"] {
            assert!(
                matches!(
                    LkoPathRef::parse(bad).unwrap_err(),
                    PathError::MissingRoot | PathError::BackslashSeparator
                ),
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_trailing_space_or_dot() {
        assert_eq!(
            LkoPathRef::parse("LKO/Users/Adam/report ").unwrap_err(),
            PathError::TrailingSpaceOrDot
        );
        assert_eq!(
            LkoPathRef::parse("LKO/Users/Adam/report.").unwrap_err(),
            PathError::TrailingSpaceOrDot
        );
    }

    #[test]
    fn rejects_oversized_input() {
        let long_component = "a".repeat(MAX_COMPONENT_LEN + 1);
        assert_eq!(
            LkoPathRef::parse(&alloc::format!("LKO/Users/{long_component}")).unwrap_err(),
            PathError::ComponentTooLong
        );
        let long_path = alloc::format!("LKO/Users/{}", "a/".repeat(MAX_PATH_LEN));
        assert_eq!(
            LkoPathRef::parse(&long_path).unwrap_err(),
            PathError::TooLong
        );
    }

    #[test]
    fn accepts_unicode_in_user_filenames() {
        // Requirement 51 and basic decency: names in any script must work.
        for good in [
            "LKO/Users/Adam/Bericht über Größen.txt",
            "LKO/Users/Adam/日本語のファイル.md",
            "LKO/Users/Adam/файл.txt",
            "LKO/Users/Adam/emoji 🎉 party.png",
        ] {
            assert!(LkoPathRef::parse(good).is_ok(), "{good} should be accepted");
        }
    }

    #[test]
    fn containment_is_component_wise_not_prefix_wise() {
        let adam = p("LKO/Users/Adam");
        // The bug this test exists to prevent.
        assert!(!p("LKO/Users/Adam2").is_within(&adam));
        assert!(!p("LKO/Users/Adamant/secret").is_within(&adam));
        // The cases that must still work.
        assert!(p("LKO/Users/Adam").is_within(&adam));
        assert!(p("LKO/Users/Adam/Documents/a.txt").is_within(&adam));
        assert!(!p("LKO/Users/Bea").is_within(&adam));
        assert!(!p("LKO/System").is_within(&adam));
    }

    #[test]
    fn containment_cannot_be_evaded_by_case() {
        let system = p("LKO/System");
        assert!(p("LKO/system/Kernel").is_within(&system));
        assert!(p("LKO/SYSTEM/Kernel").is_within(&system));
    }

    #[test]
    fn everything_is_within_the_volume_root() {
        let root = LkoPathRef::ROOT;
        for path in ["LKO/System", "LKO/Users/Adam", "LKO"] {
            assert!(p(path).is_within(&root));
        }
    }

    #[test]
    fn parent_walks_up_and_stops_at_the_volume_root() {
        let path = p("LKO/Users/Adam/Documents");
        let a = path.parent().unwrap();
        assert_eq!(a.to_string(), "LKO/Users/Adam");
        let b = a.parent().unwrap();
        assert_eq!(b.to_string(), "LKO/Users");
        let c = b.parent().unwrap();
        assert_eq!(c.to_string(), "LKO");
        assert_eq!(c.parent(), None, "the volume root has no parent");
    }

    #[test]
    fn parent_works_for_the_short_spelling_too() {
        let path = p("/Users/Adam/Documents");
        assert_eq!(path.parent().unwrap().to_string(), "LKO/Users/Adam");
        assert_eq!(
            path.parent().unwrap().parent().unwrap().to_string(),
            "LKO/Users"
        );
    }

    #[test]
    fn tail_skips_the_root_component() {
        let path = p("LKO/Users/Adam/Documents");
        let tail: alloc::vec::Vec<_> = path.tail().collect();
        assert_eq!(tail, ["Adam", "Documents"]);
        assert_eq!(p("LKO/System").tail().count(), 0);
        assert_eq!(LkoPathRef::ROOT.tail().count(), 0);
    }

    #[test]
    fn owned_paths_canonicalise() {
        let owned = LkoPath::parse("/Users/Adam/").unwrap();
        assert_eq!(owned.as_str(), "LKO/Users/Adam");
        assert_eq!(owned.root(), Some(Root::Users));
    }

    #[test]
    fn join_validates_and_cannot_escape() {
        let base = LkoPath::parse("LKO/Users/Adam").unwrap();
        assert_eq!(
            base.join("Documents").unwrap().as_str(),
            "LKO/Users/Adam/Documents"
        );
        assert_eq!(base.join("..").unwrap_err(), PathError::RelativeComponent);
        assert_eq!(base.join("").unwrap_err(), PathError::EmptyComponent);
        assert_eq!(
            base.join("a\\b").unwrap_err(),
            PathError::BackslashSeparator
        );
        // A separator inside a "component" must be rejected, not silently
        // treated as two levels. Otherwise `join` is a way to synthesise a
        // deeper path than the caller asked for, and a caller that checked only
        // the component it passed in would be checking the wrong thing.
        assert_eq!(base.join("a/b").unwrap_err(), PathError::SeparatorInName);
        assert_eq!(
            base.join("../../System").unwrap_err(),
            PathError::SeparatorInName
        );
    }

    #[test]
    fn join_establishes_the_root_from_the_volume_root() {
        let root = LkoPath::parse("LKO").unwrap();
        assert_eq!(root.root(), None);
        let system = root.join("System").unwrap();
        assert_eq!(system.root(), Some(Root::System));
        assert_eq!(root.join("Nonsense").unwrap_err(), PathError::UnknownRoot);
    }

    #[test]
    fn display_is_always_canonical() {
        for (input, expected) in [
            ("/System", "LKO/System"),
            ("LKO/System/", "LKO/System"),
            ("lko/System", "LKO/System"),
            ("LKO", "LKO"),
            ("/", "LKO"),
        ] {
            assert_eq!(p(input).to_string(), expected, "input {input}");
        }
    }

    #[test]
    fn path_errors_explain_themselves_in_plain_language() {
        for e in [
            PathError::MissingRoot,
            PathError::UnknownRoot,
            PathError::TooLong,
            PathError::ComponentTooLong,
            PathError::EmptyComponent,
            PathError::RelativeComponent,
            PathError::BackslashSeparator,
            PathError::SeparatorInName,
            PathError::ControlCharacter,
            PathError::BidirectionalOverride,
            PathError::TrailingSpaceOrDot,
            PathError::NonAsciiRoot,
        ] {
            let text = e.explanation();
            assert!(text.ends_with('.'), "{e:?} explanation is not a sentence");
            assert!(!text.contains("0x"), "{e:?} leaks a code");
        }
    }
}
