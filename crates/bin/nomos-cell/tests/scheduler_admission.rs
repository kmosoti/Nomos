//! Experiment `scheduler-admission` (grounding plan, `05-transition-kernel`):
//! is serialized, reservation-based greedy selection safe under declared
//! footprints, budgets, and involuntary failures, with reservations held
//! until settlement?
//!
//! Named scripts for the negative controls, then a property over generated
//! interleavings of delivery, reordering, loss, duplication, time, crashes,
//! and supersession, checked after every step. The negative controls are
//! `budget-not-world`, a reservation released at timeout (which must allow
//! an overlap, and the corrected rule must not), and two admissions against
//! one stale snapshot.

mod support;

use std::collections::BTreeSet;

use nomos_app::kernel::{Canon, Event, Input, Plan, RunOutcome};
use nomos_core::action::Stage;
use nomos_core::observation::Instant;
use nomos_substrate_mock::MockHost;
use nomos_warp::budget::{Budget, Node};
use nomos_warp::graph::EdgeKind;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use support::*;

fn node(name: &str) -> Node {
    Node::new(name).unwrap()
}

fn drifted(paths: &[&str]) -> MockHost {
    let mut host = MockHost::new();
    for path in paths {
        host.write(&p(path), d(OLD));
    }
    host
}

fn dispatched(sim: &Sim) -> Vec<nomos_core::resource::ResourceKey> {
    sim.decisions
        .iter()
        .flat_map(|d| &d.events)
        .filter_map(|e| match e {
            Event::ActionDispatched { resource, .. } => Some(resource.clone()),
            _ => None,
        })
        .collect()
}

/// The exit criterion, and semantic mutant `SM-TRANSITION-005`'s named
/// test: A and B share a key. A is dispatched, its receipt never comes, and
/// it times out with its effect unsettled. B is not admitted until A's
/// settle-by instant passes, although A is terminal.
#[test]
fn a_timeout_keeps_its_reservation() {
    let canon = Canon::new(
        vec![file("/a", d(NEW), &["dpkg"]), file("/b", d(NEW), &["dpkg"])],
        vec![],
    );
    let mut sim = Sim::new(drifted(&["/a", "/b"]));
    sim.enforce(plan("p", 1, canon, 3));
    while !matches!(sim.queue.front(), Some(Item::Execute { .. }) | None) {
        sim.next();
    }
    let held = sim.queue.pop_front().expect("A was dispatched");
    assert_eq!(dispatched(&sim), vec![k("/a")]);
    for _ in 0..19 {
        sim.tick(1);
        while sim.next() {}
        let a = &sim.snapshot.round().unwrap().actions[&k("/a")];
        if sim.now >= 5 {
            assert_eq!(a.stage, Stage::TimedOut, "at {}", sim.now);
        }
        assert_eq!(dispatched(&sim), vec![k("/a")], "B admitted at {}", sim.now);
    }
    sim.tick(1);
    assert_eq!(
        dispatched(&sim),
        vec![k("/a"), k("/b")],
        "B admitted once A is Settled"
    );
    sim.deliver(held);
    assert_eq!(
        sim.run(),
        RunOutcome::Failed {
            failed: vec![],
            unknown: vec![k("/a")]
        }
    );
}

fn restarts(budget_age: u64) -> Plan {
    let mut canon_resources = Vec::new();
    for (path, n) in [("/run/a", "n1"), ("/run/b", "n2")] {
        let mut s = service(path, &[]);
        s.disrupts = [node(n)].into_iter().collect();
        canon_resources.push(s);
    }
    let mut plan = plan("p", 1, Canon::new(canon_resources, vec![]), 3);
    plan.policy.budgets = vec![Budget::new(
        [node("n1"), node("n2"), node("n3")].into_iter().collect(),
        1,
    )];
    plan.policy.budget_max_age = budget_age;
    plan
}

fn stopped_services() -> MockHost {
    let mut host = MockHost::new();
    for (status, config) in [("/run/a", "/etc/a"), ("/run/b", "/etc/b")] {
        host.write(&p(config), d(OLD));
        host.add_service(
            p(status),
            nomos_substrate_mock::Service {
                config: p(config),
                loaded: None,
                active: false,
                loaded_path: None,
            },
        );
    }
    host
}

/// N9: two restarts in a rack that may lose one node at a time are admitted
/// one after the other, never together, although both are Ready at once.
#[test]
fn two_disruptive_restarts_are_admitted_one_at_a_time() {
    let mut sim = Sim::new(stopped_services());
    sim.enforce(restarts(u64::MAX));
    let mut most = 0;
    for _ in 0..1000 {
        most = most.max(sim.snapshot.effects().len());
        if sim.outcome().is_some() && sim.queue.is_empty() {
            break;
        }
        if !sim.next() {
            sim.tick(1);
        }
    }
    assert_eq!(sim.outcome(), Some(RunOutcome::Converged));
    assert_eq!(most, 1);
    assert_eq!(dispatched(&sim).len(), 2);
}

