//! Experiment `compatibility-matrix` (grounding plan, `06-canon-artifact`):
//! do explicit schema versions and migrations preserve accepted semantics
//! across old and new readers?
//!
//! Committed fixtures, schemas 1, 2, and 3, some using what an older
//! reader does not know, are read by each reader. Milestone
//! `08-resource-families` added schema 3, its fixtures, and the readers of
//! `06` as old readers; the cells of `06` are unchanged but for the current
//! reader, which now migrates schema 2 too. Every cell of the matrix
//! is stated here, before the run, from canon-ir.md; the test compares.
//! The fixtures under `tests/fixtures/canon/compat/` are written with
//! `NOMOS_CANON_BLESS=1` from the values below; the golden files beside them
//! are the independent encoders' output (`tests/encoding.rs`).

mod support;

use std::fs;
use std::path::PathBuf;

use nomos_canon::artifact::{
    DecodeError, Decoded, Place, Profile, Reader, canon_id, decode, encode, inspect,
};
use nomos_canon::model::{Canon, CanonError, Kind};
use nomos_canon::value::Value;
use support::*;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/canon")
}

fn ext(profile: Profile) -> &'static str {
    match profile {
        Profile::Cbor => "cbor",
        Profile::Jcs => "json",
    }
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

fn golden_canon(name: &str) -> Canon {
    if let Some((_, raw)) = golden().into_iter().find(|(n, _)| *n == name) {
        return Canon::try_from(raw).unwrap();
    }
    let raw = golden_v3().into_iter().find(|(n, _)| *n == name).unwrap().1;
    Canon::try_from(raw).unwrap()
}

/// A schema-3 resource value with the given kind, name, and spec, and any
/// extra fields.
fn resource_value3(kind: &str, name: &str, spec: Value, extra: &[(&str, Value)]) -> Value {
    let mut entries = vec![
        ("kind", text(kind)),
        ("name", text(name)),
        ("spec", spec),
        ("keys", Value::Array(vec![])),
        ("disrupts", Value::Array(vec![])),
    ];
    entries.extend(extra.iter().cloned());
    map(&entries)
}

/// A schema-2 resource value with the given spec and any extra fields.
fn resource_value(path: &str, spec: Value, extra: &[(&str, Value)]) -> Value {
    let mut entries = vec![
        ("path", text(path)),
        ("spec", spec),
        ("keys", Value::Array(vec![])),
        ("disrupts", Value::Array(vec![])),
    ];
    entries.extend(extra.iter().cloned());
    map(&entries)
}

fn canon_value(schema: u64, resources: Vec<Value>) -> Value {
    map(&[
        ("schema", Value::Uint(schema)),
        ("name", text("compat")),
        ("resources", Value::Array(resources)),
        ("relations", Value::Array(vec![])),
    ])
}

