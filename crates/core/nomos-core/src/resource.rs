//! Resource identity: a validated absolute path, a content digest, the
//! validated names of the other families, and the [`ResourceKey`] that
//! names one resource of any family ([resource-families.md]).
//!
//! Every type here has private fields and a fallible constructor. A
//! `ResourcePath` that exists is absolute, has no empty, `.`, or `..`
//! component, and has no trailing separator except for the root itself. A
//! `Digest` is exactly 32 bytes. A unit, sysctl, account, or package name
//! that exists is valid for its family. Nothing else can be built, so
//! nothing downstream checks again (ADR 0004 §4).
//!
//! [resource-families.md]: ../../../../docs/formal/resource-families.md

use alloc::string::String;
use core::fmt;

/// Why a string is not a resource path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// The path is empty.
    Empty,
    /// The path does not start with `/`.
    Relative,
    /// A component is empty (`//`), `.`, or `..`.
    UnnormalizedComponent,
    /// A path other than `/` ends with `/`.
    TrailingSeparator,
    /// The path contains a NUL byte, which no operating system path may.
    Nul,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathError::Empty => "path is empty",
            PathError::Relative => "path is not absolute",
            PathError::UnnormalizedComponent => "path has an empty, `.`, or `..` component",
            PathError::TrailingSeparator => "path ends with a separator",
            PathError::Nul => "path contains a NUL byte",
        })
    }
}

/// An absolute, normalized path naming one file resource on a host.
///
/// Equality is textual. Two paths that name the same inode through a symlink
/// are different resources here; resolution happens at the Substrate
/// boundary, at the moment of use (ADR 0013 §2).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourcePath(String);

impl ResourcePath {
    /// Validates `text` as an absolute, normalized path.
    pub fn new(text: &str) -> Result<Self, PathError> {
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        if text.contains('\0') {
            return Err(PathError::Nul);
        }
        let Some(rest) = text.strip_prefix('/') else {
            return Err(PathError::Relative);
        };
        if rest.is_empty() {
            return Ok(ResourcePath(String::from("/")));
        }
        if rest.ends_with('/') {
            return Err(PathError::TrailingSeparator);
        }
        if rest
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
        {
            return Err(PathError::UnnormalizedComponent);
        }
        Ok(ResourcePath(String::from(text)))
    }

    /// The path as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// An unvalidated, empty path for the bounded-verifier harnesses: it
    /// allocates nothing, which keeps the harnesses over enums that carry a
    /// path tractable. It exists only under `cfg(kani)`.
    #[cfg(kani)]
    pub(crate) fn for_harness() -> Self {
        ResourcePath(String::new())
    }
}

impl fmt::Debug for ResourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResourcePath({:?})", self.0)
    }
}

impl fmt::Display for ResourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a value is not a digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestError {
    /// The hex text is not exactly 64 characters.
    Length,
    /// A character is not a hex digit.
    NotHex,
}

impl fmt::Display for DigestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DigestError::Length => "digest is not 64 hex characters",
            DigestError::NotHex => "digest has a non-hex character",
        })
    }
}

/// A 256-bit content digest. Which hash function produced it is fixed by the
/// Canon that names it; this type only carries the bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct Digest([u8; 32]);

impl Digest {
    /// A digest from its bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Digest(bytes)
    }

    /// A digest from 64 lowercase or uppercase hex characters.
    pub fn from_hex(text: &str) -> Result<Self, DigestError> {
        if text.len() != 64 {
            return Err(DigestError::Length);
        }
        let mut bytes = [0u8; 32];
        for (i, pair) in text.as_bytes().chunks(2).enumerate() {
            let hi = hex_value(pair[0]).ok_or(DigestError::NotHex)?;
            let lo = hex_value(pair[1]).ok_or(DigestError::NotHex)?;
            if let Some(slot) = bytes.get_mut(i) {
                *slot = (hi << 4) | lo;
            }
        }
        Ok(Digest(bytes))
    }

    /// The digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Digest(")?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(")")
    }
}

/// Why text is not a valid name for its family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// The name is empty.
    Empty,
    /// The name is longer than its family allows.
    TooLong,
    /// The name has a character its family does not allow.
    Character,
    /// The characters are allowed but the shape is not: a unit without a
    /// known suffix, a sysctl with one segment, a package name too short.
    Shape,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NameError::Empty => "name is empty",
            NameError::TooLong => "name is too long",
            NameError::Character => "name has a character its family does not allow",
            NameError::Shape => "name does not have its family's shape",
        })
    }
}

