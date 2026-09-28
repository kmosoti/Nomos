//! Experiment `typed-validation` (grounding plan, `06-canon-artifact`): do
//! construction, decoding, and migration enforce the same domain invariants?
//!
//! Five paths reach a Canon: the validator itself, the typed builder, the
//! decoder in each profile, and the schema-1 migration. Each is given the
//! same inputs. The oracle for "malformed" is construction, not the code
//! under test: every malformed input is a valid one with one known rule
//! broken (`support::Break`), and the rule broken is the expected error.
//! The negative control is a `serde` derive over the same shape with no
//! `try_from` boundary, run through the same harness, which must report it.

mod support;

use std::cell::Cell;
use std::collections::BTreeMap;

use nomos_canon::artifact::{DecodeError, Profile, Reader, decode, raw_value, raw_value_v1};
use nomos_canon::model::{
    Canon, CanonBuilder, CanonError, RawCanon, RawSpec, RelationKind, Requirement,
    ServiceRequirement,
};
use nomos_core::condition::{Content, FileCondition};
use nomos_core::resource::Digest;
use proptest::prelude::*;
use support::*;

// ---------------------------------------------------------------------------
// The paths

/// What one path made of an input: a Canon, a typed error, or nothing,
/// because the path's types cannot say the input at all.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Accepted(Box<Canon>),
    Rejected(String),
    Unrepresentable,
}

fn validator(raw: &RawCanon) -> Outcome {
    match Canon::try_from(raw.clone()) {
        Ok(c) => Outcome::Accepted(Box::new(c)),
        Err(e) => Outcome::Rejected(format!("{e:?}")),
    }
}

