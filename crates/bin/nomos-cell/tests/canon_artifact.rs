//! The Canon artifact at the composition boundary (`06-canon-artifact`):
//! bytes are decoded and validated by `nomos-canon`, become the kernel's
//! Canon, and are enforced on the mock host. The compatibility-matrix
//! negative control runs here, where effects exist: an artifact with a
//! mutating kind the reader does not know must be refused before any
//! effect, and a reader that skips the kind "for forward compatibility"
//! must be seen to act on the rest.

mod support;

use nomos_app::kernel::{Canon, RunOutcome};
use nomos_canon::artifact::{DecodeError, Profile, Reader, decode, encode};
use nomos_canon::model::{CanonBuilder, RawCanon, RawRelation, RawResource, RawSpec};
use nomos_canon::value::Value;
use support::*;

fn hex(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

/// The refresh scenario as an artifact: the configuration must hold the new
/// revision, the service must run, and a change to the first refreshes the
/// second.
fn obligation_artifact(profile: Profile) -> Vec<u8> {
    let raw = RawCanon {
        name: "refresh".into(),
        resources: vec![
            RawResource {
                path: CONF.into(),
                spec: RawSpec {
                    kind: "file".into(),
                    state: "present-exact".into(),
                    digest: Some(hex(NEW)),
                },
                keys: vec!["file:/etc/svc.conf".into()],
                disrupts: vec![],
            },
            RawResource {
                path: SVC.into(),
                spec: RawSpec {
                    kind: "service".into(),
                    state: "running".into(),
                    digest: None,
                },
                keys: vec!["systemd:svc".into()],
                disrupts: vec![],
            },
        ],
        relations: vec![RawRelation {
            source: CONF.into(),
            target: SVC.into(),
            kind: "on_change".into(),
        }],
    };
    let canon = nomos_canon::model::Canon::try_from(raw).unwrap();
    encode(&canon, profile)
}

/// The Cell's intake: bytes to the kernel's Canon, or the reason there is
/// none. Nothing is enforced on an error.
fn intake(bytes: &[u8], profile: Profile) -> Result<Canon, DecodeError> {
    let decoded = decode(bytes, profile, &Reader::current())?;
    Ok(Canon::try_from(&decoded.canon).expect("a validated Canon has no empty label"))
}

/// The artifact's kernel Canon is the one the refresh experiments build by
/// hand, and enforcing it converges and consumes the refresh, in both
/// profiles.
#[test]
fn a_decoded_artifact_converges_on_the_host() {
    for profile in Profile::ALL {
        let canon = intake(&obligation_artifact(profile), profile).unwrap();
        assert_eq!(canon, obligation_canon(), "{profile:?}");
        let mut sim = Sim::new(refresh_host());
        sim.enforce(plan("p1", 1, canon, 3));
        assert_eq!(sim.run(), RunOutcome::Converged, "{profile:?}");
        assert_eq!(sim.host.file(&p(CONF)), Some(d(NEW)));
        assert!(sim.refresh_consumed());
    }
}

/// The typed builder and the decoded artifact reach the same kernel Canon:
/// the authoring path and the decoding path are one validator.
#[test]
fn authoring_and_decoding_reach_the_same_kernel_canon() {
    use nomos_canon::model::{RelationKind, Requirement, ServiceRequirement};
    use nomos_core::condition::{Content, FileCondition};
    let built = CanonBuilder::new("refresh")
        .resource(
            CONF,
            Requirement::File(FileCondition::Present {
                content: Content::Exactly(d(NEW)),
            }),
            &["file:/etc/svc.conf"],
            &[],
        )
        .resource(
            SVC,
            Requirement::Service(ServiceRequirement::Running),
            &["systemd:svc"],
            &[],
        )
        .relate(CONF, RelationKind::OnChange, SVC)
        .build()
        .unwrap();
    for profile in Profile::ALL {
        assert_eq!(encode(&built, profile), obligation_artifact(profile));
        assert_eq!(
            Canon::try_from(&built).unwrap(),
            intake(&obligation_artifact(profile), profile).unwrap()
        );
    }
}

/// The refresh artifact with one more resource, of a kind no reader here
/// knows, in its place by path, as a newer writer would write it.
fn unknown_kind_artifact(profile: Profile) -> Vec<u8> {
    let Value::Map(mut top) = profile.decode_value(&obligation_artifact(profile)).unwrap() else {
        panic!()
    };
    for (k, v) in &mut top {
        if k == "resources"
            && let Value::Array(items) = v
        {
            items.insert(
                1,
                Value::Map(vec![
                    (
                        "path".into(),
                        Value::Text("/proc/sys/net/ipv4/ip_forward".into()),
                    ),
                    (
                        "spec".into(),
                        Value::Map(vec![
                            ("kind".into(), Value::Text("sysctl".into())),
                            ("state".into(), Value::Text("value".into())),
                        ]),
                    ),
                    ("keys".into(), Value::Array(vec![])),
                    ("disrupts".into(), Value::Array(vec![])),
                ]),
            );
        }
    }
    profile.encode_value(&Value::Map(top))
}

/// The strict intake refuses the unknown kind, so no Plan exists and the
/// host sees no effect; the lenient intake, which drops what it does not
/// know, enforces the rest and the host is changed. The harness tells the
/// two apart by the effects executed.
#[test]
fn an_unknown_mutating_kind_is_refused_before_any_effect() {
    for profile in Profile::ALL {
        let bytes = unknown_kind_artifact(profile);
        let strict = intake(&bytes, profile);
        assert_eq!(
            strict.err(),
            Some(DecodeError::UnsupportedKind { resource: 1 })
        );
        let sim = Sim::new(refresh_host());
        assert!(sim.host.executions().is_empty());
        assert_eq!(
            sim.host,
            refresh_host(),
            "the strict intake left the host untouched"
        );

        // The rival: skip the unknown resource and read the rest.
        let Value::Map(mut top) = profile.decode_value(&bytes).unwrap() else {
            panic!()
        };
        for (k, v) in &mut top {
            if k == "resources"
                && let Value::Array(items) = v
            {
                items.retain(|r| {
                    !matches!(
                        r.get("spec").and_then(|s| s.get("kind")),
                        Some(Value::Text(k)) if k == "sysctl"
                    )
                });
            }
        }
        let lenient = intake(&profile.encode_value(&Value::Map(top)), profile).unwrap();
        let mut sim = Sim::new(refresh_host());
        sim.enforce(plan("p1", 1, lenient, 3));
        sim.run();
        assert!(
            !sim.host.executions().is_empty(),
            "the negative control did not fire: the lenient intake issued no effect"
        );
        assert_ne!(sim.host, refresh_host());
    }
}
