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

// ---------------------------------------------------------------------------
// Tests the mutation run of the transition kernel found missing.

/// The first two inputs of a run: the Plan, then a fresh observation that
/// finds the file drifted and the service running, which dispatches the
/// write at `at`.
fn dispatched(at: u64) -> (KernelSnapshot, nomos_core::effect::EffectKey) {
    let (snapshot, log, _) = script(vec![
        Input::Tick(Instant(at)),
        Input::Enforce(plan("a", 1)),
        Input::Observed(vec![seen("/etc/c", d(1), at), seen("/run/s", d(0), at)]),
    ]);
    let key = log
        .iter()
        .find_map(|e| match e {
            Event::EffectRequested { key, .. } => Some(key.clone()),
            _ => None,
        })
        .unwrap();
    (snapshot, key)
}

fn stage_of(snapshot: &KernelSnapshot, path: &str) -> Option<nomos_core::action::Stage> {
    snapshot.round().map(|r| r.actions[&p(path)].stage.clone())
}

/// An Observation collected before the run asked for one says nothing
/// about the host now, even if it arrives late.
#[test]
fn a_stale_observation_decides_nothing() {
    let (accepted, _, _) = script(vec![Input::Tick(Instant(5)), Input::Enforce(plan("a", 1))]);
    let stale = step(
        &accepted,
        Input::Observed(vec![seen("/etc/c", d(1), 4), seen("/run/s", d(0), 4)]),
    );
    assert_eq!(stale.snapshot, accepted);
    let fresh = step(
        &accepted,
        Input::Observed(vec![seen("/etc/c", d(1), 5), seen("/run/s", d(0), 5)]),
    );
    assert!(fresh.snapshot.round().is_some());
}

/// An observation that leaves a managed resource out decides nothing.
#[test]
fn a_partial_observation_decides_nothing() {
    let (accepted, _, _) = script(vec![Input::Enforce(plan("a", 1))]);
    let partial = step(&accepted, Input::Observed(vec![seen("/etc/c", d(1), 0)]));
    assert_eq!(partial.snapshot, accepted);
}

/// An acceptance settles nothing, and an Action accepted and never heard
/// from again still times out, with its effect unsettled and reserved.
#[test]
fn an_accepted_action_still_times_out_and_stays_reserved() {
    let (snapshot, key) = dispatched(0);
    let accepted = step(&snapshot, Input::Receipt(key.clone(), Receipt::Accepted));
    assert!(!accepted.snapshot.effects()[&key].settlement.is_settled());
    let late = step(&accepted.snapshot, Input::Tick(Instant(6)));
    assert!(late.events.contains(&Event::ActionAdvanced {
        resource: p("/etc/c"),
        stage: nomos_core::action::Stage::TimedOut,
        deadline: None,
    }));
    assert!(!late.snapshot.effects()[&key].settlement.is_settled());
}

/// A receipt that arrives after the timeout settles the effect, and the
/// record says so rather than calling it ignored.
#[test]
fn a_late_receipt_settles_and_is_not_ignored() {
    let (snapshot, key) = dispatched(0);
    let timed_out = step(&snapshot, Input::Tick(Instant(6)));
    let late = step(
        &timed_out.snapshot,
        Input::Receipt(key.clone(), Receipt::Completed { changed: true }),
    );
    assert!(late.events.contains(&Event::EffectSettled {
        key: key.clone(),
        by: nomos_core::effect::SettledBy::Receipt,
    }));
    assert!(
        !late
            .events
            .iter()
            .any(|e| matches!(e, Event::ReceiptIgnored { .. }))
    );
}

/// Verification has its own deadline, counted from the completion: an
/// effect completed just before its receipt deadline can still be verified
/// after that deadline.
#[test]
fn verification_has_its_own_deadline() {
    let (snapshot, key) = dispatched(0);
    let at_four = step(&snapshot, Input::Tick(Instant(4)));
    let completed = step(
        &at_four.snapshot,
        Input::Receipt(key, Receipt::Completed { changed: true }),
    );
    let at_six = step(&completed.snapshot, Input::Tick(Instant(6)));
    assert!(matches!(
        stage_of(&at_six.snapshot, "/etc/c"),
        Some(nomos_core::action::Stage::Verifying { .. })
    ));
    let verified = step(
        &at_six.snapshot,
        Input::Observed(vec![seen("/etc/c", d(2), 6)]),
    );
    assert!(verified.events.iter().any(|e| matches!(
        e,
        Event::ActionAdvanced {
            stage: nomos_core::action::Stage::Succeeded { .. },
            ..
        }
    )));
}

