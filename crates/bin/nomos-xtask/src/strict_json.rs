//! Strict JSON parsing for NDJSON records.
//!
//! `serde_json` already rejects `NaN` and `Infinity`, which are not JSON. What
//! it does not reject is a duplicate object key: the last value silently wins.
//! A research record with two `id` fields is corrupt, not ambiguous, so this
//! module parses through a visitor that fails on the second occurrence of a key.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

/// Parses one JSON text into a `Value`, rejecting duplicate object keys.
pub(crate) fn parse(text: &str) -> Result<Value, String> {
    let mut de = serde_json::Deserializer::from_str(text);
    let value = Strict::deserialize(&mut de).map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())?;
    Ok(value.0)
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
            .ok_or_else(|| E::custom(format!("Non-finite JSON number: {v}")))
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
                return Err(de::Error::custom(format!(
                    "Duplicate JSON object key: {key}"
                )));
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

    #[test]
    fn accepts_ordinary_objects() {
        let v = parse(r#"{"a": 1, "b": [true, null, "x"], "c": {"d": 2.5}}"#).unwrap();
        assert_eq!(v["b"][2], "x");
        assert_eq!(v["c"]["d"], 2.5);
    }

    #[test]
    fn rejects_duplicate_keys_at_any_depth() {
        assert!(parse(r#"{"x": 1, "x": 2}"#).is_err());
        assert!(parse(r#"{"a": {"x": 1, "x": 2}}"#).is_err());
        assert!(parse(r#"[{"x": 1, "x": 2}]"#).is_err());
    }

    #[test]
    fn rejects_non_finite_constants() {
        assert!(parse(r#"{"x": NaN}"#).is_err());
        assert!(parse(r#"{"x": Infinity}"#).is_err());
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse(r#"{"x": 1} {"y": 2}"#).is_err());
    }
}
