//! Experiment `admission-composition` (Phase 1 plan, `08-resource-families`):
//! does admission reject every overlapping written property, and only
//! those? And the schema-3 validator's reading of each family.
//!
//! The oracle for admission is the property table of resource-families.md,
//! restated here as `writes`, not the validator's `Property::written`: every
//! pair of resources over every pair of families, with names chosen so that
//! the same text appears in several families, is admitted exactly when the
//! table says no property is written twice. The per-family cases are the
//! key and `spec` rules of canon-ir.md, Schema Version 3, each a valid
//! resource with one rule broken; the rule broken is the expected error.

mod support;

use nomos_canon::artifact::{DecodeError, Place, Profile, Reader, decode, raw_value};
use nomos_canon::model::{Canon, CanonError, RawCanon3, RawResource3};
use support::*;

/// A digest in its text form.
const DIGEST: &str = "abababababababababababababababababababababababababababababababab";

/// A `spec` as field pairs.
type Fields = Vec<(&'static str, &'static str)>;

/// A valid spec per family, with every optional field left out.
fn minimal(kind: &str) -> Vec<(&'static str, &'static str)> {
    match kind {
        "file" => vec![("state", "present"), ("content", "any")],
        "directory" => vec![("state", "present")],
        "service" => vec![("state", "running")],
        "unit" => vec![("active", "active"), ("enabled", "any")],
        "sysctl" => vec![("value", "1")],
        "user" => vec![("state", "present"), ("class", "system")],
        "package" => vec![("state", "installed")],
        _ => unreachable!(),
    }
}

const KINDS: [&str; 7] = [
    "directory",
    "file",
    "package",
    "service",
    "sysctl",
    "unit",
    "user",
];

/// Two valid names per family. The same word, `nginx`, is in every family,
/// and the paths are shared by the three path families.
fn names(kind: &str) -> [&'static str; 2] {
    match kind {
        "file" | "directory" | "service" => ["/etc/nginx", "/etc/other"],
        "unit" => ["nginx.service", "other.service"],
        "sysctl" => ["net.nginx", "net.other"],
        "user" => ["nginx", "other"],
        "package" => ["nginx", "other"],
        _ => unreachable!(),
    }
}

/// The property a resource writes: the table of resource-families.md,
/// Admission.
fn writes(kind: &str, name: &str) -> String {
    match kind {
        "file" | "directory" | "service" => format!("path:{name}"),
        _ => format!("{kind}:{name}"),
    }
}

fn one(kind: &str, name: &str, spec: &[(&str, &str)]) -> RawResource3 {
    resource3(kind, name, spec, &[], &[])
}

fn canon3(resources: Vec<RawResource3>) -> RawCanon3 {
    RawCanon3 {
        name: "families".into(),
        resources,
        relations: vec![],
    }
}

/// Every pair of resources over every pair of families and names: a second
/// resource with the first one's key is a duplicate; with another key but
/// the first one's written property, a conflict; otherwise admitted. The
/// validator and the decoder in both profiles agree with the table.
#[test]
fn admission_rejects_exactly_the_overlapping_properties() {
    let (mut admitted, mut conflicts, mut duplicates) = (0, 0, 0);
    for a in KINDS {
        for b in KINDS {
            for an in names(a) {
                for bn in names(b) {
                    let raw = canon3(vec![one(a, an, &minimal(a)), one(b, bn, &minimal(b))]);
                    let expected = if a == b && an == bn {
                        duplicates += 1;
                        Err(CanonError::DuplicatePath { resource: 1 })
                    } else if writes(a, an) == writes(b, bn) {
                        conflicts += 1;
                        Err(CanonError::Conflict { resource: 1 })
                    } else {
                        admitted += 1;
                        Ok(())
                    };
                    let got = Canon::try_from(raw.clone()).map(|_| ());
                    assert_eq!(got, expected, "{a}:{an} then {b}:{bn}");
                    // The decoder reads the artifact as written, so an
                    // admitted pair is given in its canonical order.
                    for profile in Profile::ALL {
                        let written = match Canon::try_from(raw.clone()) {
                            Ok(c) => c.to_raw(),
                            Err(_) => raw.clone(),
                        };
                        let bytes = profile.encode_value(&raw_value(&written));
                        let got = decode(&bytes, profile, &Reader::current()).map(|_| ());
                        assert_eq!(
                            got,
                            expected.map_err(DecodeError::Invalid),
                            "{a}:{an} then {b}:{bn} {profile:?}"
                        );
                    }
                }
            }
        }
    }
    println!("pairs: {admitted} admitted, {conflicts} conflicts, {duplicates} duplicates");
    // Three path families sharing two paths: 3 * 3 * 2 same-path pairs, of
    // which 3 * 2 are one key.
    assert_eq!(duplicates, 7 * 2);
    assert_eq!(conflicts, 3 * 2 * 2);
    assert_eq!(admitted, 7 * 7 * 4 - 14 - 12);
}

