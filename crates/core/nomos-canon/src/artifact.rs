//! The Canon artifact: encoding, strict decoding, archival inspection,
//! migration, and `CanonID` ([canon-ir.md](../../../../docs/formal/canon-ir.md)).
//!
//! Decoding runs the four stages of canon-ir.md, and one more check that
//! makes the second exact: the validated Canon must re-encode, in the schema
//! it was read from, to the input bytes. Bytes that decode but are not the
//! canonical encoding of what they decode to are rejected, so one Canon has
//! one encoding per profile and schema.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::cbor;
use crate::jcs;
use crate::model::{Canon, CanonError, Kind, RawCanon, RawRelation, RawResource, RawSpec};
use crate::sha256;
use crate::value::{SyntaxError, Value};

/// The first schema: files only.
pub const SCHEMA_V1: u64 = 1;
/// The current schema.
pub const SCHEMA_V2: u64 = 2;

/// An encoding profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Profile {
    /// Deterministic CBOR, RFC 8949 §4.2.1.
    Cbor,
    /// The JSON Canonicalization Scheme, RFC 8785.
    Jcs,
}

impl Profile {
    /// Both profiles.
    pub const ALL: [Profile; 2] = [Profile::Cbor, Profile::Jcs];

    /// The canonical encoding of a value of the data model.
    pub fn encode_value(&self, value: &Value) -> Vec<u8> {
        match self {
            Profile::Cbor => cbor::encode(value),
            Profile::Jcs => jcs::encode(value),
        }
    }

    /// Parses bytes into a value of the data model.
    pub fn decode_value(&self, bytes: &[u8]) -> Result<Value, SyntaxError> {
        match self {
            Profile::Cbor => cbor::decode(bytes),
            Profile::Jcs => jcs::decode(bytes),
        }
    }

    fn tag(&self) -> &'static [u8] {
        match self {
            Profile::Cbor => b"nomos.canon-id.cbor",
            Profile::Jcs => b"nomos.canon-id.jcs",
        }
    }
}

/// What a reader can execute: the schema versions it reads and the resource
/// kinds it knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reader {
    schemas: BTreeSet<u64>,
    kinds: BTreeSet<Kind>,
}

impl Reader {
    /// The current reader: versions 1 and 2, files and services.
    pub fn current() -> Self {
        Reader::new(&[SCHEMA_V1, SCHEMA_V2], &[Kind::File, Kind::Service])
    }

    /// The version 1 reader: version 1, files only. The compatibility tests
    /// use it as an old reader.
    pub fn v1() -> Self {
        Reader::new(&[SCHEMA_V1], &[Kind::File])
    }

    /// A reader of `schemas` and `kinds`.
    pub fn new(schemas: &[u64], kinds: &[Kind]) -> Self {
        Reader {
            schemas: schemas.iter().copied().collect(),
            kinds: kinds.iter().copied().collect(),
        }
    }
}

/// A content-derived Canon identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonId([u8; 32]);

impl CanonId {
    /// The digest bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for CanonId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sha256:")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CanonId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CanonId({self})")
    }
}

/// Where in the IR a schema error is. The text is the schema's, never the
/// input's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    /// The structure: `canon`, `resource`, `spec`, `relation`, or `file`.
    pub what: &'static str,
    /// Its position in its array, if it is in one.
    pub index: Option<usize>,
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.index {
            Some(i) => write!(f, "{} {i}", self.what),
            None => f.write_str(self.what),
        }
    }
}

/// Why bytes are not an artifact this reader may execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Stage 1: not a value of the data model under the profile.
    Syntax(SyntaxError),
    /// Stage 2: not the canonical encoding of what the bytes decode to.
    NonCanonical,
    /// Stage 3: a schema version the reader does not read.
    UnsupportedSchema,
    /// Stage 3: a field the schema does not define.
    UnknownField(Place),
    /// Stage 3: a field the schema requires is missing.
    MissingField(Place),
    /// Stage 3: a field of the wrong type.
    WrongType(Place),
    /// Stage 3: a resource kind the reader does not know. Rejected, never
    /// skipped: skipping would silently change executable intent.
    UnsupportedKind {
        /// The resource's position.
        resource: usize,
    },
    /// Stage 4: the validator rejected the Canon.
    Invalid(CanonError),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Syntax(e) => write!(f, "syntax: {e}"),
            DecodeError::NonCanonical => f.write_str("not the canonical encoding"),
            DecodeError::UnsupportedSchema => f.write_str("unsupported schema version"),
            DecodeError::UnknownField(p) => write!(f, "{p}: a field the schema does not define"),
            DecodeError::MissingField(p) => write!(f, "{p}: a required field is missing"),
            DecodeError::WrongType(p) => write!(f, "{p}: a field of the wrong type"),
            DecodeError::UnsupportedKind { resource } => {
                write!(
                    f,
                    "resource {resource}: a kind this reader does not execute"
                )
            }
            DecodeError::Invalid(e) => write!(f, "invalid Canon: {e}"),
        }
    }
}

