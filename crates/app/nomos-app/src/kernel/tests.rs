//! Scripted inputs, no host: the kernel's handling of authority, and one
//! Obligation path the mock cannot produce. The scenario experiments run
//! against the mock in `crates/bin/nomos-cell/tests/`.

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use nomos_core::condition::{Condition, Content, FileCondition};
use nomos_core::effect::{EffectRequest, Receipt};
use nomos_core::observation::{
    Collection, CollectorId, FileEvidence, Instant, Observation, Provenance, Window,
};
use nomos_core::plan::{FenceError, Generation, PlanId};
use nomos_core::resource::{Digest, ResourcePath};
use nomos_warp::graph::{Edge, EdgeKind};

use super::*;

fn p(text: &str) -> ResourcePath {
    ResourcePath::new(text).unwrap()
}

fn d(n: u8) -> Digest {
    Digest::from_bytes([n; 32])
}

fn managed(path: &str, kind: Kind, content: Content) -> Managed {
    Managed {
        condition: Condition::file(p(path), FileCondition::Present { content }),
        kind,
        keys: BTreeSet::new(),
        disrupts: BTreeSet::new(),
    }
}

fn canon() -> Canon {
    Canon::new(
        vec![
            managed("/etc/c", Kind::File, Content::Exactly(d(2))),
            managed("/run/s", Kind::Service, Content::Any),
        ],
        vec![Edge::new(p("/etc/c"), p("/run/s"), EdgeKind::OnChange)],
    )
}

fn plan(id: &str, generation: u64) -> Plan {
    Plan {
        id: PlanId::new(id).unwrap(),
        generation: Generation(generation),
        canon: canon(),
        bound: 3,
        policy: Policy::new(4, 5, 5, 20),
        expires: None,
    }
}

fn seen(path: &str, digest: Digest, at: u64) -> Observation {
    Observation::file(
        p(path),
        Collection::Collected(FileEvidence::Present { digest, size: 1 }),
        Provenance::new(
            CollectorId::new("script").unwrap(),
            Window::new(Instant(at), Instant(at)).unwrap(),
        ),
    )
}

/// Steps every input in turn and checks that the snapshot is the fold of
/// every Event so far.
fn script(inputs: Vec<Input>) -> (KernelSnapshot, Vec<Event>, Vec<Decision>) {
    let mut snapshot = KernelSnapshot::new();
    let mut log = Vec::new();
    let mut decisions = Vec::new();
    for input in inputs {
        let decision = step(&snapshot, input);
        log.extend(decision.events.iter().cloned());
        snapshot = decision.snapshot.clone();
        assert_eq!(snapshot, replay(&log));
        decisions.push(decision);
    }
    (snapshot, log, decisions)
}

#[test]
fn an_accepted_plan_observes_first() {
    let (snapshot, log, decisions) = script(vec![Input::Enforce(plan("a", 1))]);
    assert!(matches!(log[0], Event::PlanAccepted(_)));
    assert_eq!(log[1], Event::ObservationRequested { since: Instant(0) });
    assert_eq!(
        decisions[0].effects,
        vec![EffectRequest::Observe(vec![p("/etc/c"), p("/run/s")])]
    );
    assert_eq!(
        snapshot.fence().accepted(),
        Some((Generation(1), &PlanId::new("a").unwrap()))
    );
}

/// N5 at acceptance: a stale Plan and a conflicting one are recorded as
/// rejected and change nothing else.
#[test]
fn stale_and_conflicting_plans_change_nothing() {
    let (accepted, _, _) = script(vec![Input::Enforce(plan("a", 5))]);
    for (offered, error) in [
        (
            plan("b", 4),
            FenceError::Stale {
                offered: Generation(4),
                accepted: Generation(5),
            },
        ),
        (
            plan("b", 5),
            FenceError::Conflict {
                generation: Generation(5),
            },
        ),
    ] {
        let decision = step(&accepted, Input::Enforce(offered.clone()));
        assert_eq!(decision.snapshot, accepted);
        assert!(decision.effects.is_empty());
        assert_eq!(
            decision.events,
            vec![Event::PlanRejected {
                plan: offered.id,
                generation: offered.generation,
                reason: Rejection::Fence(error),
            }]
        );
    }
}