/// A restart loses the verification in flight, not the effect: an Action
/// that was Verifying is observed again.
#[test]
fn recovery_observes_again_what_was_verifying() {
    let (snapshot, key) = dispatched(0);
    let completed = step(
        &snapshot,
        Input::Receipt(key, Receipt::Completed { changed: true }),
    );
    let recovered = step(&completed.snapshot, Input::Recovered);
    assert!(
        recovered
            .effects
            .contains(&EffectRequest::Observe(vec![p("/etc/c")]))
    );
    assert!(matches!(
        stage_of(&recovered.snapshot, "/etc/c"),
        Some(nomos_core::action::Stage::Verifying { .. })
    ));
}

/// The refresh reads the file its `on_change` source writes, so its
/// reservation holds the file's key as well as its own (ADR 0009 note).
#[test]
fn a_refresh_holds_the_keys_of_what_it_reads() {
    let key = |k: &str| nomos_warp::graph::ConflictKey::new(k).unwrap();
    let mut keyed = plan("a", 1);
    let mut conf = managed("/etc/c", Kind::File, Content::Exactly(d(2)));
    conf.keys = [key("file:c")].into_iter().collect();
    let mut svc = managed("/run/s", Kind::Service, Content::Any);
    svc.keys = [key("svc")].into_iter().collect();
    keyed.canon = Canon::new(
        vec![conf, svc],
        vec![Edge::new(p("/etc/c"), p("/run/s"), EdgeKind::OnChange)],
    );
    let (snapshot, log, _) = script(vec![
        Input::Enforce(keyed),
        Input::Observed(vec![seen("/etc/c", d(1), 0), seen("/run/s", d(0), 0)]),
    ]);
    let write = log
        .iter()
        .find_map(|e| match e {
            Event::EffectRequested { key, .. } => Some(key.clone()),
            _ => None,
        })
        .unwrap();
    let completed = step(
        &snapshot,
        Input::Receipt(write, Receipt::Completed { changed: true }),
    );
    let verified = step(
        &completed.snapshot,
        Input::Observed(vec![seen("/etc/c", d(2), 0)]),
    );
    let refresh = verified
        .events
        .iter()
        .find_map(|e| match e {
            Event::EffectRequested { key, record } if key.resource() == &p("/run/s") => {
                Some(record.keys.clone())
            }
            _ => None,
        })
        .expect("the refresh was dispatched");
    assert_eq!(refresh, [key("file:c"), key("svc")].into_iter().collect());
}

/// A complete, fresh observation of a one-resource Canon decides the
/// iteration.
#[test]
fn a_complete_observation_decides() {
    let mut one = plan("a", 1);
    one.canon = Canon::new(
        vec![managed("/etc/c", Kind::File, Content::Exactly(d(2)))],
        vec![],
    );
    let (snapshot, _, _) = script(vec![
        Input::Enforce(one),
        Input::Observed(vec![seen("/etc/c", d(1), 0)]),
    ]);
    assert!(snapshot.round().is_some());
}

/// N9's freshness clause: a budget snapshot within its maximum age admits
/// a disruptive Action, and one past it does not.
#[test]
fn budget_freshness_decides_disruptive_admission() {
    let admitted = |observed_at: u64| {
        let mut disruptive = plan("a", 1);
        let mut conf = managed("/etc/c", Kind::File, Content::Exactly(d(2)));
        conf.disrupts = [nomos_warp::budget::Node::new("n").unwrap()]
            .into_iter()
            .collect();
        disruptive.canon = Canon::new(vec![conf], vec![]);
        disruptive.policy.budgets = vec![nomos_warp::budget::Budget::new(
            [nomos_warp::budget::Node::new("n").unwrap()]
                .into_iter()
                .collect(),
            1,
        )];
        disruptive.policy.budget_observed_at = Instant(observed_at);
        disruptive.policy.budget_max_age = 3;
        let (snapshot, _, _) = script(vec![
            Input::Tick(Instant(10)),
            Input::Enforce(disruptive),
            Input::Observed(vec![seen("/etc/c", d(1), 10)]),
        ]);
        !snapshot.effects().is_empty()
    };
    assert!(admitted(8), "two old, within three");
    assert!(!admitted(6), "four old, past three");
}