/// A budget snapshot older than the policy allows admits nothing that
/// disrupts; a newer Plan with a fresh snapshot does.
#[test]
fn a_stale_budget_snapshot_admits_nothing_disruptive() {
    let mut sim = Sim::new(stopped_services());
    sim.tick(10);
    sim.enforce(restarts(3));
    for _ in 0..50 {
        if !sim.next() {
            sim.tick(1);
        }
    }
    assert!(dispatched(&sim).is_empty());
    assert!(sim.outcome().is_none(), "waiting, not failed");
    let mut fresh = restarts(3);
    fresh.id = nomos_core::plan::PlanId::new("q").unwrap();
    fresh.generation = nomos_core::plan::Generation(2);
    fresh.policy.budget_observed_at = Instant(sim.now);
    sim.enforce(fresh);
    assert_eq!(sim.run(), RunOutcome::Converged);
}

/// Supersession drains (ADR 0010 note): the newer Plan's acceptance moves
/// the fence, the old Plan's undispatched Action is Cancelled and never
/// issued, and the newer Plan observes only once the old in-flight effect
/// is Settled.
#[test]
fn a_superseding_plan_drains_before_it_acts() {
    let canon = || {
        Canon::new(
            vec![file("/a", d(NEW), &[]), file("/b", d(NEW), &[])],
            vec![edge("/a", "/b", EdgeKind::Requires)],
        )
    };
    let mut sim = Sim::new(drifted(&["/a", "/b"]));
    sim.enforce(plan("old", 1, canon(), 3));
    while !matches!(sim.queue.front(), Some(Item::Execute { .. }) | None) {
        sim.next();
    }
    let held = sim.queue.pop_front().expect("A was dispatched");
    sim.enforce(plan("new", 2, canon(), 3));
    let old_b = sim
        .decisions
        .iter()
        .flat_map(|d| &d.events)
        .any(|e| matches!(e, Event::ActionAdvanced { resource, stage: Stage::Cancelled, .. } if *resource == k("/b")));
    assert!(old_b, "the old Plan's B was Cancelled");
    assert!(
        sim.snapshot.run().unwrap().phase == nomos_app::kernel::Phase::Awaiting,
        "the new Plan waits for the old effect"
    );
    sim.deliver(held);
    assert_eq!(sim.run(), RunOutcome::Converged);
    let old_keys: Vec<_> = sim
        .issued
        .iter()
        .filter(|k| k.plan().as_str() == "old")
        .collect();
    assert_eq!(old_keys.len(), 1, "only the old A was ever issued");
}

// ---------------------------------------------------------------------------
// Generated interleavings

#[derive(Debug, Clone)]
enum Op {
    Deliver(usize),
    Drop(usize),
    Duplicate(usize),
    Tick(u64),
    Crash,
    Supersede,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        8 => (0usize..8).prop_map(Op::Deliver),
        1 => (0usize..8).prop_map(Op::Drop),
        1 => (0usize..8).prop_map(Op::Duplicate),
        3 => (1u64..8).prop_map(Op::Tick),
        1 => Just(Op::Crash),
        1 => Just(Op::Supersede),
    ]
}

/// Three files with generated keys and disruption, and the refresh pair.
type Shape = Vec<(BTreeSet<u8>, BTreeSet<u8>)>;

fn shape() -> impl Strategy<Value = Shape> {
    prop::collection::vec(
        (
            prop::collection::btree_set(0u8..2, 0..2),
            prop::collection::btree_set(0u8..3, 0..2),
        ),
        3,
    )
}