/// The artifacts only this experiment has: each is what a newer writer, or
/// a broken one, could produce.
fn compat_values() -> Vec<(&'static str, Value)> {
    let file = |state: &str| map(&[("kind", text("file")), ("state", text(state))]);
    vec![
        (
            // A mutating kind no reader here knows.
            "unknown-kind",
            canon_value(
                2,
                vec![
                    resource_value("/etc/a", file("present-any"), &[]),
                    resource_value(
                        "/proc/sys/net/ipv4/ip_forward",
                        map(&[("kind", text("sysctl")), ("state", text("value"))]),
                        &[],
                    ),
                ],
            ),
        ),
        (
            // A field schema 2 does not have, on a resource.
            "unknown-field",
            canon_value(
                2,
                vec![resource_value(
                    "/etc/a",
                    file("present-any"),
                    &[("mode", Value::Uint(0o600))],
                )],
            ),
        ),
        (
            // A state of a known kind that schema 2 does not have.
            "unknown-state",
            canon_value(
                2,
                vec![resource_value("/etc/a", file("present-sparse"), &[])],
            ),
        ),
        (
            // Schema 3 by its number, shaped like schema 2: written by `06`
            // as a schema from the future, now a malformed schema 3.
            "schema-3",
            canon_value(3, vec![resource_value("/etc/a", file("present-any"), &[])]),
        ),
        (
            // A schema from the future, otherwise shaped like schema 3.
            "schema-4",
            canon_value(
                4,
                vec![resource_value3(
                    "file",
                    "/etc/a",
                    map(&[("state", text("present")), ("content", text("any"))]),
                    &[],
                )],
            ),
        ),
        (
            // A family no reader here knows, in schema 3.
            "v3-unknown-kind",
            canon_value(
                3,
                vec![
                    resource_value3("file", "/etc/a", map(&[("state", text("absent"))]), &[]),
                    resource_value3("firewall", "input", map(&[("policy", text("drop"))]), &[]),
                ],
            ),
        ),
        (
            // A spec field schema 3 does not define for the family.
            "v3-unknown-spec-field",
            canon_value(
                3,
                vec![resource_value3(
                    "file",
                    "/etc/a",
                    map(&[
                        ("state", text("present")),
                        ("content", text("any")),
                        ("immutable", text("true")),
                    ]),
                    &[],
                )],
            ),
        ),
        (
            // A spec value that is not text.
            "v3-wrong-type",
            canon_value(
                3,
                vec![resource_value3(
                    "sysctl",
                    "net.ipv4.ip_forward",
                    map(&[("value", Value::Uint(1))]),
                    &[],
                )],
            ),
        ),
        (
            // Schema 1 with a schema-2 field in a file.
            "v1-with-keys",
            map(&[
                ("schema", Value::Uint(1)),
                ("name", text("compat")),
                (
                    "files",
                    Value::Array(vec![map(&[
                        ("path", text("/etc/a")),
                        ("state", text("absent")),
                        ("keys", Value::Array(vec![])),
                    ])]),
                ),
                ("relations", Value::Array(vec![])),
            ]),
        ),
    ]
}

fn fixture(name: &str, profile: Profile) -> Vec<u8> {
    let dir = if compat_values().iter().any(|(n, _)| *n == name) {
        "compat"
    } else {
        "golden"
    };
    let file = root().join(dir).join(format!("{name}.{}", ext(profile)));
    fs::read(&file).unwrap_or_else(|_| panic!("missing {}", file.display()))
}

#[test]
fn compat_fixtures_are_the_committed_bytes() {
    let bless = std::env::var_os("NOMOS_CANON_BLESS").is_some();
    for (name, value) in compat_values() {
        for profile in Profile::ALL {
            let bytes = profile.encode_value(&value);
            let file = root()
                .join("compat")
                .join(format!("{name}.{}", ext(profile)));
            if bless {
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(&file, &bytes).unwrap();
            }
            assert_eq!(fs::read(&file).unwrap(), bytes, "{}", file.display());
        }
    }
}

// ---------------------------------------------------------------------------
// The matrix

/// What a reader must do with an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cell {
    /// Accept it as this golden Canon, read in this schema.
    Accept(&'static str, u64),
    /// Refuse it with this error.
    Refuse(DecodeError),
}

use Cell::{Accept, Refuse};

fn file_only_reader() -> Reader {
    Reader::new(&[1, 2], &[Kind::File])
}

/// The readers, in the order of the matrix's columns: schema 1 only; the
/// current reader of `06`, schemas 1 and 2 with files and services; the
/// current reader; a schema-1-and-2 reader that does not know services; and
/// a host reader, every schema and every kind but the legacy `service`.
fn readers() -> [Reader; 5] {
    [
        Reader::v1(),
        Reader::v2(),
        Reader::current(),
        file_only_reader(),
        Reader::host(),
    ]
}

