//! Experiment `canonical-encoding` (grounding plan, `06-canon-artifact`):
//! which restricted profile makes semantic equivalence and byte equality
//! coincide on the IR?
//!
//! Both profiles run the same tests. The oracles are from outside the
//! encoders under test: `ciborium` decodes the CBOR and, with its map keys
//! sorted by their encodings, re-encodes it; `serde_json` parses the JSON and
//! `serde_json_canonicalizer` canonicalizes it; `sha2` recomputes every
//! `CanonID`. The golden files under `tests/fixtures/canon/golden/` are the
//! independent encoders' output, written with `NOMOS_CANON_BLESS=1`.

mod support;

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use nomos_canon::artifact::{
    CanonId, DecodeError, Profile, Reader, canon_id, decode, encode, raw_value, raw_value_v1,
};
use nomos_canon::model::{Canon, RawCanon};
use nomos_canon::value::{SyntaxError, Value};
use proptest::prelude::*;
use sha2::Digest as _;
use support::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/canon/golden")
}

fn ext(profile: Profile) -> &'static str {
    match profile {
        Profile::Cbor => "cbor",
        Profile::Jcs => "json",
    }
}

// ---------------------------------------------------------------------------
// The independent references

fn to_ciborium(v: &Value) -> ciborium::Value {
    match v {
        Value::Uint(n) => ciborium::Value::Integer((*n).into()),
        Value::Text(s) => ciborium::Value::Text(s.clone()),
        Value::Array(items) => ciborium::Value::Array(items.iter().map(to_ciborium).collect()),
        Value::Map(entries) => {
            // RFC 8949 §4.2.1, applied by the reference: sort by the encoded key.
            let mut pairs: Vec<(Vec<u8>, ciborium::Value, ciborium::Value)> = entries
                .iter()
                .map(|(k, v)| {
                    let key = ciborium::Value::Text(k.clone());
                    let mut bytes = Vec::new();
                    ciborium::ser::into_writer(&key, &mut bytes).unwrap();
                    (bytes, key, to_ciborium(v))
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            ciborium::Value::Map(pairs.into_iter().map(|(_, k, v)| (k, v)).collect())
        }
    }
}

fn from_ciborium(v: &ciborium::Value) -> Value {
    match v {
        ciborium::Value::Integer(n) => Value::Uint(u64::try_from(*n).unwrap()),
        ciborium::Value::Text(s) => Value::Text(s.clone()),
        ciborium::Value::Array(items) => Value::Array(items.iter().map(from_ciborium).collect()),
        ciborium::Value::Map(entries) => Value::Map(
            entries
                .iter()
                .map(|(k, v)| (k.as_text().unwrap().to_string(), from_ciborium(v)))
                .collect(),
        ),
        other => panic!("outside the model: {other:?}"),
    }
}

fn to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Uint(n) => serde_json::Value::from(*n),
        Value::Text(s) => serde_json::Value::String(s.clone()),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(to_json).collect()),
        Value::Map(entries) => serde_json::Value::Object(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), to_json(v)))
                .collect(),
        ),
    }
}

fn from_json(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Number(n) => Value::Uint(n.as_u64().unwrap()),
        serde_json::Value::String(s) => Value::Text(s.clone()),
        serde_json::Value::Array(items) => Value::Array(items.iter().map(from_json).collect()),
        serde_json::Value::Object(entries) => Value::Map(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), from_json(v)))
                .collect(),
        ),
        other => panic!("outside the model: {other:?}"),
    }
}

/// The reference encoding of `value` under `profile`.
fn reference(value: &Value, profile: Profile) -> Vec<u8> {
    match profile {
        Profile::Cbor => {
            let mut out = Vec::new();
            ciborium::ser::into_writer(&to_ciborium(value), &mut out).unwrap();
            out
        }
        Profile::Jcs => serde_json_canonicalizer::to_vec(&to_json(value)).unwrap(),
    }
}

/// The reference decoding of `bytes` under `profile`, in the order read.
fn reference_decode(bytes: &[u8], profile: Profile) -> Value {
    match profile {
        Profile::Cbor => from_ciborium(&ciborium::de::from_reader(bytes).unwrap()),
        Profile::Jcs => from_json(&serde_json::from_slice(bytes).unwrap()),
    }
}

