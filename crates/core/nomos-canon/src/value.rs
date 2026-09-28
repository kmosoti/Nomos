//! The restricted data model both candidate encodings carry
//! ([canon-ir.md](../../../../docs/formal/canon-ir.md#the-data-model)).
//!
//! Unsigned integers up to $2^{53} - 1$, UTF-8 text, arrays, and maps with
//! text keys, within fixed limits. Nothing else: no floating point, no
//! negative integers, no byte strings, no booleans, no null, no tags. A
//! decoder that meets anything outside the model rejects it.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// The largest integer the model carries: the largest both encodings
/// represent exactly.
pub const MAX_UINT: u64 = (1 << 53) - 1;
/// The most bytes in a text string.
pub const MAX_TEXT: usize = 4096;
/// The most items in an array.
pub const MAX_ARRAY: usize = 65_536;
/// The most entries in a map.
pub const MAX_MAP: usize = 64;
/// The deepest nesting of arrays and maps.
pub const MAX_DEPTH: usize = 8;
/// The most bytes in an artifact.
pub const MAX_ARTIFACT: usize = 16 * 1024 * 1024;

/// A value of the data model. A map keeps its entries in the order they
/// were given or read; the encoders put them in their profile's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// An unsigned integer, at most [`MAX_UINT`].
    Uint(u64),
    /// UTF-8 text.
    Text(String),
    /// An array.
    Array(Vec<Value>),
    /// A map with text keys, each at most once.
    Map(Vec<(String, Value)>),
}

impl Value {
    /// A text value.
    pub fn text(s: &str) -> Value {
        Value::Text(String::from(s))
    }

    /// The entry of a map under `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Why bytes are not a value of the data model under a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxError {
    /// The input ended inside a value.
    Truncated,
    /// Bytes follow the value.
    TrailingBytes,
    /// The bytes are not well formed under the profile.
    Malformed,
    /// A well-formed item the model excludes: a float, a negative number, a
    /// tag, a byte string, a boolean, null, or an indefinite length.
    OutsideModel,
    /// Text that is not valid UTF-8, or a lone surrogate.
    InvalidText,
    /// A map key appears twice.
    DuplicateKey,
    /// An integer, a length, the nesting, or the artifact exceeds its limit.
    LimitExceeded,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SyntaxError::Truncated => "the input ends inside a value",
            SyntaxError::TrailingBytes => "bytes follow the value",
            SyntaxError::Malformed => "the input is not well formed",
            SyntaxError::OutsideModel => "an item outside the data model",
            SyntaxError::InvalidText => "text that is not valid UTF-8",
            SyntaxError::DuplicateKey => "a map key appears twice",
            SyntaxError::LimitExceeded => "a limit of the data model is exceeded",
        })
    }
}

/// Checks the limits that a decoder cannot check while reading: used by the
/// decoders on every value they build.
pub(crate) fn check_map_keys(entries: &[(String, Value)]) -> Result<(), SyntaxError> {
    if entries.len() > MAX_MAP {
        return Err(SyntaxError::LimitExceeded);
    }
    for (i, (k, _)) in entries.iter().enumerate() {
        if entries[..i].iter().any(|(other, _)| other == k) {
            return Err(SyntaxError::DuplicateKey);
        }
    }
    Ok(())
}