/// The matrix, stated in full: artifact, then the cell for each reader of
/// [`readers`].
fn matrix() -> Vec<(&'static str, [Cell; 5])> {
    let schema = || Refuse(DecodeError::UnsupportedSchema);
    let kind = |resource| Refuse(DecodeError::UnsupportedKind { resource });
    let unknown = |what, index| {
        Refuse(DecodeError::UnknownField(Place {
            what,
            index: Some(index),
        }))
    };
    let all = |name, schema| {
        [
            Accept(name, schema),
            Accept(name, schema),
            Accept(name, schema),
            Accept(name, schema),
            Accept(name, schema),
        ]
    };
    let v2 = |name| {
        [
            schema(),
            Accept(name, 2),
            Accept(name, 2),
            Accept(name, 2),
            Accept(name, 2),
        ]
    };
    let v3 = |name| {
        [
            schema(),
            schema(),
            Accept(name, 3),
            schema(),
            Accept(name, 3),
        ]
    };
    vec![
        ("empty.v1", all("empty", 1)),
        ("text.v1", all("text", 1)),
        ("legacy.v1", all("legacy", 1)),
        ("empty.v2", v2("empty")),
        ("text.v2", v2("text")),
        ("legacy.v2", v2("legacy")),
        (
            "telemetry.v2",
            [
                schema(),
                Accept("telemetry", 2),
                Accept("telemetry", 2),
                kind(3),
                kind(3),
            ],
        ),
        (
            "loaded.v2",
            [
                schema(),
                Accept("loaded", 2),
                Accept("loaded", 2),
                kind(1),
                kind(1),
            ],
        ),
        (
            "unknown-kind",
            [schema(), kind(1), kind(1), kind(1), kind(1)],
        ),
        (
            "unknown-field",
            [
                schema(),
                unknown("resource", 0),
                unknown("resource", 0),
                unknown("resource", 0),
                unknown("resource", 0),
            ],
        ),
        (
            "unknown-state",
            [
                schema(),
                Refuse(DecodeError::Invalid(CanonError::UnknownRequirement {
                    resource: 0,
                })),
                Refuse(DecodeError::Invalid(CanonError::UnknownRequirement {
                    resource: 0,
                })),
                Refuse(DecodeError::Invalid(CanonError::UnknownRequirement {
                    resource: 0,
                })),
                Refuse(DecodeError::Invalid(CanonError::UnknownRequirement {
                    resource: 0,
                })),
            ],
        ),
        (
            "schema-3",
            [
                schema(),
                schema(),
                unknown("resource", 0),
                schema(),
                unknown("resource", 0),
            ],
        ),
        (
            "schema-4",
            [schema(), schema(), schema(), schema(), schema()],
        ),
        (
            "v1-with-keys",
            [
                unknown("file", 0),
                unknown("file", 0),
                unknown("file", 0),
                unknown("file", 0),
                unknown("file", 0),
            ],
        ),
        ("empty.v3", v3("empty")),
        ("text.v3", v3("text")),
        ("legacy.v3", v3("legacy")),
        ("text3.v3", v3("text3")),
        (
            "telemetry.v3",
            [
                schema(),
                schema(),
                Accept("telemetry", 3),
                schema(),
                kind(3),
            ],
        ),
        (
            "loaded.v3",
            [schema(), schema(), Accept("loaded", 3), schema(), kind(1)],
        ),
        (
            "families.v3",
            [schema(), schema(), Accept("families", 3), schema(), kind(4)],
        ),
        (
            "v3-unknown-kind",
            [schema(), schema(), kind(1), schema(), kind(1)],
        ),
        (
            "v3-unknown-spec-field",
            [
                schema(),
                schema(),
                unknown("spec", 0),
                schema(),
                unknown("spec", 0),
            ],
        ),
        (
            "v3-wrong-type",
            [
                schema(),
                schema(),
                Refuse(DecodeError::WrongType(Place {
                    what: "spec",
                    index: Some(0),
                })),
                schema(),
                Refuse(DecodeError::WrongType(Place {
                    what: "spec",
                    index: Some(0),
                })),
            ],
        ),
    ]
}

