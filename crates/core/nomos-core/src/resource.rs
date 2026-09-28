//! Resource identity for the file family: a validated absolute path and a
//! content digest.
//!
//! Both types have private fields and fallible constructors. A `ResourcePath`
//! that exists is absolute, has no empty, `.`, or `..` component, and has no
//! trailing separator except for the root itself. A `Digest` is exactly 32
//! bytes. Nothing else can be built, so nothing downstream checks again
//! (ADR 0004 §4).

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
