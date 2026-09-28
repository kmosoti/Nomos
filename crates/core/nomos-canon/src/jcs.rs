//! The JSON Canonicalization Scheme (JCS) of RFC 8785 over the data model of
//! [`crate::value`].
//!
//! The encoder writes no whitespace, object members sorted by the UTF-16
//! code units of their names, strings with only the escapes RFC 8785
//! requires (quotation mark, reverse solidus, and control characters, the
//! latter as `\b`, `\t`, `\n`, `\f`, `\r`, or `\u00xx`), and integers in their
//! shortest decimal form. The decoder accepts RFC 8259 JSON within the data
//! model: whitespace and optional escapes parse, and the canonical-form
//! stage of [`crate::artifact`] rejects them by re-encoding and comparing.

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::value::{
    MAX_ARRAY, MAX_ARTIFACT, MAX_DEPTH, MAX_MAP, MAX_TEXT, MAX_UINT, SyntaxError, Value,
    check_map_keys,
};

/// The canonical encoding of `value`.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = String::new();
    write(value, &mut out);
    out.into_bytes()
}

fn utf16_order(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn write(value: &Value, out: &mut String) {
    match value {
        Value::Uint(n) => {
            let mut digits = [0u8; 20];
            let mut i = digits.len();
            let mut n = *n;
            loop {
                i -= 1;
                digits[i] = b'0' + (n % 10) as u8;
                n /= 10;
                if n == 0 {
                    break;
                }
            }
            for d in &digits[i..] {
                out.push(char::from(*d));
            }
        }
        Value::Text(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Value::Map(entries) => {
            let mut sorted: Vec<&(String, Value)> = entries.iter().collect();
            sorted.sort_by(|a, b| utf16_order(&a.0, &b.0));
            out.push('{');
            for (i, (k, v)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write(v, out);
            }
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                let n = c as u32 as usize;
                out.push_str("\\u00");
                out.push(char::from(HEX[n >> 4]));
                out.push(char::from(HEX[n & 0xf]));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Decodes one value of the data model from `bytes`, which must hold
/// exactly that value, with optional whitespace around it.
pub fn decode(bytes: &[u8]) -> Result<Value, SyntaxError> {
    if bytes.len() > MAX_ARTIFACT {
        return Err(SyntaxError::LimitExceeded);
    }
    let text = core::str::from_utf8(bytes).map_err(|_| SyntaxError::InvalidText)?;
    let mut p = Parser {
        s: text.as_bytes(),
        at: 0,
    };
    p.whitespace();
    let value = p.value(0)?;
    p.whitespace();
    if p.at != p.s.len() {
        return Err(SyntaxError::TrailingBytes);
    }
    Ok(value)
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn next(&mut self) -> Result<u8, SyntaxError> {
        let b = self.peek().ok_or(SyntaxError::Truncated)?;
        self.at += 1;
        Ok(b)
    }

    fn expect(&mut self, b: u8) -> Result<(), SyntaxError> {
        if self.next()? == b {
            Ok(())
        } else {
            Err(SyntaxError::Malformed)
        }
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, SyntaxError> {
        match self.peek().ok_or(SyntaxError::Truncated)? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => Ok(Value::Text(self.string()?)),
            b'0'..=b'9' => self.number(),
            b'-' => {
                self.number()?;
                Err(SyntaxError::OutsideModel)
            }
            b't' => self.literal(b"true"),
            b'f' => self.literal(b"false"),
            b'n' => self.literal(b"null"),
            _ => Err(SyntaxError::Malformed),
        }
    }

    /// `true`, `false`, and `null` are well formed and outside the model.
    fn literal(&mut self, word: &[u8]) -> Result<Value, SyntaxError> {
        for b in word {
            self.expect(*b)?;
        }
        Err(SyntaxError::OutsideModel)
    }

    fn number(&mut self) -> Result<Value, SyntaxError> {
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        let start = self.at;
        match self.next()? {
            b'0' => {}
            b'1'..=b'9' => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.at += 1;
                }
            }
            _ => return Err(SyntaxError::Malformed),
        }
        let digits = &self.s[start..self.at];
        let mut fraction_or_exponent = false;
        if self.peek() == Some(b'.') {
            self.at += 1;
            self.digits()?;
            fraction_or_exponent = true;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            self.digits()?;
            fraction_or_exponent = true;
        }
        if fraction_or_exponent {
            return Err(SyntaxError::OutsideModel);
        }
        let mut n: u64 = 0;
        for d in digits {
            n = n
                .checked_mul(10)
                .and_then(|n| n.checked_add(u64::from(d - b'0')))
                .ok_or(SyntaxError::LimitExceeded)?;
        }
        if n > MAX_UINT {
            return Err(SyntaxError::LimitExceeded);
        }
        Ok(Value::Uint(n))
    }

    fn digits(&mut self) -> Result<(), SyntaxError> {
        if !matches!(self.peek(), Some(b'0'..=b'9')) {
            return Err(SyntaxError::Malformed);
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, SyntaxError> {
        let mut n = 0u32;
        for _ in 0..4 {
            let d = match self.next()? {
                b @ b'0'..=b'9' => b - b'0',
                b @ b'a'..=b'f' => b - b'a' + 10,
                b @ b'A'..=b'F' => b - b'A' + 10,
                _ => return Err(SyntaxError::Malformed),
            };
            n = n * 16 + u32::from(d);
        }
        Ok(n)
    }

    fn string(&mut self) -> Result<String, SyntaxError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            // The input is valid UTF-8 and the run ends on an ASCII byte.
            let run = core::str::from_utf8(&self.s[start..self.at])
                .map_err(|_| SyntaxError::InvalidText)?;
            out.push_str(run);
            match self.next()? {
                b'"' => break,
                b'\\' => {
                    let c = match self.next()? {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{08}',
                        b'f' => '\u{0c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let first = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&first) {
                                self.expect(b'\\')
                                    .and_then(|_| self.expect(b'u'))
                                    .map_err(|_| SyntaxError::InvalidText)?;
                                let second = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&second) {
                                    return Err(SyntaxError::InvalidText);
                                }
                                0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
                            } else if (0xdc00..0xe000).contains(&first) {
                                return Err(SyntaxError::InvalidText);
                            } else {
                                first
                            };
                            char::from_u32(code).ok_or(SyntaxError::InvalidText)?
                        }
                        _ => return Err(SyntaxError::Malformed),
                    };
                    out.push(c);
                }
                // A raw control character inside a string.
                _ => return Err(SyntaxError::Malformed),
            }
            if out.len() > MAX_TEXT {
                return Err(SyntaxError::LimitExceeded);
            }
        }
        if out.len() > MAX_TEXT {
            return Err(SyntaxError::LimitExceeded);
        }
        Ok(out)
    }

    fn array(&mut self, depth: usize) -> Result<Value, SyntaxError> {
        if depth >= MAX_DEPTH {
            return Err(SyntaxError::LimitExceeded);
        }
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.whitespace();
            items.push(self.value(depth + 1)?);
            if items.len() > MAX_ARRAY {
                return Err(SyntaxError::LimitExceeded);
            }
            self.whitespace();
            match self.next()? {
                b',' => continue,
                b']' => break,
                _ => return Err(SyntaxError::Malformed),
            }
        }
        Ok(Value::Array(items))
    }

    fn object(&mut self, depth: usize) -> Result<Value, SyntaxError> {
        if depth >= MAX_DEPTH {
            return Err(SyntaxError::LimitExceeded);
        }
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Map(entries));
        }
        loop {
            self.whitespace();
            if self.peek() != Some(b'"') {
                return Err(SyntaxError::Malformed);
            }
            let key = self.string()?;
            self.whitespace();
            self.expect(b':')?;
            self.whitespace();
            let v = self.value(depth + 1)?;
            entries.push((key, v));
            if entries.len() > MAX_MAP {
                return Err(SyntaxError::LimitExceeded);
            }
            self.whitespace();
            match self.next()? {
                b',' => continue,
                b'}' => break,
                _ => return Err(SyntaxError::Malformed),
            }
        }
        check_map_keys(&entries)?;
        Ok(Value::Map(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn enc(v: &Value) -> String {
        String::from_utf8(encode(v)).unwrap()
    }

    /// RFC 8785 §3.2.3: members sorted by UTF-16 code units, which puts a
    /// character above the Basic Multilingual Plane before U+FB33.
    #[test]
    fn members_sort_by_utf16_code_units() {
        let map = Value::Map(vec![
            (String::from("\u{fb33}"), Value::Uint(1)),
            (String::from("\u{1f600}"), Value::Uint(2)),
            (String::from("b"), Value::Uint(3)),
            (String::from("a"), Value::Uint(4)),
        ]);
        assert_eq!(
            enc(&map),
            "{\"a\":4,\"b\":3,\"\u{1f600}\":2,\"\u{fb33}\":1}"
        );
    }

    /// RFC 8785 §3.2.2.2: only the required escapes, lowercase hex.
    #[test]
    fn strings_carry_only_the_required_escapes() {
        let s = "\u{0}\u{1f}\u{8}\t\n\u{c}\r\"\\/\u{7f}\u{e9}\u{2028}";
        assert_eq!(
            enc(&Value::text(s)),
            "\"\\u0000\\u001f\\b\\t\\n\\f\\r\\\"\\\\/\u{7f}\u{e9}\u{2028}\""
        );
        assert_eq!(decode(&encode(&Value::text(s))), Ok(Value::text(s)));
    }

    #[test]
    fn integers_have_one_form() {
        for n in [0u64, 7, 10, 4096, MAX_UINT] {
            assert_eq!(enc(&Value::Uint(n)), alloc::format!("{n}"));
            assert_eq!(decode(&encode(&Value::Uint(n))), Ok(Value::Uint(n)));
        }
    }

    #[test]
    fn items_outside_the_model_are_rejected() {
        let cases: [(&str, SyntaxError); 12] = [
            ("-1", SyntaxError::OutsideModel),
            ("-0", SyntaxError::OutsideModel),
            ("1.0", SyntaxError::OutsideModel),
            ("1e0", SyntaxError::OutsideModel),
            ("true", SyntaxError::OutsideModel),
            ("null", SyntaxError::OutsideModel),
            ("01", SyntaxError::TrailingBytes),
            ("9007199254740992", SyntaxError::LimitExceeded),
            ("\"\\ud800\"", SyntaxError::InvalidText),
            ("\"\\udc00\"", SyntaxError::InvalidText),
            ("{\"a\":1,\"a\":2}", SyntaxError::DuplicateKey),
            ("[1,]", SyntaxError::Malformed),
        ];
        for (input, error) in cases {
            assert_eq!(decode(input.as_bytes()), Err(error), "{input}");
        }
        assert_eq!(decode(b"\xef\xbb\xbf1"), Err(SyntaxError::Malformed));
        assert_eq!(decode(b"\"\xff\""), Err(SyntaxError::InvalidText));
        assert_eq!(decode(b"\"a\x01\""), Err(SyntaxError::Malformed));
    }

    #[test]
    fn empty_containers_decode() {
        assert_eq!(decode(b"{}"), Ok(Value::Map(vec![])));
        assert_eq!(decode(b"[]"), Ok(Value::Array(vec![])));
        assert_eq!(
            decode(b"[{},[]]"),
            Ok(Value::Array(vec![Value::Map(vec![]), Value::Array(vec![])]))
        );
    }

    /// The number grammar of RFC 8259: a fraction or an exponent without
    /// digits is malformed; a complete one is well formed and outside the
    /// model.
    #[test]
    fn numbers_follow_the_grammar() {
        assert_eq!(decode(b"-"), Err(SyntaxError::Truncated));
        for input in ["1.", "1.e5", "1e", "1e+", "1.5e", "-x", "-.5"] {
            assert_eq!(
                decode(input.as_bytes()),
                Err(SyntaxError::Malformed),
                "{input}"
            );
        }
        for input in ["1.5", "1e5", "1e+5", "1E-5", "0.0", "-7", "false"] {
            assert_eq!(
                decode(input.as_bytes()),
                Err(SyntaxError::OutsideModel),
                "{input}"
            );
        }
    }

    /// Whitespace and optional escapes parse; the canonical-form stage, not
    /// the parser, rejects them.
    #[test]
    fn the_parser_accepts_what_the_canonical_stage_rejects() {
        assert_eq!(decode(b" [ 1 ] "), Ok(Value::Array(vec![Value::Uint(1)])));
        assert_eq!(decode(b"\"\\u0061\""), Ok(Value::text("a")));
        assert_eq!(
            decode(b"\"\\u00E9\\u00e9\""),
            Ok(Value::text("\u{e9}\u{e9}"))
        );
        assert_eq!(decode(b"\"\\u004A\\u00Ff\""), Ok(Value::text("J\u{ff}")));
        assert_eq!(decode(b"\"a\\/b\""), Ok(Value::text("a/b")));
        assert_eq!(decode(b"\"\\ud83d\\ude00\""), Ok(Value::text("\u{1f600}")));
        assert_ne!(encode(&Value::text("a")), b"\"\\u0061\"".to_vec());
    }
}