fn check_length(text: &str, max: usize) -> Result<(), NameError> {
    if text.is_empty() {
        return Err(NameError::Empty);
    }
    if text.len() > max {
        return Err(NameError::TooLong);
    }
    Ok(())
}

/// The suffixes a managed unit may have.
pub const UNIT_SUFFIXES: [&str; 6] = [
    ".service", ".socket", ".timer", ".target", ".path", ".mount",
];

/// A systemd unit name: `[A-Za-z0-9:_.\\@-]`, a known suffix, a stem.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitName(String);

impl UnitName {
    /// Validates `text` as a unit name.
    pub fn new(text: &str) -> Result<Self, NameError> {
        check_length(text, 255)?;
        if !text.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'.' | b'\\' | b'@' | b'-')
        }) {
            return Err(NameError::Character);
        }
        let known = UNIT_SUFFIXES
            .iter()
            .any(|suffix| text.len() > suffix.len() && text.ends_with(suffix));
        if !known {
            return Err(NameError::Shape);
        }
        Ok(UnitName(String::from(text)))
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A kernel parameter name: dot-separated segments of `[a-z0-9_-]`, at
/// least two, so it cannot name a path outside `/proc/sys`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SysctlKey(String);

impl SysctlKey {
    /// Validates `text` as a sysctl name.
    pub fn new(text: &str) -> Result<Self, NameError> {
        check_length(text, 255)?;
        if !text.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        }) {
            return Err(NameError::Character);
        }
        let mut segments = 0usize;
        for segment in text.split('.') {
            if segment.is_empty() {
                return Err(NameError::Shape);
            }
            segments += 1;
        }
        if segments < 2 {
            return Err(NameError::Shape);
        }
        Ok(SysctlKey(String::from(text)))
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An account name, of a user or a group: Debian's default,
/// `[a-z_][a-z0-9_-]*`, optionally ending in `$`, at most 32 bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountName(String);

impl AccountName {
    /// Validates `text` as an account name.
    pub fn new(text: &str) -> Result<Self, NameError> {
        check_length(text, 32)?;
        let body = text.strip_suffix('$').unwrap_or(text);
        let mut bytes = body.bytes();
        match bytes.next() {
            Some(b) if b.is_ascii_lowercase() || b == b'_' => {}
            Some(_) => return Err(NameError::Character),
            None => return Err(NameError::Shape),
        }
        if !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
        {
            return Err(NameError::Character);
        }
        Ok(AccountName(String::from(text)))
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Debian package name: `[a-z0-9][a-z0-9+.-]+`, 2 to 128 bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageName(String);

impl PackageName {
    /// Validates `text` as a package name.
    pub fn new(text: &str) -> Result<Self, NameError> {
        check_length(text, 128)?;
        let mut bytes = text.bytes();
        match bytes.next() {
            Some(b) if b.is_ascii_lowercase() || b.is_ascii_digit() => {}
            _ => return Err(NameError::Character),
        }
        if !bytes.all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'+' | b'.' | b'-')
        }) {
            return Err(NameError::Character);
        }
        if text.len() < 2 {
            return Err(NameError::Shape);
        }
        Ok(PackageName(String::from(text)))
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! name_formatting {
    ($($ty:ident),*) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($ty), "({:?})"), self.0)
            }
        }
        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    )*};
}
name_formatting!(UnitName, SysctlKey, AccountName, PackageName);

/// A resource family ([resource-families.md]). The order is the order of
/// the family's name as text, so that ordering keys by family agrees with
/// sorting their text.
///
/// [resource-families.md]: ../../../../docs/formal/resource-families.md
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Family {
    /// Directories.
    Directory,
    /// Regular files.
    File,
    /// Debian packages.
    Package,
    /// The abstract service of the transition kernel, served by the mock.
    Service,
    /// Kernel parameters.
    Sysctl,
    /// systemd units.
    Unit,
    /// Accounts.
    User,
}

impl Family {
    /// Every family, in order.
    pub const ALL: [Family; 7] = [
        Family::Directory,
        Family::File,
        Family::Package,
        Family::Service,
        Family::Sysctl,
        Family::Unit,
        Family::User,
    ];

