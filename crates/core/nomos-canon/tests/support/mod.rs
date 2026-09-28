//! Generators and fixtures shared by the Canon artifact experiments.
//!
//! The generators produce `RawCanon` values, valid and malformed, from a
//! fixed seed bank. Valid ones are built from parts the validator accepts,
//! in a generated order, with generated duplicates of set members, so that
//! the equivalence laws see the same Canon written many ways.

#![allow(dead_code)]

use nomos_canon::model::{RawCanon, RawRelation, RawResource, RawSpec};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

pub fn runner(seed: u8, cases: u32) -> TestRunner {
    let mut bytes = [0u8; 32];
    bytes[0] = 0x4e;
    bytes[1] = 0x36;
    bytes[2] = seed;
    let config = Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    };
    TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &bytes))
}

pub fn hex_digest(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

pub fn spec(kind: &str, state: &str, digest: Option<String>) -> RawSpec {
    RawSpec {
        kind: kind.into(),
        state: state.into(),
        digest,
    }
}

pub fn resource(path: &str, spec: RawSpec, keys: &[&str], disrupts: &[&str]) -> RawResource {
    RawResource {
        path: path.into(),
        spec,
        keys: keys.iter().map(|s| s.to_string()).collect(),
        disrupts: disrupts.iter().map(|s| s.to_string()).collect(),
    }
}

pub fn relation(source: &str, kind: &str, target: &str) -> RawRelation {
    RawRelation {
        source: source.into(),
        target: target.into(),
        kind: kind.into(),
    }
}

/// The golden corpus: named Canons, each exercising part of the encodings.
pub fn golden() -> Vec<(&'static str, RawCanon)> {
    vec![
        (
            "empty",
            RawCanon {
                name: "empty".into(),
                ..RawCanon::default()
            },
        ),
        (
            "telemetry",
            RawCanon {
                name: "telemetry-node".into(),
                resources: vec![
                    resource(
                        "/etc/nomos/cell.conf",
                        spec("file", "present-exact", Some(hex_digest(0xab))),
                        &["file:/etc/nomos/cell.conf"],
                        &[],
                    ),
                    resource(
                        "/etc/nomos/ca.pem",
                        spec("file", "present-any", None),
                        &[],
                        &[],
                    ),
                    resource(
                        "/run/nomos-cell",
                        spec("service", "running", None),
                        &["systemd:nomos-cell.service"],
                        &["node-a"],
                    ),
                    resource("/etc/motd", spec("file", "absent", None), &[], &[]),
                ],
                relations: vec![
                    relation("/etc/nomos/cell.conf", "on_change", "/run/nomos-cell"),
                    relation("/etc/nomos/ca.pem", "requires", "/run/nomos-cell"),
                    relation("/etc/motd", "after", "/etc/nomos/ca.pem"),
                ],
            },
        ),
        (
            "text",
            RawCanon {
                name: "text".into(),
                resources: vec![
                    resource(
                        "/srv/caf\u{e9}/menu",
                        spec("file", "present-any", None),
                        &[],
                        &[],
                    ),
                    resource(
                        "/srv/cafe\u{301}/menu",
                        spec("file", "absent", None),
                        &[],
                        &[],
                    ),
                    resource(
                        "/tmp/quote\"back\\slash",
                        spec("file", "absent", None),
                        &[],
                        &[],
                    ),
                    resource(
                        "/tmp/tab\there\u{1}",
                        spec("file", "absent", None),
                        &[],
                        &[],
                    ),
                    resource("/tmp/\u{1f600}", spec("file", "absent", None), &[], &[]),
                ],
                relations: vec![],
            },
        ),
        (
            "legacy",
            RawCanon {
                name: "legacy-files".into(),
                resources: vec![
                    resource(
                        "/etc/hosts",
                        spec("file", "present-exact", Some(hex_digest(0x0f))),
                        &[],
                        &[],
                    ),
                    resource("/etc/hostname", spec("file", "present-any", None), &[], &[]),
                    resource("/etc/issue", spec("file", "absent", None), &[], &[]),
                ],
                relations: vec![
                    relation("/etc/hostname", "requires", "/etc/hosts"),
                    relation("/etc/hosts", "on_change", "/etc/issue"),
                ],
            },
        ),
        (
            "loaded",
            RawCanon {
                name: "loaded".into(),
                resources: vec![
                    resource(
                        "/etc/svc.conf",
                        spec("file", "present-exact", Some(hex_digest(0x02))),
                        &["file:/etc/svc.conf"],
                        &[],
                    ),
                    resource(
                        "/run/svc.loaded",
                        spec("service", "loaded", Some(hex_digest(0x02))),
                        &["systemd:svc"],
                        &["rack-1/node-7"],
                    ),
                ],
                relations: vec![relation("/etc/svc.conf", "requires", "/run/svc.loaded")],
            },
        ),
    ]
}

// ---------------------------------------------------------------------------
// Generators

/// One character over the 64 a Canon name may have.
const LONG_NAME: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

const PATHS: [&str; 6] = ["/a", "/b", "/etc/c", "/etc/c/d", "/srv/\u{e9}", "/x y"];
const LABELS: [&str; 4] = ["k1", "file:/a", "systemd:x.service", "n/1"];

fn valid_spec() -> impl Strategy<Value = RawSpec> {
    prop_oneof![
        Just(spec("file", "absent", None)),
        Just(spec("file", "present-any", None)),
        (0u8..3).prop_map(|d| spec("file", "present-exact", Some(hex_digest(d)))),
        Just(spec("service", "running", None)),
        (0u8..3).prop_map(|d| spec("service", "loaded", Some(hex_digest(d)))),
    ]
}

fn labels() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(prop::sample::select(LABELS.to_vec()), 0..4)
        .prop_map(|v| v.into_iter().map(String::from).collect())
}