/// What a migration from an older schema records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lineage {
    /// The schema the artifact was written in.
    pub from_schema: u64,
    /// The SHA-256 digest of the original artifact's bytes.
    pub from_digest: [u8; 32],
    /// The `CanonID` of the migrated Canon in the current schema.
    pub to: CanonId,
}

/// A decoded, validated Canon and how it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The Canon.
    pub canon: Canon,
    /// The schema the artifact was written in.
    pub schema: u64,
    /// Present when the artifact was migrated.
    pub lineage: Option<Lineage>,
}

/// What archival inspection reports. It is not a Canon and cannot become
/// one: preserving an artifact never implies permission to execute it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// The schema version the artifact declares.
    pub schema: u64,
    /// The name it declares, if it has one as text.
    pub name: Option<String>,
    /// The SHA-256 digest of its bytes.
    pub digest: [u8; 32],
}

// ---------------------------------------------------------------------------
// Encoding

fn text_array(items: &[String]) -> Value {
    Value::Array(items.iter().map(|s| Value::Text(s.clone())).collect())
}

fn spec_value(spec: &RawSpec) -> Value {
    let mut entries = alloc::vec![
        (String::from("kind"), Value::Text(spec.kind.clone())),
        (String::from("state"), Value::Text(spec.state.clone())),
    ];
    if let Some(d) = &spec.digest {
        entries.push((String::from("digest"), Value::Text(d.clone())));
    }
    Value::Map(entries)
}

fn relations_value(relations: &[RawRelation]) -> Value {
    Value::Array(
        relations
            .iter()
            .map(|r| {
                Value::Map(alloc::vec![
                    (String::from("source"), Value::Text(r.source.clone())),
                    (String::from("target"), Value::Text(r.target.clone())),
                    (String::from("kind"), Value::Text(r.kind.clone())),
                ])
            })
            .collect(),
    )
}