    /// The family's name, as the IR and a key's text form write it.
    pub fn as_str(&self) -> &'static str {
        match self {
            Family::Directory => "directory",
            Family::File => "file",
            Family::Package => "package",
            Family::Service => "service",
            Family::Sysctl => "sysctl",
            Family::Unit => "unit",
            Family::User => "user",
        }
    }

    /// The family with this name.
    pub fn from_name(text: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.as_str() == text)
    }

    /// Whether resources of the family are named by a path.
    pub fn is_path(&self) -> bool {
        matches!(self, Family::Directory | Family::File | Family::Service)
    }
}

/// Why text is not a resource key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// No `:` separates a family from a name.
    NoFamily,
    /// The family is not one of the seven.
    UnknownFamily,
    /// The path is not valid.
    Path(PathError),
    /// The name is not valid for the family.
    Name(NameError),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::NoFamily => f.write_str("key has no `<family>:` prefix"),
            KeyError::UnknownFamily => f.write_str("key names an unknown family"),
            KeyError::Path(e) => write!(f, "key's {e}"),
            KeyError::Name(e) => write!(f, "key's {e}"),
        }
    }
}

/// One resource of any family: its family and its name. Keys order by
/// name, then family: a path begins with `/` and sorts before every other
/// name, so the resources named by paths keep the order of their paths,
/// the order of schema 2 and of the transition kernel's scenarios
/// (resource-families.md).
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ResourceKey {
    /// A directory at a path.
    Directory(ResourcePath),
    /// A regular file at a path.
    File(ResourcePath),
    /// A package by name.
    Package(PackageName),
    /// The abstract service at a path.
    Service(ResourcePath),
    /// A kernel parameter.
    Sysctl(SysctlKey),
    /// A systemd unit.
    Unit(UnitName),
    /// An account.
    User(AccountName),
}

impl Ord for ResourceKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.name()
            .cmp(other.name())
            .then_with(|| self.family().cmp(&other.family()))
    }
}

impl PartialOrd for ResourceKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl ResourceKey {
    /// The key's family.
    pub fn family(&self) -> Family {
        match self {
            ResourceKey::Directory(_) => Family::Directory,
            ResourceKey::File(_) => Family::File,
            ResourceKey::Package(_) => Family::Package,
            ResourceKey::Service(_) => Family::Service,
            ResourceKey::Sysctl(_) => Family::Sysctl,
            ResourceKey::Unit(_) => Family::Unit,
            ResourceKey::User(_) => Family::User,
        }
    }

    /// The name within the family, as text.
    pub fn name(&self) -> &str {
        match self {
            ResourceKey::Directory(p) | ResourceKey::File(p) | ResourceKey::Service(p) => {
                p.as_str()
            }
            ResourceKey::Package(n) => n.as_str(),
            ResourceKey::Sysctl(n) => n.as_str(),
            ResourceKey::Unit(n) => n.as_str(),
            ResourceKey::User(n) => n.as_str(),
        }
    }

    /// The path, for a family named by one.
    pub fn path(&self) -> Option<&ResourcePath> {
        match self {
            ResourceKey::Directory(p) | ResourceKey::File(p) | ResourceKey::Service(p) => Some(p),
            _ => None,
        }
    }

    /// A key of `family` named `name`.
    pub fn from_parts(family: Family, name: &str) -> Result<Self, KeyError> {
        let path = || ResourcePath::new(name).map_err(KeyError::Path);
        Ok(match family {
            Family::Directory => ResourceKey::Directory(path()?),
            Family::File => ResourceKey::File(path()?),
            Family::Service => ResourceKey::Service(path()?),
            Family::Package => {
                ResourceKey::Package(PackageName::new(name).map_err(KeyError::Name)?)
            }
            Family::Sysctl => ResourceKey::Sysctl(SysctlKey::new(name).map_err(KeyError::Name)?),
            Family::Unit => ResourceKey::Unit(UnitName::new(name).map_err(KeyError::Name)?),
            Family::User => ResourceKey::User(AccountName::new(name).map_err(KeyError::Name)?),
        })
    }

