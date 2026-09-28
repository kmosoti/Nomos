//! Experiment `effect-recovery` (grounding plan, `05-transition-kernel`):
//! do run-scoped idempotency keys deduplicate retries within one execution
//! while allowing later repairs?
//!
//! Scripts: duplicate receipts before, during, and after completion; new
//! drift after completion followed by a new run with the same semantic
//! Action; an effect with no readable postcondition; a receipt for nothing
//! the kernel dispatched; a refused request. The negative control is
//! `dedup-scope`.

mod support;

use nomos_app::kernel::{Event, Ignored, Input, RunOutcome};
use nomos_core::effect::{EffectKey, Receipt};
use nomos_core::plan::{Generation, PlanId};
use nomos_substrate_mock::{Fault, MockHost};
use support::*;

const F: &str = "/etc/app.conf";

fn one_file() -> nomos_app::kernel::Canon {
    nomos_app::kernel::Canon::new(vec![file(F, d(NEW), &["file:/etc/app.conf"])], vec![])
}

fn drifted() -> MockHost {
    let mut host = MockHost::new();
    host.write(&p(F), d(OLD));
    host
}

fn ignored(sim: &Sim) -> Vec<Ignored> {
    sim.decisions
        .iter()
        .flat_map(|d| &d.events)
        .filter_map(|e| match e {
            Event::ReceiptIgnored { why, .. } => Some(*why),
            _ => None,
        })
        .collect()
}

/// Every receipt is delivered twice, the copy at the back of the queue, so
/// duplicates arrive before the completion and during verification; then
/// every receipt is delivered a third time, after every effect was
/// released. Nothing executes twice, the run converges, and the late copies
/// change nothing.
#[test]
fn duplicate_receipts_change_nothing() {
    let mut sim = Sim::new(refresh_host());
    sim.enforce(plan("p", 1, obligation_canon(), 3));
    let mut copied = std::collections::BTreeSet::new();
    for _ in 0..10_000 {
        if sim.outcome().is_some() && sim.queue.is_empty() {
            break;
        }
        match sim.queue.pop_front() {
            Some(Item::Deliver(Input::Receipt(key, receipt))) => {
                if copied.insert((key.clone(), receipt)) {
                    sim.queue
                        .push_back(Item::Deliver(Input::Receipt(key.clone(), receipt)));
                }
                sim.step(Input::Receipt(key, receipt));
            }
            Some(item) => sim.deliver(item),
            None => sim.tick(1),
        }
    }
    assert_eq!(sim.outcome(), Some(RunOutcome::Converged));
    assert!(sim.refresh_consumed());
    assert_eq!(sim.host.executions().len(), 2, "one write, one refresh");
    // And once more, after every effect was released.
    let before = sim.snapshot.clone();
    for (key, receipt) in copied {
        sim.step(Input::Receipt(key, receipt));
    }
    assert_eq!(sim.snapshot, before);
    let why = ignored(&sim);
    assert!(why.contains(&Ignored::Late), "{why:?}");
    assert!(why.contains(&Ignored::Duplicate), "{why:?}");
}

/// The negative control `dedup-scope`, and semantic mutant
/// `SM-TRANSITION-008`'s named test: after a converged run, the file drifts
/// again, and a new run repairs it under a new key. A key derived from the
/// Action's content would find the first run's entry and suppress the
/// repair.
#[test]
fn a_later_run_repairs_the_same_drift() {
    let mut sim = Sim::new(drifted());
    sim.enforce(plan("p1", 1, one_file(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    sim.host.write(&p(F), d(OLD));
    sim.enforce(plan("p2", 2, one_file(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    assert_eq!(sim.host.file(&p(F)), Some(d(NEW)));
    let keys: Vec<&EffectKey> = sim.host.executions().iter().map(|(k, _)| k).collect();
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
}

/// The write completes, and then the file cannot be read: verification has
/// nothing fresh to decide on, the Action times out with its outcome
/// unknown, and nothing retries it inside the run (N10).
#[test]
fn an_effect_without_a_readable_postcondition_stays_unresolved() {
    let mut sim = Sim::new(drifted());
    sim.enforce(plan("p", 1, one_file(), 3));
    while sim.host.executions().is_empty() {
        assert!(sim.next());
    }
    sim.host.deny(&p(F), true);
    assert_eq!(
        sim.run(),
        RunOutcome::Failed {
            failed: vec![],
            unknown: vec![p(F)]
        }
    );
    assert_eq!(sim.host.executions().len(), 1);
}

/// A receipt naming nothing the kernel dispatched is recorded and changes
/// nothing (ADR 0006, failure behavior).
#[test]
fn a_receipt_for_an_unknown_execution_changes_nothing() {
    let mut sim = Sim::new(drifted());
    sim.enforce(plan("p", 1, one_file(), 3));
    sim.run();
    let before = sim.snapshot.clone();
    let stranger = EffectKey::new(PlanId::new("x").unwrap(), Generation(9), 0, p(F));
    sim.step(Input::Receipt(
        stranger,
        Receipt::Completed { changed: true },
    ));
    assert_eq!(sim.snapshot, before);
    assert_eq!(ignored(&sim), vec![Ignored::Unknown]);
}

/// A refused request started nothing: the Action is Rejected, a known
/// failure, and the Obligation its dispatch recorded is withdrawn.
#[test]
fn a_refused_request_withdraws_its_obligation() {
    let mut host = refresh_host();
    host.fault(&p(CONF), Fault::Refuse);
    let mut sim = Sim::new(host);
    sim.enforce(plan("p", 1, obligation_canon(), 3));
    assert_eq!(
        sim.run(),
        RunOutcome::Failed {
            failed: vec![p(CONF)],
            unknown: vec![]
        }
    );
    assert!(sim.snapshot.obligations().is_empty());
    assert!(sim.host.executions().is_empty());
}

/// A failed write keeps its Obligation, because a failed write may have
/// changed the file; the Blocked refresh runs in the next run, once the
/// write is repaired (ADR 0009 note).
#[test]
fn a_blocked_refresh_reruns_once_its_trigger_is_repaired() {
    let mut host = refresh_host();
    host.fault(&p(CONF), Fault::Fail);
    let mut sim = Sim::new(host);
    sim.enforce(plan("p1", 1, obligation_canon(), 3));
    assert_eq!(
        sim.run(),
        RunOutcome::Failed {
            failed: vec![p(CONF)],
            unknown: vec![]
        }
    );
    assert_eq!(sim.snapshot.obligations().len(), 1);
    assert!(sim.host.refreshes().is_empty(), "Blocked, not Skipped");
    sim.enforce(plan("p2", 2, obligation_canon(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    assert!(sim.refresh_consumed());
    assert_eq!(sim.host.refreshes().len(), 1);
    assert!(sim.snapshot.obligations().is_empty());
}