/// The schema-2 value of a `RawCanon`, as given: no validation, no
/// normalization. For writing fixtures and adversarial inputs; decoding
/// validates whatever this produces.
pub fn raw_value(raw: &RawCanon) -> Value {
    Value::Map(alloc::vec![
        (String::from("schema"), Value::Uint(SCHEMA_V2)),
        (String::from("name"), Value::Text(raw.name.clone())),
        (
            String::from("resources"),
            Value::Array(
                raw.resources
                    .iter()
                    .map(|r| {
                        Value::Map(alloc::vec![
                            (String::from("path"), Value::Text(r.path.clone())),
                            (String::from("spec"), spec_value(&r.spec)),
                            (String::from("keys"), text_array(&r.keys)),
                            (String::from("disrupts"), text_array(&r.disrupts)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (String::from("relations"), relations_value(&raw.relations)),
    ])
}

/// The schema-1 value of a `RawCanon`, or `None` if it uses anything
/// schema 1 cannot say: a service, a conflict key, or a node.
pub fn raw_value_v1(raw: &RawCanon) -> Option<Value> {
    let mut files = Vec::new();
    for r in &raw.resources {
        if r.spec.kind != "file" || !r.keys.is_empty() || !r.disrupts.is_empty() {
            return None;
        }
        let mut entries = alloc::vec![
            (String::from("path"), Value::Text(r.path.clone())),
            (String::from("state"), Value::Text(r.spec.state.clone())),
        ];
        if let Some(d) = &r.spec.digest {
            entries.push((String::from("digest"), Value::Text(d.clone())));
        }
        files.push(Value::Map(entries));
    }
    Some(Value::Map(alloc::vec![
        (String::from("schema"), Value::Uint(SCHEMA_V1)),
        (String::from("name"), Value::Text(raw.name.clone())),
        (String::from("files"), Value::Array(files)),
        (String::from("relations"), relations_value(&raw.relations)),
    ]))
}

/// The canonical schema-2 artifact of `canon`.
pub fn encode(canon: &Canon, profile: Profile) -> Vec<u8> {
    profile.encode_value(&raw_value(&canon.to_raw()))
}

/// The canonical schema-1 artifact of `canon`, if schema 1 can say it.
pub fn encode_v1(canon: &Canon, profile: Profile) -> Option<Vec<u8>> {
    raw_value_v1(&canon.to_raw()).map(|v| profile.encode_value(&v))
}

fn id(profile: Profile, schema: u64, bytes: &[u8]) -> CanonId {
    let mut h = sha256::Sha256::new();
    h.update(profile.tag());
    h.update(&[0]);
    h.update(&schema.to_be_bytes());
    h.update(bytes);
    CanonId(h.finish())
}

/// The `CanonID` of `canon` under `profile`: SHA-256 over the profile's tag,
/// a zero byte, the schema version, and the canonical encoding.
pub fn canon_id(canon: &Canon, profile: Profile) -> CanonId {
    id(profile, SCHEMA_V2, &encode(canon, profile))
}

// ---------------------------------------------------------------------------
// Decoding

struct Fields<'a> {
    entries: &'a [(String, Value)],
    place: Place,
}

impl<'a> Fields<'a> {
    fn of(value: &'a Value, place: Place, allowed: &[&str]) -> Result<Self, DecodeError> {
        let Value::Map(entries) = value else {
            return Err(DecodeError::WrongType(place));
        };
        if entries.iter().any(|(k, _)| !allowed.contains(&k.as_str())) {
            return Err(DecodeError::UnknownField(place));
        }
        Ok(Fields { entries, place })
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn required(&self, key: &str) -> Result<&'a Value, DecodeError> {
        self.get(key).ok_or(DecodeError::MissingField(self.place))
    }

    fn text(&self, key: &str) -> Result<String, DecodeError> {
        match self.required(key)? {
            Value::Text(s) => Ok(s.clone()),
            _ => Err(DecodeError::WrongType(self.place)),
        }
    }

    fn optional_text(&self, key: &str) -> Result<Option<String>, DecodeError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Text(s)) => Ok(Some(s.clone())),
            Some(_) => Err(DecodeError::WrongType(self.place)),
        }
    }

    fn array(&self, key: &str) -> Result<&'a [Value], DecodeError> {
        match self.required(key)? {
            Value::Array(items) => Ok(items),
            _ => Err(DecodeError::WrongType(self.place)),
        }
    }

    fn texts(&self, key: &str) -> Result<Vec<String>, DecodeError> {
        self.array(key)?
            .iter()
            .map(|v| match v {
                Value::Text(s) => Ok(s.clone()),
                _ => Err(DecodeError::WrongType(self.place)),
            })
            .collect()
    }
}

fn place(what: &'static str, index: Option<usize>) -> Place {
    Place { what, index }
}

fn schema_of(value: &Value) -> Result<u64, DecodeError> {
    let Value::Map(_) = value else {
        return Err(DecodeError::WrongType(place("canon", None)));
    };
    match value.get("schema") {
        Some(Value::Uint(v)) => Ok(*v),
        Some(_) => Err(DecodeError::WrongType(place("canon", None))),
        None => Err(DecodeError::MissingField(place("canon", None))),
    }
}

fn kind_of(text: &str) -> Option<Kind> {
    match text {
        "file" => Some(Kind::File),
        "service" => Some(Kind::Service),
        _ => None,
    }
}

fn relations(top: &Fields<'_>) -> Result<Vec<RawRelation>, DecodeError> {
    top.array("relations")?
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let f = Fields::of(v, place("relation", Some(i)), &["source", "target", "kind"])?;
            Ok(RawRelation {
                source: f.text("source")?,
                target: f.text("target")?,
                kind: f.text("kind")?,
            })
        })
        .collect()
}