/// The rival: an admission that checks keys only, as a map keyed by
/// resource does on its own. Under the same table it admits a file and a
/// directory at one path, so the harness above would report it.
#[test]
fn the_harness_catches_an_admission_that_checks_keys_only() {
    let keys_only = |a: &str, an: &str, b: &str, bn: &str| !(a == b && an == bn);
    let mut missed = 0;
    for a in KINDS {
        for b in KINDS {
            for an in names(a) {
                for bn in names(b) {
                    let table = writes(a, an) != writes(b, bn);
                    if keys_only(a, an, b, bn) != table {
                        missed += 1;
                    }
                }
            }
        }
    }
    assert_eq!(missed, 3 * 2 * 2, "the negative control did not fire");
}

/// The error each broken resource must give, alone in a Canon.
fn rejected(kind: &str, name: &str, spec: &[(&str, &str)]) -> CanonError {
    Canon::try_from(canon3(vec![one(kind, name, spec)])).unwrap_err()
}

#[test]
fn every_family_rejects_an_invalid_name() {
    let path = |resource| CanonError::InvalidPath { resource };
    let name = |resource| CanonError::InvalidResourceName { resource };
    let cases: [(&str, &str, CanonError); 20] = [
        ("file", "relative", path(0)),
        ("file", "/a/../b", path(0)),
        ("file", "/a/", path(0)),
        ("directory", "/", path(0)),
        ("directory", "/a//b", path(0)),
        ("service", "", path(0)),
        ("unit", "nginx", name(0)),
        ("unit", ".service", name(0)),
        ("unit", "a/b.service", name(0)),
        ("unit", "a b.service", name(0)),
        ("sysctl", "net", name(0)),
        ("sysctl", "net..a", name(0)),
        ("sysctl", "net/ipv4/ip_forward", name(0)),
        ("sysctl", "Net.a", name(0)),
        ("user", "Root", name(0)),
        ("user", "1abc", name(0)),
        ("user", "abcdefghijklmnopqrstuvwxyz0123456", name(0)),
        ("package", "a", name(0)),
        ("package", "Nginx", name(0)),
        ("package", "-a", name(0)),
    ];
    for (kind, bad, expected) in cases {
        assert_eq!(
            rejected(kind, bad, &minimal(kind)),
            expected,
            "{kind}:{bad}"
        );
    }
    let long = format!("/{}", "a".repeat(4096));
    assert_eq!(rejected("file", &long, &minimal("file")), path(0));
}