/// A valid Canon, written in a generated order with generated duplicates of
/// relations, keys, and nodes. Relations only go from a lower to a higher
/// path index, so they never form a cycle.
pub fn valid_raw() -> impl Strategy<Value = RawCanon> {
    (
        prop::sample::select(vec!["a", "canon-1", "telemetry-node"]),
        prop::sample::subsequence(PATHS.to_vec(), 0..=PATHS.len()),
        prop::collection::vec((valid_spec(), labels(), labels()), PATHS.len()),
        prop::collection::vec((0usize..6, 0usize..6, 0usize..3), 0..8),
        any::<u64>(),
    )
        .prop_map(|(name, paths, specs, rels, shuffle)| {
            let mut resources: Vec<RawResource> = paths
                .iter()
                .zip(specs)
                .map(|(p, (s, k, d))| RawResource {
                    path: p.to_string(),
                    spec: s,
                    keys: k,
                    disrupts: d,
                })
                .collect();
            let kinds = ["requires", "after", "on_change"];
            let mut relations: Vec<RawRelation> = rels
                .into_iter()
                .filter(|(a, b, _)| a < b && *b < paths.len())
                .map(|(a, b, k)| relation(paths[a], kinds[k], paths[b]))
                .collect();
            rotate(&mut resources, shuffle);
            rotate(&mut relations, shuffle >> 8);
            RawCanon {
                name: name.into(),
                resources,
                relations,
            }
        })
}

/// A deterministic reordering, from `seed`, that the laws must see through.
pub fn rotate<T>(items: &mut [T], seed: u64) {
    if items.len() > 1 {
        let k = (seed as usize) % items.len();
        items.rotate_left(k);
        if seed & 1 == 1 {
            items.reverse();
        }
    }
}

/// One way to break a valid Canon.
#[derive(Debug, Clone)]
pub enum Break {
    Name(&'static str),
    Path(&'static str),
    Kind(&'static str),
    State(&'static str),
    Digest(Option<&'static str>),
    MisplacedDigest,
    Label(&'static str),
    DuplicatePath,
    Dangling,
    SelfRelation,
    RelationKind(&'static str),
    Cycle,
}

pub fn a_break() -> impl Strategy<Value = Break> {
    prop_oneof![
        prop::sample::select(vec!["", "A", "1abc", "a b", "a_b", LONG_NAME]).prop_map(Break::Name),
        prop::sample::select(vec!["", "relative", "/a/../b", "/a//b", "/a/", "/a\0b"])
            .prop_map(Break::Path),
        prop::sample::select(vec!["", "sysctl", "File", "service "]).prop_map(Break::Kind),
        prop::sample::select(vec!["", "present", "Absent", "stopped"]).prop_map(Break::State),
        prop::sample::select(vec![
            None,
            Some(""),
            Some("AB"),
            Some("abababababababababababababababababababababababababababababababaB"),
            Some("zzababababababababababababababababababababababababababababababab"),
        ])
        .prop_map(Break::Digest),
        Just(Break::MisplacedDigest),
        prop::sample::select(vec!["", "K", "a b", "é", "k\n"]).prop_map(Break::Label),
        Just(Break::DuplicatePath),
        Just(Break::Dangling),
        Just(Break::SelfRelation),
        prop::sample::select(vec!["", "Requires", "before"]).prop_map(Break::RelationKind),
        Just(Break::Cycle),
    ]
}

/// Applies `b` to a Canon with at least two resources, or gives it two.
pub fn apply(mut raw: RawCanon, b: &Break) -> RawCanon {
    while raw.resources.len() < 2 {
        let path = if raw.resources.is_empty() {
            "/zz0"
        } else {
            "/zz1"
        };
        raw.resources
            .push(resource(path, spec("file", "absent", None), &[], &[]));
    }
    let first = raw.resources[0].path.clone();
    let second = raw.resources[1].path.clone();
    match b {
        Break::Name(n) => raw.name = n.to_string(),
        Break::Path(p) => raw.resources[0].path = p.to_string(),
        Break::Kind(k) => raw.resources[0].spec.kind = k.to_string(),
        Break::State(s) => raw.resources[0].spec.state = s.to_string(),
        Break::Digest(d) => {
            raw.resources[0].spec = RawSpec {
                kind: "file".into(),
                state: "present-exact".into(),
                digest: d.map(String::from),
            };
        }
        Break::MisplacedDigest => {
            raw.resources[0].spec = spec("file", "absent", Some(hex_digest(1)));
        }
        Break::Label(l) => raw.resources[0].keys.push(l.to_string()),
        Break::DuplicatePath => {
            let mut copy = raw.resources[0].clone();
            copy.spec = spec("file", "present-any", None);
            raw.resources.push(copy);
        }
        Break::Dangling => raw.relations.push(relation(&first, "requires", "/nowhere")),
        Break::SelfRelation => raw.relations.push(relation(&first, "after", &first)),
        Break::RelationKind(k) => raw.relations.push(relation(&first, k, &second)),
        Break::Cycle => {
            raw.relations.push(relation(&first, "requires", &second));
            raw.relations.push(relation(&second, "after", &first));
        }
    }
    raw
}