#[test]
fn a_redelivered_plan_changes_nothing() {
    let (accepted, _, _) = script(vec![Input::Enforce(plan("a", 5))]);
    let decision = step(&accepted, Input::Enforce(plan("a", 5)));
    assert_eq!(decision.snapshot, accepted);
    assert_eq!(
        decision.events,
        vec![Event::PlanRedelivered {
            plan: PlanId::new("a").unwrap()
        }]
    );
}

#[test]
fn an_expired_or_cyclic_plan_is_rejected() {
    let mut expired = plan("a", 1);
    expired.expires = Some(Instant(3));
    let (snapshot, log, _) = script(vec![Input::Tick(Instant(3)), Input::Enforce(expired)]);
    assert!(snapshot.run().is_none());
    assert!(matches!(
        log.last(),
        Some(Event::PlanRejected {
            reason: Rejection::Expired,
            ..
        })
    ));
    let mut cyclic = plan("b", 1);
    cyclic.canon = Canon::new(
        vec![
            managed("/a", Kind::File, Content::Any),
            managed("/b", Kind::File, Content::Any),
        ],
        vec![
            Edge::new(p("/a"), p("/b"), EdgeKind::After),
            Edge::new(p("/b"), p("/a"), EdgeKind::Requires),
        ],
    );
    let (snapshot, log, _) = script(vec![Input::Enforce(cyclic)]);
    assert!(snapshot.run().is_none());
    assert!(matches!(
        log.last(),
        Some(Event::PlanRejected {
            reason: Rejection::InvalidCanon,
            ..
        })
    ));
}

/// ADR 0009 note: the Obligation a dispatch records is withdrawn when the
/// write is verified unchanged, and the refresh is Skipped. The mock always
/// reports a change for a write that repairs a Variance, so this is
/// scripted.
#[test]
fn a_write_verified_unchanged_withdraws_its_obligation() {
    let (snapshot, log, decisions) = script(vec![
        Input::Enforce(plan("a", 1)),
        Input::Observed(vec![seen("/etc/c", d(1), 0), seen("/run/s", d(0), 0)]),
    ]);
    let recorded: Vec<&Obligation> = log
        .iter()
        .filter_map(|e| match e {
            Event::ObligationRecorded(o) => Some(o),
            _ => None,
        })
        .collect();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].target, p("/run/s"));
    let key = recorded[0].cause.clone();
    let recorded_at = log
        .iter()
        .position(|e| matches!(e, Event::ObligationRecorded(_)))
        .unwrap();
    let requested_at = log
        .iter()
        .position(|e| matches!(e, Event::EffectRequested { .. }))
        .unwrap();
    assert!(recorded_at < requested_at, "recorded before the change");
    assert!(matches!(
        decisions[1].effects.last(),
        Some(EffectRequest::Apply(_))
    ));
    let completed = step(
        &snapshot,
        Input::Receipt(key.clone(), Receipt::Completed { changed: false }),
    );
    let verified = step(
        &completed.snapshot,
        Input::Observed(vec![seen("/etc/c", d(2), 0)]),
    );
    assert!(
        verified
            .events
            .contains(&Event::ObligationWithdrawn(recorded[0].clone()))
    );
    assert!(verified.snapshot.obligations().is_empty());
    assert!(
        !verified
            .effects
            .iter()
            .any(|e| matches!(e, EffectRequest::Apply(a) if a.key.resource() == &p("/run/s"))),
        "the refresh is Skipped"
    );
}

/// Recovery treats an Action dispatched and not reported complete as an
/// unknown outcome (ADR 0012 §1, N10).
#[test]
fn recovery_times_out_what_was_in_flight() {
    let (snapshot, log, _) = script(vec![
        Input::Enforce(plan("a", 1)),
        Input::Observed(vec![seen("/etc/c", d(1), 0), seen("/run/s", d(0), 0)]),
    ]);
    let recovered = step(&replay(&log), Input::Recovered);
    assert_eq!(replay(&log), snapshot);
    assert!(recovered.events.contains(&Event::ActionAdvanced {
        resource: p("/etc/c"),
        stage: nomos_core::action::Stage::TimedOut,
        deadline: None,
    }));
    assert_eq!(
        recovered.snapshot.outcome(),
        Some(&RunOutcome::Failed {
            failed: vec![],
            unknown: vec![p("/etc/c")]
        })
    );
    assert_eq!(recovered.snapshot.obligations().len(), 1, "kept");
    assert_eq!(recovered.snapshot.effects().len(), 1, "still reserved");
}