#[test]
fn every_family_reads_only_its_spec() {
    let field = CanonError::InvalidField { resource: 0 };
    let unknown = CanonError::UnknownRequirement { resource: 0 };
    let digest = CanonError::InvalidDigest { resource: 0 };
    let cases: Vec<(&str, &str, Fields, CanonError)> = vec![
        // A state the family does not have.
        ("file", "/a", vec![("state", "present-any")], unknown),
        ("directory", "/a", vec![], unknown),
        ("user", "u", vec![("state", "running")], unknown),
        ("package", "pp", vec![("state", "present")], unknown),
        ("service", "/a", vec![("state", "absent")], unknown),
        // A field the state does not allow.
        (
            "file",
            "/a",
            vec![("state", "absent"), ("mode", "0644")],
            unknown,
        ),
        (
            "directory",
            "/a",
            vec![("state", "present"), ("content", "any")],
            unknown,
        ),
        (
            "service",
            "/a",
            vec![("state", "running"), ("digest", "00")],
            unknown,
        ),
        (
            "user",
            "u",
            vec![("state", "absent"), ("class", "system")],
            unknown,
        ),
        (
            "package",
            "pp",
            vec![("state", "absent"), ("version", "1.0")],
            unknown,
        ),
        (
            "unit",
            "a.service",
            vec![("active", "active"), ("enabled", "any"), ("state", "x")],
            unknown,
        ),
        // A required field missing.
        ("file", "/a", vec![("state", "present")], field),
        ("unit", "a.service", vec![("active", "active")], field),
        ("unit", "a.service", vec![("enabled", "enabled")], field),
        ("sysctl", "net.a", vec![], field),
        ("user", "u", vec![("state", "present")], field),
        ("service", "/a", vec![("state", "loaded")], digest),
        // A value the field does not allow.
        (
            "file",
            "/a",
            vec![("state", "present"), ("content", "AB")],
            digest,
        ),
        (
            "file",
            "/a",
            vec![("state", "present"), ("content", "any"), ("mode", "644")],
            field,
        ),
        (
            "file",
            "/a",
            vec![("state", "present"), ("content", "any"), ("mode", "0648")],
            field,
        ),
        (
            "file",
            "/a",
            vec![("state", "present"), ("content", "any"), ("mode", "07777")],
            field,
        ),
        (
            "file",
            "/a",
            vec![("state", "present"), ("content", "any"), ("owner", "Root")],
            field,
        ),
        (
            "directory",
            "/a",
            vec![("state", "present"), ("group", "")],
            field,
        ),
        (
            "unit",
            "a.service",
            vec![("active", "running"), ("enabled", "any")],
            field,
        ),
        (
            "unit",
            "a.service",
            vec![("active", "any"), ("enabled", "static")],
            field,
        ),
        ("sysctl", "net.a", vec![("value", "")], field),
        ("sysctl", "net.a", vec![("value", " 1")], field),
        ("sysctl", "net.a", vec![("value", "1  2")], field),
        ("sysctl", "net.a", vec![("value", "1\t2")], field),
        (
            "user",
            "u",
            vec![("state", "present"), ("class", "admin")],
            field,
        ),
        (
            "user",
            "u",
            vec![
                ("state", "present"),
                ("class", "system"),
                ("home", "home/u"),
            ],
            field,
        ),
        (
            "user",
            "u",
            vec![
                ("state", "present"),
                ("class", "system"),
                ("shell", "/bin/../sh"),
            ],
            field,
        ),
        (
            "package",
            "pp",
            vec![("state", "installed"), ("version", "")],
            field,
        ),
        (
            "package",
            "pp",
            vec![("state", "installed"), ("version", "v1")],
            field,
        ),
        (
            "package",
            "pp",
            vec![("state", "installed"), ("version", "1 0")],
            field,
        ),
    ];
    for (kind, name, spec, expected) in cases {
        assert_eq!(
            rejected(kind, name, &spec),
            expected,
            "{kind}:{name} {spec:?}"
        );
    }
}