/// The builder takes typed requirements, so a spec that is not one of the
/// five requirements cannot be written to it. This mapping is written here,
/// from canon-ir.md's table, not taken from the crate.
fn typed_requirement(spec: &RawSpec) -> Option<Requirement> {
    let digest = || {
        spec.digest
            .as_deref()
            .filter(|d| {
                d.len() == 64
                    && d.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .and_then(|d| Digest::from_hex(d).ok())
    };
    let none = spec.digest.is_none();
    match (spec.kind.as_str(), spec.state.as_str()) {
        ("file", "absent") if none => Some(Requirement::File(FileCondition::Absent)),
        ("file", "present-any") if none => Some(Requirement::File(FileCondition::Present {
            content: Content::Any,
        })),
        ("file", "present-exact") => digest().map(|d| {
            Requirement::File(FileCondition::Present {
                content: Content::Exactly(d),
            })
        }),
        ("service", "running") if none => Some(Requirement::Service(ServiceRequirement::Running)),
        ("service", "loaded") => {
            digest().map(|d| Requirement::Service(ServiceRequirement::Loaded(d)))
        }
        _ => None,
    }
}

fn typed_relation(kind: &str) -> Option<RelationKind> {
    match kind {
        "requires" => Some(RelationKind::Requires),
        "after" => Some(RelationKind::After),
        "on_change" => Some(RelationKind::OnChange),
        _ => None,
    }
}

fn builder(raw: &RawCanon) -> Outcome {
    let mut b = CanonBuilder::new(&raw.name);
    for r in &raw.resources {
        let Some(req) = typed_requirement(&r.spec) else {
            return Outcome::Unrepresentable;
        };
        let keys: Vec<&str> = r.keys.iter().map(String::as_str).collect();
        let nodes: Vec<&str> = r.disrupts.iter().map(String::as_str).collect();
        b = b.resource(&r.path, req, &keys, &nodes);
    }
    for rel in &raw.relations {
        let Some(kind) = typed_relation(&rel.kind) else {
            return Outcome::Unrepresentable;
        };
        b = b.relate(&rel.source, kind, &rel.target);
    }
    match b.build() {
        Ok(c) => Outcome::Accepted(Box::new(c)),
        Err(e) => Outcome::Rejected(format!("{e:?}")),
    }
}

/// The decoder, given the input as written. A valid input written out of
/// order is rejected as non-canonical, which is correct and not a parity
/// failure, so valid inputs are given in their canonical order.
fn decoder(raw: &RawCanon, profile: Profile) -> Outcome {
    let raw = match Canon::try_from(raw.clone()) {
        Ok(c) => c.to_raw(),
        Err(_) => raw.clone(),
    };
    decoded(decode(
        &profile.encode_value(&raw_value(&raw)),
        profile,
        &Reader::current(),
    ))
}

/// The schema-1 migration, for inputs schema 1 can say.
fn migration(raw: &RawCanon, profile: Profile) -> Outcome {
    let raw = match Canon::try_from(raw.clone()) {
        Ok(c) => c.to_raw(),
        Err(_) => raw.clone(),
    };
    match raw_value_v1(&raw) {
        None => Outcome::Unrepresentable,
        Some(v) => decoded(decode(
            &profile.encode_value(&v),
            profile,
            &Reader::current(),
        )),
    }
}

fn decoded(result: Result<nomos_canon::artifact::Decoded, DecodeError>) -> Outcome {
    match result {
        Ok(d) => Outcome::Accepted(Box::new(d.canon)),
        Err(DecodeError::Invalid(e)) => Outcome::Rejected(format!("{e:?}")),
        Err(e) => Outcome::Rejected(format!("{e:?}")),
    }
}

/// Every path's outcome for one input, by path name.
fn all_paths(raw: &RawCanon) -> BTreeMap<&'static str, Outcome> {
    BTreeMap::from([
        ("validator", validator(raw)),
        ("builder", builder(raw)),
        ("decode-cbor", decoder(raw, Profile::Cbor)),
        ("decode-jcs", decoder(raw, Profile::Jcs)),
        ("migrate-cbor", migration(raw, Profile::Cbor)),
        ("migrate-jcs", migration(raw, Profile::Jcs)),
    ])
}

/// The error each break must produce, stated from the break, not read from
/// the validator. `resource` is the position of the broken resource and
/// `relation` of the added relation.
fn expected(b: &Break, resources: usize, relations: usize) -> Vec<String> {
    let r0 = CanonError::InvalidPath { resource: 0 };
    let e = match b {
        Break::Name(_) => CanonError::InvalidName,
        Break::Path(_) => r0,
        Break::Kind(_) => {
            // The decoder refuses an unknown kind at the schema stage, before
            // the validator; either refusal names resource 0.
            return vec![
                format!("{:?}", CanonError::UnknownRequirement { resource: 0 }),
                format!("{:?}", DecodeError::UnsupportedKind { resource: 0 }),
            ];
        }
        Break::State(_) => CanonError::UnknownRequirement { resource: 0 },
        Break::Digest(None) | Break::Digest(Some(_)) | Break::MisplacedDigest => {
            CanonError::InvalidDigest { resource: 0 }
        }
        Break::Label(_) => CanonError::InvalidLabel { resource: 0 },
        Break::DuplicatePath => CanonError::DuplicatePath {
            resource: resources - 1,
        },
        Break::Dangling => CanonError::DanglingRelation {
            relation: relations - 1,
        },
        Break::SelfRelation => CanonError::SelfRelation {
            relation: relations - 1,
        },
        Break::RelationKind(_) => CanonError::UnknownRelation {
            relation: relations - 1,
        },
        Break::Cycle => CanonError::Cycle,
    };
    vec![format!("{e:?}")]
}

// ---------------------------------------------------------------------------
// Parity

/// Every path accepts every valid input, and all of them produce the same
/// Canon.
#[test]
fn every_path_accepts_a_valid_canon_as_the_same_canon() {
    let reached = Cell::new(BTreeMap::<&'static str, u32>::new());
    support::runner(11, 512)
        .run(&valid_raw(), |raw| {
            let reference = validator(&raw);
            let Outcome::Accepted(c) = &reference else {
                panic!("the generator made an invalid Canon: {reference:?}");
            };
            let mut counts = reached.take();
            for (path, outcome) in all_paths(&raw) {
                match outcome {
                    Outcome::Unrepresentable => {}
                    Outcome::Accepted(d) => {
                        prop_assert_eq!(&d, c, "{}", path);
                        *counts.entry(path).or_default() += 1;
                    }
                    Outcome::Rejected(e) => {
                        prop_assert!(false, "{path} rejected a valid Canon: {e}")
                    }
                }
            }
            reached.set(counts);
            Ok(())
        })
        .unwrap();
    println!("valid inputs accepted, by path: {:?}", reached.take());
}

/// No path accepts a malformed input, and every path that can say the
/// input rejects it with the error the break predicts.
#[test]
fn no_path_accepts_a_malformed_canon() {
    let counts = Cell::new(BTreeMap::<&'static str, (u32, u32)>::new());
    support::runner(12, 2048)
        .run(&(valid_raw(), a_break()), |(raw, b)| {
            let bad = apply(raw, &b);
            let want = expected(&b, bad.resources.len(), bad.relations.len());
            let mut tally = counts.take();
            for (path, outcome) in all_paths(&bad) {
                let entry = tally.entry(path).or_default();
                match outcome {
                    Outcome::Accepted(c) => {
                        prop_assert!(false, "{path} accepted {b:?} as {c:?}")
                    }
                    Outcome::Rejected(e) => {
                        prop_assert!(
                            want.contains(&e),
                            "{path} on {b:?}: {e}, expected one of {want:?}"
                        );
                        entry.0 += 1;
                    }
                    Outcome::Unrepresentable => entry.1 += 1,
                }
            }
            counts.set(tally);
            Ok(())
        })
        .unwrap();
    println!(
        "malformed inputs (rejected, unrepresentable), by path: {:?}",
        counts.take()
    );
}

/// The absent-with-contents countercase at the IR: a file that must be
/// absent and carries a digest is refused by every path that can say it,
/// and the builder cannot say it (`nomos-core`'s
/// `tests/compile-fail/absent-with-content.rs`).
#[test]
fn an_absent_file_with_contents_is_refused_everywhere() {
    let raw = RawCanon {
        name: "countercase".into(),
        resources: vec![resource(
            "/etc/a",
            spec("file", "absent", Some(hex_digest(7))),
            &[],
            &[],
        )],
        relations: vec![],
    };
    let want = format!("{:?}", CanonError::InvalidDigest { resource: 0 });
    for (path, outcome) in all_paths(&raw) {
        match outcome {
            Outcome::Rejected(e) => assert_eq!(e, want, "{path}"),
            Outcome::Unrepresentable => assert_eq!(path, "builder"),
            Outcome::Accepted(_) => panic!("{path} accepted an absent file with contents"),
        }
    }
}

/// Arbitrary text in every field: the validator never panics, and whatever
/// it accepts every other path accepts as the same Canon. Most of these
/// inputs are rejected; the count says how many were not.
#[test]
fn arbitrary_fields_neither_panic_nor_split_the_paths() {
    let text = || {
        prop::sample::select(vec![
            "",
            "a",
            "/a",
            "/b",
            "file",
            "service",
            "absent",
            "present-any",
            "present-exact",
            "running",
            "loaded",
            "requires",
            "after",
            "on_change",
            "/a/",
            "A",
            "k",
            "\u{0}",
            "\u{e9}",
            "aa",
        ])
        .prop_map(String::from)
    };
    let digest = prop::option::of(prop::sample::select(vec![
        hex_digest(1),
        "AB".into(),
        String::new(),
    ]));
    let strategy = (
        text(),
        prop::collection::vec(
            (
                text(),
                text(),
                text(),
                digest,
                prop::collection::vec(text(), 0..2),
            ),
            0..4,
        ),
        prop::collection::vec((text(), text(), text()), 0..3),
    )
        .prop_map(|(name, rs, rels)| RawCanon {
            name,
            resources: rs
                .into_iter()
                .map(
                    |(path, kind, state, digest, keys)| nomos_canon::model::RawResource {
                        path,
                        spec: RawSpec {
                            kind,
                            state,
                            digest,
                        },
                        keys,
                        disrupts: vec![],
                    },
                )
                .collect(),
            relations: rels
                .into_iter()
                .map(|(s, k, t)| relation(&s, &k, &t))
                .collect(),
        });
    let accepted = Cell::new(0u32);
    support::runner(13, 4096)
        .run(&strategy, |raw| {
            let reference = validator(&raw);
            for (path, outcome) in all_paths(&raw) {
                match (&reference, &outcome) {
                    (_, Outcome::Unrepresentable) => {}
                    (Outcome::Accepted(a), Outcome::Accepted(b)) => {
                        prop_assert_eq!(a, b, "{}", path)
                    }
                    (Outcome::Rejected(_), Outcome::Rejected(_)) => {}
                    (r, o) => prop_assert!(false, "{path}: validator {r:?}, path {o:?}"),
                }
            }
            if matches!(reference, Outcome::Accepted(_)) {
                accepted.set(accepted.get() + 1);
            }
            Ok(())
        })
        .unwrap();
    println!(
        "arbitrary inputs accepted by every path: {} of 4096",
        accepted.get()
    );
}

// ---------------------------------------------------------------------------
// Errors are secret-free

/// A sentinel written into each field of a malformed input never appears in
/// the error, from the validator or from either decoder, in `Debug` or in
/// `Display`.
#[test]
fn errors_never_repeat_the_input() {
    const S: &str = "SENTINEL-4f1c";
    let base = || RawCanon {
        name: "sentinel".into(),
        resources: vec![
            resource("/etc/a", spec("file", "absent", None), &[], &[]),
            resource("/etc/b", spec("file", "absent", None), &[], &[]),
        ],
        relations: vec![],
    };
    let mut cases: Vec<RawCanon> = Vec::new();
    let mut c = base();
    c.name = S.into();
    cases.push(c);
    let mut c = base();
    c.resources[0].path = format!("relative/{S}");
    cases.push(c);
    let mut c = base();
    c.resources[0].spec.kind = S.into();
    cases.push(c);
    let mut c = base();
    c.resources[0].spec.state = S.into();
    cases.push(c);
    let mut c = base();
    c.resources[0].spec = spec("file", "present-exact", Some(S.into()));
    cases.push(c);
    let mut c = base();
    c.resources[0].keys.push(S.into());
    cases.push(c);
    let mut c = base();
    c.resources[0].disrupts.push(S.into());
    cases.push(c);
    let mut c = base();
    c.relations.push(relation("/etc/a", S, "/etc/b"));
    cases.push(c);
    let mut c = base();
    c.relations
        .push(relation("/etc/a", "requires", &format!("/{S}")));
    cases.push(c);
    for (i, raw) in cases.iter().enumerate() {
        let e = Canon::try_from(raw.clone()).unwrap_err();
        for shown in [format!("{e:?}"), format!("{e}")] {
            assert!(!shown.contains(S), "case {i}: {shown}");
        }
        for profile in Profile::ALL {
            let bytes = profile.encode_value(&raw_value(raw));
            let e = decode(&bytes, profile, &Reader::current()).unwrap_err();
            for shown in [format!("{e:?}"), format!("{e}")] {
                assert!(!shown.contains(S), "case {i} {profile:?}: {shown}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The negative control

/// The rival: the same shape, deserialized by a `serde` derive straight into
/// a value with public fields, with no `try_from` boundary.
mod derived {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct Canon {
        pub schema: u64,
        pub name: String,
        pub resources: Vec<Resource>,
        pub relations: Vec<Relation>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct Resource {
        pub path: String,
        pub spec: Spec,
        pub keys: Vec<String>,
        pub disrupts: Vec<String>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct Spec {
        pub kind: String,
        pub state: String,
        pub digest: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    pub struct Relation {
        pub source: String,
        pub target: String,
        pub kind: String,
    }
}

/// The harness this experiment uses, applied to one path given as a
/// function from artifact bytes to "accepted": how many malformed inputs it
/// let through, of how many.
fn leaks(accepts: impl Fn(&[u8]) -> bool, seed: u8) -> (u32, u32) {
    let leaked = Cell::new(0u32);
    let seen = Cell::new(0u32);
    support::runner(seed, 512)
        .run(&(valid_raw(), a_break()), |(raw, b)| {
            let bad = apply(raw, &b);
            let bytes = Profile::Jcs.encode_value(&raw_value(&bad));
            seen.set(seen.get() + 1);
            if accepts(&bytes) {
                leaked.set(leaked.get() + 1);
            }
            Ok(())
        })
        .unwrap();
    (leaked.get(), seen.get())
}

/// The derive lets malformed Canons through and the harness reports it; the
/// decoder under the same harness lets none through.
#[test]
fn the_harness_catches_a_derive_without_the_boundary() {
    let (leaked, seen) = leaks(|b| serde_json::from_slice::<derived::Canon>(b).is_ok(), 14);
    println!("derive without try_from: {leaked} of {seen} malformed inputs accepted");
    assert!(leaked > 0, "the negative control did not fire");
    let (leaked, seen) = leaks(|b| decode(b, Profile::Jcs, &Reader::current()).is_ok(), 14);
    println!("decoder: {leaked} of {seen} malformed inputs accepted");
    assert_eq!(leaked, 0);
}

// ---------------------------------------------------------------------------
// What the errors say

/// Each error's text names its stage and its place, from the schema's words
/// only; the sentinel test above shows the input's words never appear.
#[test]
fn errors_say_where_and_why() {
    use nomos_canon::artifact::Place;
    use nomos_canon::value::SyntaxError;
    let place = |what, index| Place { what, index };
    let cases: Vec<(DecodeError, &str)> = vec![
        (
            DecodeError::Syntax(SyntaxError::Truncated),
            "syntax: the input ends inside a value",
        ),
        (DecodeError::NonCanonical, "not the canonical encoding"),
        (DecodeError::UnsupportedSchema, "unsupported schema version"),
        (
            DecodeError::UnknownField(place("resource", Some(2))),
            "resource 2: a field the schema does not define",
        ),
        (
            DecodeError::MissingField(place("canon", None)),
            "canon: a required field is missing",
        ),
        (
            DecodeError::WrongType(place("spec", Some(0))),
            "spec 0: a field of the wrong type",
        ),
        (
            DecodeError::UnsupportedKind { resource: 4 },
            "resource 4: a kind this reader does not execute",
        ),
        (
            DecodeError::Invalid(CanonError::Cycle),
            "invalid Canon: the relations form a cycle",
        ),
    ];
    for (e, text) in cases {
        assert_eq!(e.to_string(), text);
    }
    let canon_errors: Vec<(CanonError, &str)> = vec![
        (CanonError::InvalidName, "invalid Canon name"),
        (
            CanonError::InvalidPath { resource: 1 },
            "resource 1: invalid path",
        ),
        (
            CanonError::UnknownRequirement { resource: 1 },
            "resource 1: unknown kind or state",
        ),
        (
            CanonError::InvalidDigest { resource: 1 },
            "resource 1: invalid or misplaced digest",
        ),
        (
            CanonError::InvalidLabel { resource: 1 },
            "resource 1: invalid conflict key or node",
        ),
        (
            CanonError::DuplicatePath { resource: 1 },
            "resource 1: a second resource at one path",
        ),
        (
            CanonError::UnknownRelation { relation: 3 },
            "relation 3: unknown kind",
        ),
        (
            CanonError::DanglingRelation { relation: 3 },
            "relation 3: names a resource the Canon lacks",
        ),
        (
            CanonError::SelfRelation { relation: 3 },
            "relation 3: relates a resource to itself",
        ),
    ];
    for (e, text) in canon_errors {
        assert_eq!(e.to_string(), text);
    }
    let raw = RawCanon {
        name: "a".into(),
        ..RawCanon::default()
    };
    let canon = Canon::try_from(raw).unwrap();
    assert_eq!(format!("{:?}", canon.name()), "Name(a)");
    let id = nomos_canon::artifact::canon_id(&canon, Profile::Cbor);
    assert_eq!(format!("{id:?}"), format!("CanonId({id})"));
    assert!(id.to_string().starts_with("sha256:") && id.to_string().len() == 7 + 64);
}

/// A path of 4,096 bytes is a path; one of 4,097 is not, from the builder as
/// from the validator (canon-ir.md: at most 4,096 bytes).
#[test]
fn a_path_is_at_most_4096_bytes() {
    let path = |len: usize| format!("/{}", "a".repeat(len - 1));
    assert_eq!(nomos_canon::model::MAX_PATH, 4096);
    let at = CanonBuilder::new("p")
        .file(&path(4096), FileCondition::Absent)
        .build();
    assert!(at.is_ok());
    let over = CanonBuilder::new("p")
        .file(&path(4097), FileCondition::Absent)
        .build();
    assert_eq!(over.err(), Some(CanonError::InvalidPath { resource: 0 }));
}

/// The builder's shorthands add a resource with no conflict keys and no
/// disruption, as their documentation says, and nothing else.
#[test]
fn the_builder_shorthands_add_unlabeled_resources() {
    let built = CanonBuilder::new("short")
        .file("/etc/a", FileCondition::Absent)
        .service("/run/b", ServiceRequirement::Running)
        .build()
        .unwrap();
    let raw = RawCanon {
        name: "short".into(),
        resources: vec![
            resource("/etc/a", spec("file", "absent", None), &[], &[]),
            resource("/run/b", spec("service", "running", None), &[], &[]),
        ],
        relations: vec![],
    };
    assert_eq!(built, Canon::try_from(raw).unwrap());
}