fn raw_v2(value: &Value, reader: &Reader) -> Result<RawCanon, DecodeError> {
    let top = Fields::of(
        value,
        place("canon", None),
        &["schema", "name", "resources", "relations"],
    )?;
    let mut resources = Vec::new();
    for (i, v) in top.array("resources")?.iter().enumerate() {
        let f = Fields::of(
            v,
            place("resource", Some(i)),
            &["path", "spec", "keys", "disrupts"],
        )?;
        let s = Fields::of(
            f.required("spec")?,
            place("spec", Some(i)),
            &["kind", "state", "digest"],
        )?;
        let kind = s.text("kind")?;
        match kind_of(&kind) {
            Some(k) if reader.kinds.contains(&k) => {}
            _ => return Err(DecodeError::UnsupportedKind { resource: i }),
        }
        resources.push(RawResource {
            path: f.text("path")?,
            spec: RawSpec {
                kind,
                state: s.text("state")?,
                digest: s.optional_text("digest")?,
            },
            keys: f.texts("keys")?,
            disrupts: f.texts("disrupts")?,
        });
    }
    Ok(RawCanon {
        name: top.text("name")?,
        resources,
        relations: relations(&top)?,
    })
}

/// Schema 1 into the current DTO: each file becomes a `file` resource with
/// no conflict keys and no disruption, and nothing else is added.
fn raw_v1(value: &Value, reader: &Reader) -> Result<RawCanon, DecodeError> {
    let top = Fields::of(
        value,
        place("canon", None),
        &["schema", "name", "files", "relations"],
    )?;
    let mut resources = Vec::new();
    for (i, v) in top.array("files")?.iter().enumerate() {
        if !reader.kinds.contains(&Kind::File) {
            return Err(DecodeError::UnsupportedKind { resource: i });
        }
        let f = Fields::of(v, place("file", Some(i)), &["path", "state", "digest"])?;
        resources.push(RawResource {
            path: f.text("path")?,
            spec: RawSpec {
                kind: String::from("file"),
                state: f.text("state")?,
                digest: f.optional_text("digest")?,
            },
            keys: Vec::new(),
            disrupts: Vec::new(),
        });
    }
    Ok(RawCanon {
        name: top.text("name")?,
        resources,
        relations: relations(&top)?,
    })
}

/// Decodes an artifact for execution by `reader`, in the four stages of
/// canon-ir.md. A schema-1 artifact is migrated and carries its lineage.
pub fn decode(bytes: &[u8], profile: Profile, reader: &Reader) -> Result<Decoded, DecodeError> {
    // Stage 1: syntax.
    let value = profile.decode_value(bytes).map_err(DecodeError::Syntax)?;
    // Stage 2: canonical form of the value.
    if profile.encode_value(&value) != bytes {
        return Err(DecodeError::NonCanonical);
    }
    // Stage 3: schema, fields, and kinds.
    let schema = schema_of(&value)?;
    if !reader.schemas.contains(&schema) {
        return Err(DecodeError::UnsupportedSchema);
    }
    let raw = match schema {
        SCHEMA_V1 => raw_v1(&value, reader)?,
        SCHEMA_V2 => raw_v2(&value, reader)?,
        _ => return Err(DecodeError::UnsupportedSchema),
    };
    // Stage 4: the validator.
    let canon = Canon::try_from(raw).map_err(DecodeError::Invalid)?;
    // The Canon, re-encoded in the schema it was read from, is the input:
    // bytes that differ only in what normalization removes are rejected.
    let again = match schema {
        SCHEMA_V1 => encode_v1(&canon, profile),
        _ => Some(encode(&canon, profile)),
    };
    if again.as_deref() != Some(bytes) {
        return Err(DecodeError::NonCanonical);
    }
    let lineage = (schema == SCHEMA_V1).then(|| Lineage {
        from_schema: SCHEMA_V1,
        from_digest: sha256::digest(bytes),
        to: canon_id(&canon, profile),
    });
    Ok(Decoded {
        canon,
        schema,
        lineage,
    })
}

/// Archival inspection: stages 1 and 2 only, for any schema version and any
/// kinds. Returns no Canon.
pub fn inspect(bytes: &[u8], profile: Profile) -> Result<Inspection, DecodeError> {
    let value = profile.decode_value(bytes).map_err(DecodeError::Syntax)?;
    if profile.encode_value(&value) != bytes {
        return Err(DecodeError::NonCanonical);
    }
    let schema = schema_of(&value)?;
    let name = match value.get("name") {
        Some(Value::Text(s)) => Some(s.clone()),
        _ => None,
    };
    Ok(Inspection {
        schema,
        name,
        digest: sha256::digest(bytes),
    })
}