/// Every field a family defines is accepted in every allowed value.
#[test]
fn every_family_accepts_its_full_spec() {
    let full: [(&str, &str, Fields); 7] = [
        (
            "file",
            "/etc/a",
            vec![
                ("state", "present"),
                ("content", "any"),
                ("owner", "root"),
                ("group", "adm"),
                ("mode", "7777"),
            ],
        ),
        (
            "directory",
            "/etc",
            vec![
                ("state", "present"),
                ("owner", "_apt"),
                ("group", "nogroup"),
                ("mode", "0000"),
            ],
        ),
        (
            "service",
            "/run/a",
            vec![("state", "loaded"), ("digest", DIGEST)],
        ),
        (
            "unit",
            "getty@tty1.service",
            vec![("active", "inactive"), ("enabled", "disabled")],
        ),
        (
            "sysctl",
            "net.ipv4.tcp_rmem",
            vec![("value", "4096 87380 6291456")],
        ),
        (
            "user",
            "machine$",
            vec![
                ("state", "present"),
                ("class", "regular"),
                ("home", "/home/m"),
                ("shell", "/bin/sh"),
            ],
        ),
        (
            "package",
            "libstdc++6",
            vec![
                ("state", "installed"),
                ("version", "1:12.2.0-14+deb12u1~bpo"),
            ],
        ),
    ];
    for (kind, name, spec) in full {
        let c = Canon::try_from(canon3(vec![one(kind, name, &spec)]))
            .unwrap_or_else(|e| panic!("{kind}:{name}: {e:?}"));
        // The normalized form states exactly the fields given.
        let raw = c.to_raw();
        let got: Vec<(&str, &str)> = raw.resources[0]
            .spec
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let mut want = spec.clone();
        want.sort();
        assert_eq!(got, want, "{kind}:{name}");
    }
}

/// A relation's ends are keys: a key of another family at a path the
/// Canon manages, or a bare path, names nothing.
#[test]
fn relation_ends_are_keys() {
    let base = || {
        canon3(vec![
            one("file", "/etc/a", &minimal("file")),
            one("unit", "a.service", &minimal("unit")),
        ])
    };
    let with = |source: &str, target: &str| {
        let mut raw = base();
        raw.relations.push(relation(source, "on_change", target));
        Canon::try_from(raw).map(|_| ())
    };
    assert_eq!(with("file:/etc/a", "unit:a.service"), Ok(()));
    let dangling = Err(CanonError::DanglingRelation { relation: 0 });
    assert_eq!(with("directory:/etc/a", "unit:a.service"), dangling);
    assert_eq!(with("/etc/a", "unit:a.service"), dangling);
    assert_eq!(with("file:/etc/a", "a.service"), dangling);
    assert_eq!(with("file:/etc/a", "service:a.service"), dangling);
    assert_eq!(
        with("unit:a.service", "unit:a.service"),
        Err(CanonError::SelfRelation { relation: 0 })
    );
}

/// A spec field schema 3 does not define for the kind is refused at the
/// schema stage, before the validator, and named by its resource.
#[test]
fn an_undefined_spec_field_is_a_schema_error() {
    let raw = canon3(vec![
        one("sysctl", "net.a", &minimal("sysctl")),
        one(
            "unit",
            "a.service",
            &[("active", "active"), ("enabled", "any"), ("value", "1")],
        ),
    ]);
    for profile in Profile::ALL {
        let bytes = profile.encode_value(&raw_value(&raw));
        assert_eq!(
            decode(&bytes, profile, &Reader::current()).err(),
            Some(DecodeError::UnknownField(Place {
                what: "spec",
                index: Some(1),
            }))
        );
    }
}

/// Generated schema-3 Canons: each decodes from its encoding to itself, in
/// both profiles, and one written out of order is refused as non-canonical
/// rather than read as another Canon.
#[test]
fn generated_schema3_canons_round_trip() {
    let seen = std::cell::Cell::new(0u32);
    support::runner(31, 512)
        .run(&valid_raw3(), |raw| {
            let c = Canon::try_from(raw).unwrap();
            for profile in Profile::ALL {
                let bytes = nomos_canon::artifact::encode(&c, profile);
                let d = decode(&bytes, profile, &Reader::current()).unwrap();
                proptest::prop_assert_eq!(&d.canon, &c);
                proptest::prop_assert_eq!(d.schema, 3);
                let mut reversed = c.to_raw();
                if reversed.resources.len() > 1 {
                    reversed.resources.reverse();
                    let bytes = profile.encode_value(&raw_value(&reversed));
                    proptest::prop_assert_eq!(
                        decode(&bytes, profile, &Reader::current()).err(),
                        Some(DecodeError::NonCanonical)
                    );
                }
            }
            seen.set(seen.get() + 1);
            Ok(())
        })
        .unwrap();
    println!("generated schema-3 Canons round-tripped: {}", seen.get());
}
