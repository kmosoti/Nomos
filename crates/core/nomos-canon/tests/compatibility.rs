//! Experiment `compatibility-matrix` (grounding plan, `06-canon-artifact`):
//! do explicit schema versions and migrations preserve accepted semantics
//! across old and new readers?
//!
//! Committed fixtures, schema 1 and schema 2, some using what an older
//! reader does not know, are read by each reader. Every cell of the matrix
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
    let raw = golden().into_iter().find(|(n, _)| *n == name).unwrap().1;
    Canon::try_from(raw).unwrap()
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
            // A schema from the future, otherwise shaped like schema 2.
            "schema-3",
            canon_value(3, vec![resource_value("/etc/a", file("present-any"), &[])]),
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

/// The matrix, stated in full: artifact, then the cell for the schema-1
/// reader, the current reader, and a current-schema reader that does not
/// know services.
fn matrix() -> Vec<(&'static str, [Cell; 3])> {
    let schema = || Refuse(DecodeError::UnsupportedSchema);
    vec![
        (
            "empty.v1",
            [Accept("empty", 1), Accept("empty", 1), Accept("empty", 1)],
        ),
        (
            "text.v1",
            [Accept("text", 1), Accept("text", 1), Accept("text", 1)],
        ),
        (
            "legacy.v1",
            [
                Accept("legacy", 1),
                Accept("legacy", 1),
                Accept("legacy", 1),
            ],
        ),
        (
            "empty.v2",
            [schema(), Accept("empty", 2), Accept("empty", 2)],
        ),
        ("text.v2", [schema(), Accept("text", 2), Accept("text", 2)]),
        (
            "legacy.v2",
            [schema(), Accept("legacy", 2), Accept("legacy", 2)],
        ),
        (
            "telemetry.v2",
            [
                schema(),
                Accept("telemetry", 2),
                Refuse(DecodeError::UnsupportedKind { resource: 3 }),
            ],
        ),
        (
            "loaded.v2",
            [
                schema(),
                Accept("loaded", 2),
                Refuse(DecodeError::UnsupportedKind { resource: 1 }),
            ],
        ),
        (
            "unknown-kind",
            [
                schema(),
                Refuse(DecodeError::UnsupportedKind { resource: 1 }),
                Refuse(DecodeError::UnsupportedKind { resource: 1 }),
            ],
        ),
        (
            "unknown-field",
            [
                schema(),
                Refuse(DecodeError::UnknownField(Place {
                    what: "resource",
                    index: Some(0),
                })),
                Refuse(DecodeError::UnknownField(Place {
                    what: "resource",
                    index: Some(0),
                })),
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
            ],
        ),
        ("schema-3", [schema(), schema(), schema()]),
        (
            "v1-with-keys",
            [
                Refuse(DecodeError::UnknownField(Place {
                    what: "file",
                    index: Some(0),
                })),
                Refuse(DecodeError::UnknownField(Place {
                    what: "file",
                    index: Some(0),
                })),
                Refuse(DecodeError::UnknownField(Place {
                    what: "file",
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
    let readers = [Reader::v1(), Reader::current(), file_only_reader()];
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
                            None => assert_eq!(*schema, 2),
                            Some(l) => {
                                assert_eq!(*schema, 1);
                                assert_eq!(l.from_schema, 1);
                                assert_eq!(l.from_digest, nomos_canon::sha256::digest(&bytes));
                                assert_eq!(l.to, canon_id(&want, profile));
                                assert_eq!(
                                    encode(&canon, profile),
                                    fixture(&format!("{name}.v2"), profile)
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
                "schema-3" => 3,
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
            assert_eq!(old.canon, new.canon);
            assert_eq!(old.lineage.unwrap().to, canon_id(&new.canon, profile));
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
    items.len() != written.resources.len()
        || items.iter().any(|item| {
            let Some(Value::Text(path)) = item.get("path") else {
                return true;
            };
            !written.resources.iter().any(|r| &r.path == path)
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

/// Generated Canons, written in schema 1 where it can say them: the
/// migration and the direct reading agree, so no generated case changes
/// intent across the versions.
#[test]
fn generated_migrations_agree_with_direct_reading() {
    let migrated = std::cell::Cell::new(0u32);
    support::runner(21, 1024)
        .run(&valid_raw(), |raw| {
            let c = Canon::try_from(raw).unwrap();
            for profile in Profile::ALL {
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
