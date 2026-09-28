//! Strict JSON parsing for NDJSON records.
//!
//! `serde_json` already rejects `NaN` and `Infinity`, which are not JSON. What
//! it does not reject is a duplicate object key: the last value silently wins.
//! A research record with two `id` fields is corrupt, not ambiguous, so this
//! module parses through a visitor that fails on the second occurrence of a key.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

use crate::error::{Code, VerificationError, Verified};

const DUPLICATE_KEY: &str = "Duplicate JSON object key";
const NON_FINITE: &str = "Non-finite JSON number";

/// Parses one JSON text into a `Value`, rejecting duplicate object keys.
///
/// Failures are classified: a duplicate key, a non-finite number (`NaN`,
/// `Infinity`, `-Infinity`, which JSON does not have), or any other syntax error.
pub(crate) fn parse(text: &str) -> Verified<Value> {
    let mut de = serde_json::Deserializer::from_str(text);
    let value = Strict::deserialize(&mut de)
        .and_then(|value| de.end().map(|()| value))
        .map_err(|error| classify(text, &error))?;
    Ok(value.0)
}

fn classify(text: &str, error: &serde_json::Error) -> VerificationError {
    let message = error.to_string();
    if message.starts_with(DUPLICATE_KEY) {
        return VerificationError::new(Code::JsonDuplicateKey, message);
    }
    if message.starts_with(NON_FINITE) || non_finite_token_near(text, error.column()) {
        return VerificationError::new(Code::JsonNonFiniteNumber, message);
    }
    VerificationError::new(Code::JsonSyntax, message)
}

/// `serde_json` reports a bare `NaN` as a syntax error at the token. Look there.
fn non_finite_token_near(text: &str, column: usize) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let start = column.saturating_sub(3);
    (start..column).any(|index| {
        let tail: String = chars.iter().skip(index).take(9).collect();
        ["NaN", "Infinity", "-Infinity"]
            .iter()
            .any(|token| tail.starts_with(token))
    })
}

struct Strict(Value);

impl<'de> de::Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor).map(Strict)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom(format!("{NON_FINITE}: {v}")))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(Strict(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format!("{DUPLICATE_KEY}: {key}")));
            }
            let Strict(value) = map.next_value()?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::error::Code;

    fn code(text: &str) -> Code {
        parse(text).expect_err("expected a rejection").code()
    }

    #[test]
    fn accepts_ordinary_objects() {
        let v = parse(r#"{"a": 1, "b": [true, null, "x"], "c": {"d": 2.5}}"#).unwrap();
        assert_eq!(v["b"][2], "x");
        assert_eq!(v["c"]["d"], 2.5);
    }

    #[test]
    fn rejects_duplicate_keys_at_any_depth_as_duplicate_keys() {
        assert_eq!(code(r#"{"x": 1, "x": 2}"#), Code::JsonDuplicateKey);
        assert_eq!(code(r#"{"a": {"x": 1, "x": 2}}"#), Code::JsonDuplicateKey);
        assert_eq!(code(r#"[{"x": 1, "x": 2}]"#), Code::JsonDuplicateKey);
    }

    #[test]
    fn rejects_non_finite_constants_as_non_finite() {
        assert_eq!(code(r#"{"x": NaN}"#), Code::JsonNonFiniteNumber);
        assert_eq!(code(r#"{"x":NaN}"#), Code::JsonNonFiniteNumber);
        assert_eq!(code(r#"{"x": Infinity}"#), Code::JsonNonFiniteNumber);
        assert_eq!(code(r#"{"x": -Infinity}"#), Code::JsonNonFiniteNumber);
    }

    #[test]
    fn rejects_other_malformed_text_as_syntax() {
        assert_eq!(code(r#"{"x": 1} {"y": 2}"#), Code::JsonSyntax);
        assert_eq!(code(r#"{"x": }"#), Code::JsonSyntax);
        assert_eq!(code(r#"{"x": Nope}"#), Code::JsonSyntax);
    }
}
