//! Deterministic Concise Binary Object Representation (CBOR): the core
//! deterministic encoding requirements of RFC 8949 §4.2.1 over the data
//! model of [`crate::value`].
//!
//! The encoder writes integers and lengths in their shortest form, definite
//! lengths only, and map entries sorted by the bytewise order of their
//! encoded keys. The decoder accepts well-formed CBOR within the data model
//! and nothing else; it does not itself insist on the shortest form, because
//! the canonical-form stage of [`crate::artifact`] re-encodes and compares,
//! which catches every non-canonical choice at once.

use alloc::string::String;
use alloc::vec::Vec;

use crate::value::{
    MAX_ARRAY, MAX_ARTIFACT, MAX_DEPTH, MAX_MAP, MAX_TEXT, MAX_UINT, SyntaxError, Value,
    check_map_keys,
};

const UINT: u8 = 0;
const TEXT: u8 = 3;
const ARRAY: u8 = 4;
const MAP: u8 = 5;

/// The deterministic encoding of `value`.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(value, &mut out);
    out
}

fn header(major: u8, n: u64, out: &mut Vec<u8>) {
    let m = major << 5;
    if n < 24 {
        out.push(m | n as u8);
    } else if n <= 0xff {
        out.push(m | 24);
        out.push(n as u8);
    } else if n <= 0xffff {
        out.push(m | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if n <= 0xffff_ffff {
        out.push(m | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

fn write(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Uint(n) => header(UINT, *n, out),
        Value::Text(s) => {
            header(TEXT, s.len() as u64, out);
            out.extend_from_slice(s.as_bytes());
        }
        Value::Array(items) => {
            header(ARRAY, items.len() as u64, out);
            for item in items {
                write(item, out);
            }
        }
        Value::Map(entries) => {
            header(MAP, entries.len() as u64, out);
            let mut encoded: Vec<(Vec<u8>, &Value)> = entries
                .iter()
                .map(|(k, v)| (encode(&Value::Text(k.clone())), v))
                .collect();
            encoded.sort_by(|a, b| a.0.cmp(&b.0));
            for (key, v) in encoded {
                out.extend_from_slice(&key);
                write(v, out);
            }
        }
    }
}

/// Decodes one value of the data model from `bytes`, which must hold
/// exactly that value.
pub fn decode(bytes: &[u8]) -> Result<Value, SyntaxError> {
    if bytes.len() > MAX_ARTIFACT {
        return Err(SyntaxError::LimitExceeded);
    }
    let mut reader = Reader { bytes, at: 0 };
    let value = reader.value(0)?;
    if reader.at != bytes.len() {
        return Err(SyntaxError::TrailingBytes);
    }
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, SyntaxError> {
        let b = *self.bytes.get(self.at).ok_or(SyntaxError::Truncated)?;
        self.at += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&[u8], SyntaxError> {
        let end = self.at.checked_add(n).ok_or(SyntaxError::Truncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(SyntaxError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    /// The major type and argument of the next item.
    fn head(&mut self) -> Result<(u8, u64), SyntaxError> {
        let initial = self.byte()?;
        let major = initial >> 5;
        let info = initial & 0x1f;
        let n = match info {
            0..=23 => u64::from(info),
            24 => u64::from(self.byte()?),
            25 => {
                let b = self.take(2)?;
                u64::from(u16::from_be_bytes([b[0], b[1]]))
            }
            26 => {
                let b = self.take(4)?;
                u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            }
            27 => {
                let b = self.take(8)?;
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
            // Indefinite lengths are outside the model; 28 to 30 are reserved.
            31 => return Err(SyntaxError::OutsideModel),
            _ => return Err(SyntaxError::Malformed),
        };
        Ok((major, n))
    }

    fn value(&mut self, depth: usize) -> Result<Value, SyntaxError> {
        let (major, n) = self.head()?;
        match major {
            UINT => {
                if n > MAX_UINT {
                    return Err(SyntaxError::LimitExceeded);
                }
                Ok(Value::Uint(n))
            }
            TEXT => Ok(Value::Text(self.text(n)?)),
            ARRAY => {
                if depth >= MAX_DEPTH {
                    return Err(SyntaxError::LimitExceeded);
                }
                if n > MAX_ARRAY as u64 {
                    return Err(SyntaxError::LimitExceeded);
                }
                let len = n as usize;
                if len > self.remaining() {
                    return Err(SyntaxError::Truncated);
                }
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            MAP => {
                if depth >= MAX_DEPTH {
                    return Err(SyntaxError::LimitExceeded);
                }
                if n > MAX_MAP as u64 {
                    return Err(SyntaxError::LimitExceeded);
                }
                let len = n as usize;
                let mut entries = Vec::with_capacity(len);
                for _ in 0..len {
                    let (key_major, key_len) = self.head()?;
                    if key_major != TEXT {
                        return Err(SyntaxError::OutsideModel);
                    }
                    let key = self.text(key_len)?;
                    let v = self.value(depth + 1)?;
                    entries.push((key, v));
                }
                check_map_keys(&entries)?;
                Ok(Value::Map(entries))
            }
            // Negative integers, byte strings, tags, floats, and simple values.
            _ => Err(SyntaxError::OutsideModel),
        }
    }

    fn text(&mut self, n: u64) -> Result<String, SyntaxError> {
        if n > MAX_TEXT as u64 {
            return Err(SyntaxError::LimitExceeded);
        }
        let raw = self.take(n as usize)?;
        let s = core::str::from_utf8(raw).map_err(|_| SyntaxError::InvalidText)?;
        Ok(String::from(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| alloc::format!("{b:02x}")).collect()
    }

    /// Integer and length boundaries, from RFC 8949 Appendix A.
    #[test]
    fn integers_take_their_shortest_form() {
        let cases: [(u64, &str); 9] = [
            (0, "00"),
            (1, "01"),
            (23, "17"),
            (24, "1818"),
            (255, "18ff"),
            (256, "190100"),
            (65_535, "19ffff"),
            (65_536, "1a00010000"),
            (1_000_000_000_000, "1b000000e8d4a51000"),
        ];
        for (n, expected) in cases {
            assert_eq!(hex(&encode(&Value::Uint(n))), expected, "{n}");
            assert_eq!(decode(&encode(&Value::Uint(n))), Ok(Value::Uint(n)));
        }
    }

    /// RFC 8949 Appendix A's text and container examples.
    #[test]
    fn appendix_a_examples() {
        assert_eq!(hex(&encode(&Value::text(""))), "60");
        assert_eq!(hex(&encode(&Value::text("a"))), "6161");
        assert_eq!(hex(&encode(&Value::text("IETF"))), "6449455446");
        assert_eq!(hex(&encode(&Value::text("\u{00fc}"))), "62c3bc");
        assert_eq!(hex(&encode(&Value::Array(vec![]))), "80");
        assert_eq!(
            hex(&encode(&Value::Array(vec![
                Value::Uint(1),
                Value::Uint(2),
                Value::Uint(3)
            ]))),
            "83010203"
        );
        assert_eq!(hex(&encode(&Value::Map(vec![]))), "a0");
    }

    /// RFC 8949 §4.2.1: keys sorted by their encodings, so a shorter key
    /// comes first whatever its letters.
    #[test]
    fn map_keys_sort_by_their_encoding() {
        let map = Value::Map(vec![
            (String::from("bb"), Value::Uint(1)),
            (String::from("a"), Value::Uint(2)),
            (String::from("ab"), Value::Uint(3)),
        ]);
        assert_eq!(hex(&encode(&map)), "a36161026261620362626201");
    }

    #[test]
    fn items_outside_the_model_are_rejected() {
        let cases: [(&str, SyntaxError); 10] = [
            ("20", SyntaxError::OutsideModel),       // -1
            ("40", SyntaxError::OutsideModel),       // empty byte string
            ("c000", SyntaxError::OutsideModel),     // tag 0
            ("f4", SyntaxError::OutsideModel),       // false
            ("f6", SyntaxError::OutsideModel),       // null
            ("f93c00", SyntaxError::OutsideModel),   // 1.0, half precision
            ("9fff", SyntaxError::OutsideModel),     // indefinite array
            ("7f6161ff", SyntaxError::OutsideModel), // indefinite text
            ("a1016161", SyntaxError::OutsideModel), // integer key
            ("1c", SyntaxError::Malformed),          // reserved
        ];
        for (input, error) in cases {
            assert_eq!(decode(&unhex(input)), Err(error), "{input}");
        }
    }

    #[test]
    fn broken_inputs_are_rejected() {
        assert_eq!(decode(&unhex("")), Err(SyntaxError::Truncated));
        assert_eq!(decode(&unhex("62c3")), Err(SyntaxError::Truncated));
        assert_eq!(decode(&unhex("0000")), Err(SyntaxError::TrailingBytes));
        assert_eq!(decode(&unhex("61ff")), Err(SyntaxError::InvalidText));
        assert_eq!(
            decode(&unhex("a2616101616102")),
            Err(SyntaxError::DuplicateKey)
        );
        assert_eq!(
            decode(&unhex("1b0020000000000000")),
            Err(SyntaxError::LimitExceeded)
        );
        assert_eq!(
            decode(&unhex("9a00ffffff")),
            Err(SyntaxError::LimitExceeded)
        );
        let deep = "81".repeat(9) + "00";
        assert_eq!(decode(&unhex(&deep)), Err(SyntaxError::LimitExceeded));
    }

    /// A non-shortest form is well formed and decodes; the canonical-form
    /// stage, not the parser, rejects it.
    #[test]
    fn the_parser_accepts_what_the_canonical_stage_rejects() {
        assert_eq!(decode(&unhex("1801")), Ok(Value::Uint(1)));
        assert_ne!(encode(&Value::Uint(1)), unhex("1801"));
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