/// Every cell, in both profiles. An accepted artifact is the golden Canon
/// it was written from; a migrated one carries its lineage, whose digest is
/// of the bytes read and whose identity is the current schema's.
#[test]
fn every_reader_does_what_the_matrix_says() {
    let readers = readers();
    let (mut accepted, mut refused) = (0, 0);
    for (artifact, cells) in matrix() {
        for profile in Profile::ALL {
            let bytes = fixture(artifact, profile);
            for (reader, cell) in readers.iter().zip(&cells) {
                let got = decode(&bytes, profile, reader);
                match cell {
                    Accept(name, schema) => {
                        let Decoded {
                            canon,
                            schema: read,
                            lineage,
                        } = got.unwrap_or_else(|e| panic!("{artifact} {profile:?}: {e:?}"));
                        let want = golden_canon(name);
                        assert_eq!(canon, want, "{artifact} {profile:?}: changed intent");
                        assert_eq!(read, *schema);
                        match lineage {
                            None => assert_eq!(*schema, 3),
                            Some(l) => {
                                assert!(*schema < 3);
                                assert_eq!(l.from_schema, *schema);
                                assert_eq!(l.from_digest, nomos_canon::sha256::digest(&bytes));
                                assert_eq!(l.to, canon_id(&want, profile));
                                assert_eq!(
                                    encode(&canon, profile),
                                    fixture(&format!("{name}.v3"), profile)
                                );
                            }
                        }
                        accepted += 1;
                    }
                    Refuse(e) => {
                        assert_eq!(got.err().as_ref(), Some(e), "{artifact} {profile:?}");
                        refused += 1;
                    }
                }
            }
        }
    }
    println!(
        "matrix cells: {accepted} accepted as their golden Canon, {refused} refused as stated"
    );
}

/// A Canon says which kinds it needs, which is what a reader's kinds are
/// compared against.
#[test]
fn a_canon_names_the_kinds_it_needs() {
    assert!(golden_canon("empty").kinds().is_empty());
    assert_eq!(
        golden_canon("legacy")
            .kinds()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![Kind::File]
    );
    assert_eq!(
        golden_canon("telemetry")
            .kinds()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![Kind::File, Kind::Service]
    );
    assert_eq!(
        golden_canon("families").kinds(),
        Kind::ALL.into_iter().collect()
    );
}

/// Archival inspection reads every artifact, of any schema and any kind,
/// and reports its schema, name, and digest; it never yields a Canon.
#[test]
fn archival_inspection_reads_what_execution_refuses() {
    for (artifact, _) in matrix() {
        for profile in Profile::ALL {
            let bytes = fixture(artifact, profile);
            let i = inspect(&bytes, profile).unwrap_or_else(|e| panic!("{artifact}: {e:?}"));
            let want = match artifact {
                "schema-4" => 4,
                "schema-3" => 3,
                a if a.ends_with(".v3") || a.starts_with("v3-") => 3,
                a if a.ends_with(".v1") || a == "v1-with-keys" => 1,
                _ => 2,
            };
            assert_eq!(i.schema, want, "{artifact}");
            assert_eq!(i.digest, nomos_canon::sha256::digest(&bytes));
            assert!(i.name.is_some());
        }
    }
}

/// A migrated artifact and its current-schema form are one Canon with one
/// identity, and the migration adds nothing: no conflict key, no node, no
/// relation the original did not have.
#[test]
fn migration_preserves_meaning_and_identity() {
    for name in ["empty", "text", "legacy"] {
        for profile in Profile::ALL {
            let old = decode(
                &fixture(&format!("{name}.v1"), profile),
                profile,
                &Reader::current(),
            )
            .unwrap();
            let new = decode(
                &fixture(&format!("{name}.v2"), profile),
                profile,
                &Reader::current(),
            )
            .unwrap();
            let current = decode(
                &fixture(&format!("{name}.v3"), profile),
                profile,
                &Reader::current(),
            )
            .unwrap();
            assert_eq!(old.canon, new.canon);
            assert_eq!(new.canon, current.canon);
            assert!(current.lineage.is_none());
            assert_eq!(old.lineage.unwrap().to, canon_id(&current.canon, profile));
            assert_eq!(new.lineage.unwrap().to, canon_id(&current.canon, profile));
            for r in old.canon.resources().values() {
                assert!(r.keys().is_empty() && r.disrupts().is_empty(), "{name}");
            }
            let Value::Map(top) = profile
                .decode_value(&fixture(&format!("{name}.v1"), profile))
                .unwrap()
            else {
                panic!()
            };
            let Some((_, Value::Array(rels))) = top.iter().find(|(k, _)| k == "relations") else {
                panic!()
            };
            assert_eq!(old.canon.relations().len(), rels.len(), "{name}");
        }
    }
}

// ---------------------------------------------------------------------------
// The negative control