    /// Parses the text form, `<family>:<name>`.
    pub fn parse(text: &str) -> Result<Self, KeyError> {
        let (family, name) = text.split_once(':').ok_or(KeyError::NoFamily)?;
        let family = Family::from_name(family).ok_or(KeyError::UnknownFamily)?;
        ResourceKey::from_parts(family, name)
    }
}

impl fmt::Display for ResourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.family().as_str(), self.name())
    }
}

impl fmt::Debug for ResourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResourceKey({self})")
    }
}

/// Permission bits: at most `0o7777`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mode(u16);

impl Mode {
    /// The mode with these bits, when they fit in `0o7777`.
    pub fn new(bits: u16) -> Option<Self> {
        (bits <= 0o7777).then_some(Mode(bits))
    }

    /// `0644`, a file's mode under the default umask.
    pub const DEFAULT_FILE: Mode = Mode(0o644);

    /// `0755`, a directory's mode under the default umask.
    pub const DEFAULT_DIRECTORY: Mode = Mode(0o755);

    /// The mode from exactly four octal digits, as the IR writes it.
    pub fn from_octal(text: &str) -> Option<Self> {
        if text.len() != 4 || !text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
            return None;
        }
        text.bytes()
            .try_fold(0u16, |acc, b| {
                acc.checked_mul(8)?.checked_add(u16::from(b - b'0'))
            })
            .and_then(Mode::new)
    }

    /// The bits.
    pub fn bits(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04o}", self.0)
    }
}

impl fmt::Debug for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mode({:04o})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Digest, DigestError, PathError, ResourcePath};

    #[test]
    fn absolute_normalized_paths_are_accepted() {
        for text in ["/", "/etc", "/etc/hosts", "/a/b.c/d-e_f"] {
            assert_eq!(ResourcePath::new(text).unwrap().as_str(), text, "{text}");
        }
    }

    #[test]
    fn every_other_shape_is_rejected_with_its_reason() {
        let cases = [
            ("", PathError::Empty),
            ("etc/hosts", PathError::Relative),
            ("./etc", PathError::Relative),
            ("/etc/", PathError::TrailingSeparator),
            ("//etc", PathError::UnnormalizedComponent),
            ("/etc//hosts", PathError::UnnormalizedComponent),
            ("/etc/./hosts", PathError::UnnormalizedComponent),
            ("/etc/../hosts", PathError::UnnormalizedComponent),
            ("/..", PathError::UnnormalizedComponent),
            ("/etc/ho\0sts", PathError::Nul),
        ];
        for (text, error) in cases {
            assert_eq!(ResourcePath::new(text).unwrap_err(), error, "{text:?}");
        }
    }

    #[test]
    fn digests_round_trip_through_hex_in_either_case() {
        let lower = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let upper = lower.to_uppercase();
        let d = Digest::from_hex(lower).unwrap();
        assert_eq!(Digest::from_hex(&upper).unwrap(), d);
        assert_eq!(d.as_bytes()[0], 0x00);
        assert_eq!(d.as_bytes()[2], 0x22);
        assert_eq!(d.as_bytes()[31], 0xff);
        assert_eq!(alloc::format!("{d:?}"), alloc::format!("Digest({lower})"));
    }

    // Found by mutation calibration (the assessment-algebra record): the
    // Display and Debug forms could be emptied and no test noticed.
    #[test]
    fn display_forms_say_what_they_are() {
        let path = ResourcePath::new("/etc/hosts").unwrap();
        assert_eq!(alloc::format!("{path}"), "/etc/hosts");
        assert_eq!(alloc::format!("{path:?}"), "ResourcePath(\"/etc/hosts\")");
        for error in [
            PathError::Empty,
            PathError::Relative,
            PathError::UnnormalizedComponent,
            PathError::TrailingSeparator,
            PathError::Nul,
        ] {
            assert!(alloc::format!("{error}").contains("path"), "{error:?}");
        }
        for error in [DigestError::Length, DigestError::NotHex] {
            assert!(alloc::format!("{error}").contains("digest"), "{error:?}");
        }
    }

    #[test]
    fn malformed_digests_are_rejected_with_their_reason() {
        assert_eq!(Digest::from_hex("abc").unwrap_err(), DigestError::Length);
        assert_eq!(
            Digest::from_hex(&"g".repeat(64)).unwrap_err(),
            DigestError::NotHex
        );
    }
}