fn scenario(shape: &Shape, id: &str, generation: u64) -> Plan {
    let mut resources = Vec::new();
    for (i, (ks, ns)) in shape.iter().enumerate() {
        let names: Vec<String> = ks.iter().map(|k| format!("k{k}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut f = file(&format!("/r/{i}"), d(NEW), &refs);
        f.disrupts = ns.iter().map(|n| node(&format!("n{n}"))).collect();
        resources.push(f);
    }
    resources.push(file(CONF, d(NEW), &["file:/etc/svc.conf"]));
    resources.push(service(SVC, &["systemd:svc"]));
    let canon = Canon::new(
        resources,
        vec![
            edge(CONF, SVC, EdgeKind::OnChange),
            edge("/r/0", "/r/1", EdgeKind::Requires),
        ],
    );
    let mut plan = plan(id, generation, canon, 4);
    plan.policy.budgets = vec![Budget::new(
        [node("n0"), node("n1"), node("n2")].into_iter().collect(),
        1,
    )];
    plan.policy.capacity = 3;
    plan
}

fn host() -> MockHost {
    let mut host = refresh_host();
    for i in 0..3 {
        host.write(&p(&format!("/r/{i}")), d(OLD));
    }
    host
}

/// The safety properties, after every step.
fn check(sim: &Sim, last: &nomos_app::kernel::Decision) -> Result<(), TestCaseError> {
    // No two reservations share a key.
    let effects: Vec<_> = sim.snapshot.effects().values().collect();
    for (i, a) in effects.iter().enumerate() {
        for b in &effects[i + 1..] {
            prop_assert!(a.keys.is_disjoint(&b.keys), "overlapping reservations");
        }
    }
    // No budget oversubscribed: at most one node of the rack disrupted.
    let disrupted: BTreeSet<&Node> = effects.iter().flat_map(|e| &e.disrupts).collect();
    prop_assert!(disrupted.len() <= 1, "budget oversubscribed: {disrupted:?}");
    // Work the host may still perform is reserved in the kernel.
    for item in &sim.queue {
        if let Item::Execute { request, .. } = item
            && Instant(sim.now) < request.settle_by
        {
            prop_assert!(
                sim.snapshot.effects().contains_key(&request.key),
                "live host work without a reservation: {:?}",
                request.key
            );
        }
    }
    // N4, lifecycle side: an Action is dispatched only when every `requires`
    // source has Succeeded or is a satisfaction anchor.
    if let Some(round) = sim.snapshot.round() {
        for event in &last.events {
            if let Event::ActionDispatched { resource, .. } = event {
                for e in round.graph.edges_into(resource) {
                    if e.kind() == EdgeKind::Requires {
                        let met = match round.actions.get(e.source()) {
                            Some(a) => matches!(a.stage, Stage::Succeeded { .. }),
                            None => true,
                        };
                        prop_assert!(met, "{resource} dispatched before {}", e.source());
                    }
                }
            }
        }
    }
    // Converged is sound: the host satisfies the Canon, the service loaded
    // what is on disk, nothing is owed, and nothing can still change.
    if sim.outcome() == Some(RunOutcome::Converged) {
        prop_assert!(sim.snapshot.obligations().is_empty());
        prop_assert!(
            sim.snapshot
                .effects()
                .values()
                .all(|e| e.settlement.is_settled())
        );
        for i in 0..3 {
            prop_assert_eq!(sim.host.file(&p(&format!("/r/{i}"))), Some(d(NEW)));
        }
        prop_assert_eq!(sim.host.file(&p(CONF)), Some(d(NEW)));
        prop_assert!(sim.refresh_consumed(), "Converged with the refresh lost");
    }
    Ok(())
}

/// The property: under every generated interleaving the safety properties
/// hold after every step, and once the faults stop, the Canon converges.
#[test]
fn generated_interleavings_never_overlap_oversubscribe_or_lie() {
    let seed = [
        0x4e, 0x35, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0,
    ];
    let config = Config {
        cases: 256,
        failure_persistence: None,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &seed));
    runner
        .run(
            &(shape(), prop::collection::vec(op(), 0..80)),
            |(shape, ops)| {
                let mut sim = Sim::new(host());
                sim.enforce(scenario(&shape, "p1", 1));
                let (mut crashes, mut supersessions) = (0, 0);
                for op in ops {
                    match op {
                        Op::Deliver(i) if !sim.queue.is_empty() => {
                            let i = i % sim.queue.len();
                            let item = sim.queue.remove(i).unwrap();
                            sim.deliver(item);
                        }
                        Op::Drop(i) if !sim.queue.is_empty() => {
                            let i = i % sim.queue.len();
                            sim.queue.remove(i);
                        }
                        Op::Duplicate(i) if !sim.queue.is_empty() => {
                            let i = i % sim.queue.len();
                            if let Some(Item::Deliver(input)) = sim.queue.get(i).cloned() {
                                sim.queue.push_back(Item::Deliver(input));
                            }
                        }
                        Op::Tick(dt) => sim.tick(dt),
                        Op::Crash if crashes < 2 => {
                            crashes += 1;
                            sim.crash();
                        }
                        Op::Supersede if supersessions < 1 => {
                            supersessions += 1;
                            sim.enforce(scenario(&shape, "p2", 2));
                        }
                        _ => {}
                    }
                    if let Some(last) = sim.decisions.last().cloned() {
                        check(&sim, &last)?;
                    }
                }
                // The faults stop: everything settles, and a fresh Plan runs.
                sim.quiesce();
                if sim.outcome() != Some(RunOutcome::Converged) {
                    sim.enforce(scenario(&shape, "p3", 3));
                    let outcome = sim.run();
                    prop_assert_eq!(outcome, RunOutcome::Converged);
                }
                if let Some(last) = sim.decisions.last().cloned() {
                    check(&sim, &last)?;
                }
                let _ = Input::Recovered;
                Ok(())
            },
        )
        .unwrap();
}