/// Whether `canon` says everything the artifact says: every resource the
/// artifact lists is in the Canon with the kind and state written. The
/// check reads the artifact as a value, not through the decoder.
fn silent_change(bytes: &[u8], profile: Profile, canon: &Canon) -> bool {
    let value = profile.decode_value(bytes).unwrap();
    let list = value.get("resources").or_else(|| value.get("files"));
    let Some(Value::Array(items)) = list else {
        return true;
    };
    let written = canon.to_raw();
    let path_kinds = ["file", "directory", "service"];
    items.len() != written.resources.len()
        || items.iter().any(|item| {
            // Schemas 1 and 2 name a resource by path; schema 3 by kind and
            // name.
            match (item.get("path"), item.get("kind"), item.get("name")) {
                (Some(Value::Text(path)), _, _) => !written
                    .resources
                    .iter()
                    .any(|r| &r.name == path && path_kinds.contains(&r.kind.as_str())),
                (None, Some(Value::Text(kind)), Some(Value::Text(name))) => !written
                    .resources
                    .iter()
                    .any(|r| &r.kind == kind && &r.name == name),
                _ => true,
            }
        })
}

/// A reader that accepts an unknown kind "for forward compatibility" by
/// dropping the resources that use it, then decodes what is left. This is
/// the rival.
fn lenient(bytes: &[u8], profile: Profile) -> Result<Decoded, DecodeError> {
    let Value::Map(mut top) = profile.decode_value(bytes).map_err(DecodeError::Syntax)? else {
        return Err(DecodeError::WrongType(Place {
            what: "canon",
            index: None,
        }));
    };
    for (k, v) in &mut top {
        if k == "resources"
            && let Value::Array(items) = v
        {
            items.retain(|r| {
                matches!(
                    r.get("spec").and_then(|s| s.get("kind")),
                    Some(Value::Text(k)) if k == "file" || k == "service"
                )
            });
        }
    }
    let pruned = profile.encode_value(&Value::Map(top));
    decode(&pruned, profile, &Reader::current())
}

/// The lenient reader changes intent on the unknown-kind artifact, and the
/// check sees it; the strict reader refuses the artifact, and on every
/// artifact it accepts the check sees no change.
#[test]
fn the_harness_catches_a_reader_that_skips_unknown_kinds() {
    for profile in Profile::ALL {
        let bytes = fixture("unknown-kind", profile);
        let d = lenient(&bytes, profile).expect("the rival accepts the artifact");
        assert!(
            silent_change(&bytes, profile, &d.canon),
            "the negative control did not fire"
        );
        assert!(decode(&bytes, profile, &Reader::current()).is_err());
        for (artifact, _) in matrix() {
            let bytes = fixture(artifact, profile);
            if let Ok(d) = decode(&bytes, profile, &Reader::current()) {
                assert!(!silent_change(&bytes, profile, &d.canon), "{artifact}");
            }
        }
    }
}

/// Generated Canons, written in schemas 1 and 2 where they can say them: the
/// migration and the direct reading agree, so no generated case changes
/// intent across the versions.
#[test]
fn generated_migrations_agree_with_direct_reading() {
    let migrated = std::cell::Cell::new(0u32);
    support::runner(21, 1024)
        .run(&valid_raw(), |raw| {
            let c = Canon::try_from(raw).unwrap();
            for profile in Profile::ALL {
                if let Some(v2) = nomos_canon::artifact::encode_v2(&c, profile) {
                    let d = decode(&v2, profile, &Reader::current()).unwrap();
                    proptest::prop_assert_eq!(&d.canon, &c);
                    proptest::prop_assert_eq!(d.lineage.unwrap().to, canon_id(&c, profile));
                    proptest::prop_assert!(!silent_change(&v2, profile, &d.canon));
                }
                if let Some(v1) = nomos_canon::artifact::encode_v1(&c, profile) {
                    let d = decode(&v1, profile, &Reader::current()).unwrap();
                    proptest::prop_assert_eq!(&d.canon, &c);
                    proptest::prop_assert_eq!(d.lineage.unwrap().to, canon_id(&c, profile));
                    proptest::prop_assert!(!silent_change(&v1, profile, &d.canon));
                    migrated.set(migrated.get() + 1);
                }
            }
            Ok(())
        })
        .unwrap();
    println!("generated schema-1 artifacts migrated: {}", migrated.get());
}