/// The reference `CanonID`: `sha2` over the preimage canon-ir.md states.
fn reference_id(canon: &Canon, profile: Profile) -> String {
    let tag: &[u8] = match profile {
        Profile::Cbor => b"nomos.canon-id.cbor",
        Profile::Jcs => b"nomos.canon-id.jcs",
    };
    let mut h = sha2::Sha256::new();
    h.update(tag);
    h.update([0u8]);
    h.update(2u64.to_be_bytes());
    h.update(encode(canon, profile));
    format!("sha256:{}", hex(&h.finalize()))
}

/// `v` with every map's entries in key order. Map entries are a set in the
/// model, so values are compared through this.
fn sorted(v: Value) -> Value {
    match v {
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        Value::Map(entries) => {
            let mut entries: Vec<(String, Value)> =
                entries.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Map(entries)
        }
        other => other,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn canon(raw: RawCanon) -> Canon {
    Canon::try_from(raw).unwrap()
}

// ---------------------------------------------------------------------------
// Golden vectors

/// Every golden Canon, in both profiles, schema 2 and, where it can say it,
/// schema 1: the encoder under test and the independent encoder agree, the
/// committed fixture is that encoding, the independent decoder reads the
/// value back, and the identity agrees with `sha2`.
#[test]
fn golden_vectors_agree_with_the_independent_encoders() {
    let bless = std::env::var_os("NOMOS_CANON_BLESS").is_some();
    let mut ids = String::new();
    let mut checked = 0;
    for (name, raw) in golden() {
        let c = canon(raw);
        for profile in Profile::ALL {
            let mut forms = vec![("v2", raw_value(&c.to_raw()), encode(&c, profile))];
            if let Some(v1) = raw_value_v1(&c.to_raw()) {
                let ours = nomos_canon::artifact::encode_v1(&c, profile).unwrap();
                forms.push(("v1", v1, ours));
            }
            for (schema, value, ours) in forms {
                let theirs = reference(&value, profile);
                assert_eq!(
                    ours, theirs,
                    "{name} {schema} {profile:?}: encoders disagree"
                );
                assert_eq!(
                    sorted(reference_decode(&ours, profile)),
                    sorted(value),
                    "{name} {schema} {profile:?}"
                );
                let file = fixtures().join(format!("{name}.{schema}.{}", ext(profile)));
                if bless {
                    fs::create_dir_all(fixtures()).unwrap();
                    fs::write(&file, &theirs).unwrap();
                }
                let committed =
                    fs::read(&file).unwrap_or_else(|_| panic!("missing {}", file.display()));
                assert_eq!(ours, committed, "{}", file.display());
                checked += 1;
            }
            let id = canon_id(&c, profile);
            assert_eq!(
                id.to_string(),
                reference_id(&c, profile),
                "{name} {profile:?}"
            );
            ids.push_str(&format!("{name} {} {id}\n", ext(profile)));
        }
    }
    let ids_file = fixtures().join("ids.txt");
    if bless {
        fs::write(&ids_file, &ids).unwrap();
    }
    assert_eq!(fs::read_to_string(&ids_file).unwrap(), ids);
    // Five Canons in two profiles at schema 2; three of them, the ones with
    // files only and no labels, also at schema 1.
    assert_eq!(checked, 16);
}

#[test]
fn every_golden_vector_decodes_to_its_canon() {
    for (name, raw) in golden() {
        let c = canon(raw);
        for profile in Profile::ALL {
            let decoded = decode(&encode(&c, profile), profile, &Reader::current()).unwrap();
            assert_eq!(decoded.canon, c, "{name} {profile:?}");
            assert_eq!(decoded.schema, 2);
            assert!(decoded.lineage.is_none());
        }
    }
}

// ---------------------------------------------------------------------------
// The equivalence laws

/// Law 1 (metamorphic): the same Canon written in any order, with any
/// duplicates of set members, has one encoding and one identity per profile.
#[test]
fn order_and_duplicates_change_neither_bytes_nor_identity() {
    support::runner(1, 512)
        .run(&(valid_raw(), any::<u64>()), |(raw, seed)| {
            let mut other = raw.clone();
            rotate(&mut other.resources, seed);
            rotate(&mut other.relations, seed >> 3);
            for r in &mut other.resources {
                rotate(&mut r.keys, seed >> 5);
                if let Some(k) = r.keys.first().cloned() {
                    r.keys.push(k);
                }
            }
            if let Some(rel) = other.relations.first().cloned() {
                other.relations.push(rel);
            }
            let (a, b) = (canon(raw), canon(other));
            prop_assert_eq!(&a, &b);
            for profile in Profile::ALL {
                prop_assert_eq!(encode(&a, profile), encode(&b, profile));
                prop_assert_eq!(canon_id(&a, profile), canon_id(&b, profile));
            }
            Ok(())
        })
        .unwrap();
}

/// Law 2 (the equivalence itself): two generated Canons are equal exactly
/// when their encodings are, and exactly when their identities are.
#[test]
fn equivalence_and_byte_equality_coincide() {
    let equal = std::cell::Cell::new(0u32);
    support::runner(2, 1024)
        .run(&(valid_raw(), valid_raw()), |(x, y)| {
            let (a, b) = (canon(x), canon(y));
            for profile in Profile::ALL {
                let same = a == b;
                prop_assert_eq!(same, encode(&a, profile) == encode(&b, profile));
                prop_assert_eq!(same, canon_id(&a, profile) == canon_id(&b, profile));
            }
            if a == b {
                equal.set(equal.get() + 1);
            }
            Ok(())
        })
        .unwrap();
    println!("equal pairs: {} of 1024", equal.get());
}

/// Law 3 (injectivity): whatever a mutated artifact decodes to, it is the
/// canonical encoding of it; otherwise it is rejected. No two byte strings
/// carry one Canon.
#[test]
fn a_decoded_artifact_is_the_canonical_encoding_of_its_canon() {
    let accepted = std::cell::Cell::new(0u32);
    let rejected = std::cell::Cell::new(0u32);
    support::runner(3, 1024)
        .run(
            &(
                valid_raw(),
                prop::collection::vec((any::<usize>(), any::<u8>()), 1..4),
            ),
            |(raw, edits)| {
                let c = canon(raw);
                for profile in Profile::ALL {
                    let mut bytes = encode(&c, profile);
                    for (at, b) in &edits {
                        let i = at % bytes.len();
                        bytes[i] = *b;
                    }
                    match decode(&bytes, profile, &Reader::current()) {
                        Ok(d) => {
                            prop_assert_eq!(encode(&d.canon, profile), bytes);
                            accepted.set(accepted.get() + 1);
                        }
                        Err(_) => rejected.set(rejected.get() + 1),
                    }
                }
                Ok(())
            },
        )
        .unwrap();
    println!(
        "mutated artifacts: {} accepted as their own canonical form, {} rejected",
        accepted.get(),
        rejected.get()
    );
}

/// Law 4 (independent agreement): on generated Canons, the independent
/// encoders produce the same bytes and the independent decoders read the
/// same value; `sha2` gives the same identity.
#[test]
fn generated_canons_agree_with_the_independent_encoders() {
    support::runner(4, 512)
        .run(&valid_raw(), |raw| {
            let c = canon(raw);
            let value = raw_value(&c.to_raw());
            for profile in Profile::ALL {
                let ours = encode(&c, profile);
                prop_assert_eq!(&ours, &reference(&value, profile));
                prop_assert_eq!(
                    sorted(reference_decode(&ours, profile)),
                    sorted(value.clone())
                );
                prop_assert_eq!(canon_id(&c, profile).to_string(), reference_id(&c, profile));
            }
            Ok(())
        })
        .unwrap();
}

/// The hash on its own, against `sha2`, on generated inputs of every length
/// around the block boundaries.
#[test]
fn sha256_agrees_with_sha2() {
    support::runner(5, 512)
        .run(&prop::collection::vec(any::<u8>(), 0..300), |data| {
            let ours = nomos_canon::sha256::digest(&data);
            let theirs: [u8; 32] = sha2::Sha256::digest(&data).into();
            prop_assert_eq!(ours, theirs);
            Ok(())
        })
        .unwrap();
}

/// No input panics a decoder: arbitrary bytes, and arbitrary bytes after a
/// valid prefix, into both profiles, the executable decoder, and archival
/// inspection.
#[test]
fn no_input_panics_the_decoders() {
    let cases = std::cell::Cell::new(0u32);
    support::runner(6, 2048)
        .run(
            &(
                prop::collection::vec(any::<u8>(), 0..200),
                valid_raw(),
                0usize..64,
            ),
            |(noise, raw, cut)| {
                let c = canon(raw);
                for profile in Profile::ALL {
                    let mut prefixed = encode(&c, profile);
                    prefixed.truncate(cut.min(prefixed.len()));
                    prefixed.extend_from_slice(&noise);
                    for input in [&noise, &prefixed] {
                        let _ = decode(input, profile, &Reader::current());
                        let _ = nomos_canon::artifact::inspect(input, profile);
                        cases.set(cases.get() + 1);
                    }
                }
                Ok(())
            },
        )
        .unwrap();
    println!("decoder inputs without a panic: {}", cases.get());
}

// ---------------------------------------------------------------------------
// Negative controls

/// The canon-ir.md equivalence, checked on pairs a naive normalization
/// would collapse: each pair differs in meaning, so each keeps two
/// identities in both profiles.
#[test]
fn semantically_different_canons_keep_distinct_identities() {
    let base = |r: nomos_canon::model::RawResource, rels: Vec<nomos_canon::model::RawRelation>| {
        let mut resources = vec![
            resource("/etc/a", spec("file", "absent", None), &[], &[]),
            r,
        ];
        resources.dedup_by(|a, b| a.path == b.path);
        RawCanon {
            name: "pair".into(),
            resources,
            relations: rels,
        }
    };
    let file =
        |state: &str, d: Option<String>| resource("/etc/b", spec("file", state, d), &[], &[]);
    let pairs: Vec<(&str, RawCanon, RawCanon)> = vec![
        (
            "absent vs present-any",
            base(file("absent", None), vec![]),
            base(file("present-any", None), vec![]),
        ),
        (
            "present-any vs present-exact",
            base(file("present-any", None), vec![]),
            base(file("present-exact", Some(hex_digest(0))), vec![]),
        ),
        (
            "running vs loaded",
            base(
                resource("/etc/b", spec("service", "running", None), &[], &[]),
                vec![],
            ),
            base(
                resource(
                    "/etc/b",
                    spec("service", "loaded", Some(hex_digest(0))),
                    &[],
                    &[],
                ),
                vec![],
            ),
        ),
        (
            "requires vs after",
            base(
                file("absent", None),
                vec![relation("/etc/a", "requires", "/etc/b")],
            ),
            base(
                file("absent", None),
                vec![relation("/etc/a", "after", "/etc/b")],
            ),
        ),
        (
            "after vs on_change",
            base(
                file("absent", None),
                vec![relation("/etc/a", "after", "/etc/b")],
            ),
            base(
                file("absent", None),
                vec![relation("/etc/a", "on_change", "/etc/b")],
            ),
        ),
        (
            "direction of a relation",
            base(
                file("absent", None),
                vec![relation("/etc/a", "requires", "/etc/b")],
            ),
            base(
                file("absent", None),
                vec![relation("/etc/b", "requires", "/etc/a")],
            ),
        ),
        (
            "no key vs one key",
            base(file("absent", None), vec![]),
            base(
                resource("/etc/b", spec("file", "absent", None), &["k"], &[]),
                vec![],
            ),
        ),
        (
            "a key vs the same text as a node",
            base(
                resource("/etc/b", spec("file", "absent", None), &["k"], &[]),
                vec![],
            ),
            base(
                resource("/etc/b", spec("file", "absent", None), &[], &["k"]),
                vec![],
            ),
        ),
        (
            "path case",
            base(file("absent", None), vec![]),
            base(
                resource("/etc/B", spec("file", "absent", None), &[], &[]),
                vec![],
            ),
        ),
        (
            "composed vs decomposed accent",
            base(
                resource("/srv/caf\u{e9}", spec("file", "absent", None), &[], &[]),
                vec![],
            ),
            base(
                resource("/srv/cafe\u{301}", spec("file", "absent", None), &[], &[]),
                vec![],
            ),
        ),
    ];
    for (what, x, y) in pairs {
        let (a, b) = (canon(x), canon(y));
        assert_ne!(a, b, "{what}");
        for profile in Profile::ALL {
            assert_ne!(
                encode(&a, profile),
                encode(&b, profile),
                "{what} {profile:?}"
            );
            assert_ne!(
                canon_id(&a, profile),
                canon_id(&b, profile),
                "{what} {profile:?}"
            );
        }
    }
}

/// The two profiles never share an identity for one Canon, and the
/// separation is the domain tag's, not an accident of the encodings: each
/// identity is the digest of its own profile's tagged preimage and of
/// neither the untagged one nor the other profile's tag over the same bytes.
/// (A first version compared the two identities only, and survived
/// `SM-CANON-006`: the encodings differ, so the identities differ untagged.)
#[test]
fn the_profiles_have_separate_identities() {
    let preimage = |tag: &[u8], bytes: &[u8]| {
        let mut h = sha2::Sha256::new();
        h.update(tag);
        h.update([0u8]);
        h.update(2u64.to_be_bytes());
        h.update(bytes);
        format!("sha256:{}", hex(&h.finalize()))
    };
    for (_, raw) in golden() {
        let c = canon(raw);
        let ids: Vec<CanonId> = Profile::ALL.iter().map(|p| canon_id(&c, *p)).collect();
        assert_ne!(ids[0], ids[1]);
        for (profile, other) in [(Profile::Cbor, Profile::Jcs), (Profile::Jcs, Profile::Cbor)] {
            let bytes = encode(&c, profile);
            let id = canon_id(&c, profile).to_string();
            assert_eq!(id, reference_id(&c, profile));
            assert_ne!(id, preimage(b"", &bytes), "{profile:?}: the tag is missing");
            let other_tag: &[u8] = match other {
                Profile::Cbor => b"nomos.canon-id.cbor",
                Profile::Jcs => b"nomos.canon-id.jcs",
            };
            assert_ne!(
                id,
                preimage(other_tag, &bytes),
                "{profile:?}: the other profile's tag"
            );
        }
    }
}

/// Adversarial encodings: each is the golden telemetry Canon written in a
/// way the profile's rules forbid, and each is rejected at the stage that
/// owns the rule.
#[test]
fn adversarial_cbor_is_rejected() {
    let c = canon(golden().into_iter().nth(1).unwrap().1);
    let good = encode(&c, Profile::Cbor);
    let value = raw_value(&c.to_raw());
    let reject = |bytes: Vec<u8>, expected: DecodeError, what: &str| {
        assert_eq!(
            decode(&bytes, Profile::Cbor, &Reader::current()).err(),
            Some(expected),
            "{what}"
        );
    };
    // The schema version, 2, written as a one-byte argument: 0x18 0x02.
    let at = good.windows(7).position(|w| w == b"fschema").unwrap() + 7;
    let mut long_int = good.clone();
    long_int.splice(at..at + 1, [0x18, 0x02]);
    reject(long_int, DecodeError::NonCanonical, "non-shortest integer");
    let mut trailing = good.clone();
    trailing.push(0x00);
    reject(
        trailing,
        DecodeError::Syntax(SyntaxError::TrailingBytes),
        "trailing byte",
    );
    let mut float = good.clone();
    float.splice(at..at + 1, [0xf9, 0x40, 0x00]);
    reject(
        float,
        DecodeError::Syntax(SyntaxError::OutsideModel),
        "a float for the schema",
    );
    let mut tagged = good.clone();
    tagged.insert(0, 0xc0);
    reject(
        tagged,
        DecodeError::Syntax(SyntaxError::OutsideModel),
        "a tag",
    );
    // The top-level map in insertion order rather than sorted by encoded key.
    let Value::Map(entries) = &value else {
        panic!()
    };
    let mut unsorted = vec![0xa4u8];
    for (k, v) in entries {
        unsorted.extend(nomos_canon::cbor::encode(&Value::Text(k.clone())));
        unsorted.extend(nomos_canon::cbor::encode(v));
    }
    reject(unsorted, DecodeError::NonCanonical, "unsorted map");
    // The same map with its first entry written twice, and its header saying five.
    let mut duplicate = vec![0xa5u8];
    let first = &entries[0];
    for (k, v) in std::iter::once(first).chain(entries.iter()) {
        duplicate.extend(nomos_canon::cbor::encode(&Value::Text(k.clone())));
        duplicate.extend(nomos_canon::cbor::encode(v));
    }
    reject(
        duplicate,
        DecodeError::Syntax(SyntaxError::DuplicateKey),
        "duplicate key",
    );
    // Resources written in reverse: a valid value, canonical as a value, not
    // the canonical encoding of the Canon.
    let mut raw = c.to_raw();
    raw.resources.reverse();
    reject(
        nomos_canon::cbor::encode(&raw_value(&raw)),
        DecodeError::NonCanonical,
        "resources out of order",
    );
    let mut raw = c.to_raw();
    // Resource 2 is /etc/nomos/cell.conf, the one with a key and a digest.
    let key = raw.resources[2].keys[0].clone();
    raw.resources[2].keys.push(key);
    reject(
        nomos_canon::cbor::encode(&raw_value(&raw)),
        DecodeError::NonCanonical,
        "a key twice",
    );
    let mut raw = c.to_raw();
    raw.resources[2].spec.digest = raw.resources[2]
        .spec
        .digest
        .clone()
        .map(|d| d.to_uppercase());
    reject(
        nomos_canon::cbor::encode(&raw_value(&raw)),
        DecodeError::Invalid(nomos_canon::model::CanonError::InvalidDigest { resource: 2 }),
        "uppercase digest",
    );
}

#[test]
fn adversarial_json_is_rejected() {
    let c = canon(golden().into_iter().nth(1).unwrap().1);
    let good = String::from_utf8(encode(&c, Profile::Jcs)).unwrap();
    let reject = |text: String, expected: DecodeError, what: &str| {
        assert_eq!(
            decode(text.as_bytes(), Profile::Jcs, &Reader::current()).err(),
            Some(expected),
            "{what}"
        );
    };
    reject(
        format!(" {good}"),
        DecodeError::NonCanonical,
        "leading whitespace",
    );
    reject(
        good.replacen(",", ", ", 1),
        DecodeError::NonCanonical,
        "whitespace after a comma",
    );
    reject(
        good.replacen("\"schema\":2", "\"schema\":2.0", 1),
        DecodeError::Syntax(SyntaxError::OutsideModel),
        "2.0",
    );
    reject(
        good.replacen("\"schema\":2", "\"schema\":2e0", 1),
        DecodeError::Syntax(SyntaxError::OutsideModel),
        "2e0",
    );
    reject(
        good.replacen("\"schema\":2", "\"schema\":02", 1),
        DecodeError::Syntax(SyntaxError::Malformed),
        "02",
    );
    reject(
        good.replacen("\"schema\":2", "\"schema\":true", 1),
        DecodeError::Syntax(SyntaxError::OutsideModel),
        "true",
    );
    reject(
        good.replacen("\"name\"", "\"n\\u0061me\"", 1),
        DecodeError::NonCanonical,
        "an optional escape",
    );
    reject(
        format!("\u{feff}{good}"),
        DecodeError::Syntax(SyntaxError::Malformed),
        "byte order mark",
    );
    reject(
        good.replacen("{\"name\"", "{\"schema\":2,\"name\"", 1),
        DecodeError::Syntax(SyntaxError::DuplicateKey),
        "duplicate member",
    );
    // The top-level members in insertion order, through serde_json's
    // order-preserving map.
    let value = raw_value(&c.to_raw());
    let unsorted = serde_json::to_string(&to_json(&value)).unwrap();
    assert_ne!(unsorted, good);
    reject(unsorted, DecodeError::NonCanonical, "unsorted members");
    reject(
        good.to_uppercase(),
        DecodeError::MissingField(nomos_canon::artifact::Place {
            what: "canon",
            index: None,
        }),
        "uppercase everything",
    );
}

// ---------------------------------------------------------------------------
// Measurements

/// Size and cost per profile over the golden corpus, and the size of each
/// codec. Recorded in the result record; nothing is a threshold.
#[test]
fn measurements() {
    let corpus: Vec<Canon> = golden().into_iter().map(|(_, r)| canon(r)).collect();
    for profile in Profile::ALL {
        let bytes: usize = corpus.iter().map(|c| encode(c, profile).len()).sum();
        let rounds = 2000;
        let start = Instant::now();
        for _ in 0..rounds {
            for c in &corpus {
                std::hint::black_box(encode(c, profile));
            }
        }
        let enc = start.elapsed().as_nanos() / (rounds * corpus.len()) as u128;
        let encoded: Vec<Vec<u8>> = corpus.iter().map(|c| encode(c, profile)).collect();
        let start = Instant::now();
        for _ in 0..rounds {
            for b in &encoded {
                std::hint::black_box(decode(b, profile, &Reader::current()).unwrap());
            }
        }
        let dec = start.elapsed().as_nanos() / (rounds * corpus.len()) as u128;
        let build = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        println!(
            "{profile:?}: corpus {bytes} bytes; encode {enc} ns, decode {dec} ns per Canon ({build} build)"
        );
    }
}

// ---------------------------------------------------------------------------
// Limits

/// Each limit of canon-ir.md's data model, at its boundary, in both
/// profiles: the largest value inside decodes, the smallest outside is
/// `LimitExceeded`.
#[test]
fn limits_hold_at_their_boundaries() {
    use nomos_canon::value::{MAX_ARRAY, MAX_ARTIFACT, MAX_DEPTH, MAX_MAP, MAX_TEXT, MAX_UINT};
    // The limits as canon-ir.md states them, not as the crate defines them.
    assert_eq!(MAX_UINT, (1u64 << 53) - 1);
    assert_eq!(MAX_TEXT, 4096);
    assert_eq!(MAX_ARRAY, 65_536);
    assert_eq!(MAX_MAP, 64);
    assert_eq!(MAX_DEPTH, 8);
    assert_eq!(MAX_ARTIFACT, 16 * 1024 * 1024);
    let nested = |levels: usize| {
        let mut v = Value::Uint(0);
        for _ in 0..levels {
            v = Value::Array(vec![v]);
        }
        v
    };
    let nested_maps = |levels: usize| {
        let mut v = Value::Uint(0);
        for _ in 0..levels {
            v = Value::Map(vec![("m".to_string(), v)]);
        }
        v
    };
    let map = |n: usize| {
        Value::Map(
            (0..n)
                .map(|i| (format!("k{i:03}"), Value::Uint(0)))
                .collect(),
        )
    };
    let cases: Vec<(&str, Value, Value)> = vec![
        ("integer", Value::Uint(MAX_UINT), Value::Uint(MAX_UINT + 1)),
        (
            "text",
            Value::Text("a".repeat(MAX_TEXT)),
            Value::Text("a".repeat(MAX_TEXT + 1)),
        ),
        // Text whose JSON form is all escapes, so the parser's check inside
        // the escape loop is the one that decides.
        (
            "escaped text",
            Value::Text("\n".repeat(MAX_TEXT)),
            Value::Text("\n".repeat(MAX_TEXT + 1)),
        ),
        (
            "array",
            Value::Array(vec![Value::Uint(0); MAX_ARRAY]),
            Value::Array(vec![Value::Uint(0); MAX_ARRAY + 1]),
        ),
        ("map", map(MAX_MAP), map(MAX_MAP + 1)),
        ("depth", nested(MAX_DEPTH), nested(MAX_DEPTH + 1)),
        (
            "map depth",
            nested_maps(MAX_DEPTH),
            nested_maps(MAX_DEPTH + 1),
        ),
    ];
    for profile in Profile::ALL {
        for (what, inside, outside) in &cases {
            let bytes = profile.encode_value(inside);
            assert_eq!(
                profile.decode_value(&bytes).as_ref(),
                Ok(inside),
                "{what} {profile:?}"
            );
            let bytes = profile.encode_value(outside);
            assert_eq!(
                profile.decode_value(&bytes),
                Err(SyntaxError::LimitExceeded),
                "{what} {profile:?}"
            );
        }
        // An artifact one byte over the size limit is refused before parsing;
        // one at the limit is parsed, and fails for what it holds.
        let big = vec![b' '; MAX_ARTIFACT + 1];
        assert_eq!(
            profile.decode_value(&big),
            Err(SyntaxError::LimitExceeded),
            "{profile:?}"
        );
        let at_limit = vec![b' '; MAX_ARTIFACT];
        assert_ne!(
            profile.decode_value(&at_limit),
            Err(SyntaxError::LimitExceeded),
            "{profile:?}"
        );
    }
}
