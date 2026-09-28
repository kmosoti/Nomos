//! Experiment `bounded-convergence` (grounding plan, `05-transition-kernel`):
//! does the loop report the right outcome at the bound without hiding
//! effects in flight, and does it keep Indeterminate apart from Converged
//! and Failed?
//!
//! Scripts against the mock: converge on the last permitted execution,
//! never converge against a writer that restores the old content, never
//! converge against a writer whose content never repeats, complete late,
//! deny a read, and change unrelated telemetry on every observation. The
//! negative control is `final-check`.

mod support;

use nomos_app::kernel::{Canon, Nonconvergence, RunOutcome};
use nomos_core::action::Stage;
use nomos_core::assessment::Reason;
use nomos_core::observation::CollectionFailure;
use nomos_substrate_mock::MockHost;
use nomos_warp::graph::EdgeKind;
use support::*;

const F: &str = "/etc/app.conf";

fn one_file() -> Canon {
    Canon::new(vec![file(F, d(NEW), &["file:/etc/app.conf"])], vec![])
}

fn drifted() -> MockHost {
    let mut host = MockHost::new();
    host.write(&p(F), d(OLD));
    host
}

/// Runs `sim` to the end, calling `interfere` after every delivery with
/// the number of verified writes so far.
fn run_with(sim: &mut Sim, mut interfere: impl FnMut(&mut Sim, usize)) -> RunOutcome {
    let mut verified = 0;
    for _ in 0..10_000 {
        if let Some(outcome) = sim.outcome()
            && sim.queue.is_empty()
        {
            return outcome;
        }
        if !sim.next() {
            sim.tick(1);
        }
        let total = sim
            .decisions
            .iter()
            .flat_map(|d| &d.events)
            .filter(|e| {
                matches!(
                    e,
                    nomos_app::kernel::Event::ActionAdvanced {
                        stage: Stage::Succeeded { .. },
                        ..
                    }
                )
            })
            .count();
        if total > verified {
            verified = total;
            interfere(sim, verified);
        }
    }
    panic!("the run did not end");
}

/// The exit criterion, the negative control `final-check`, and semantic
/// mutant `SM-TRANSITION-006`'s named test: the execution the bound
/// permits last converges the host, and the run says Converged.
#[test]
fn the_final_permitted_execution_converges() {
    for bound in 1..=3 {
        let mut sim = Sim::new(drifted());
        sim.enforce(plan("p", 1, one_file(), bound));
        assert_eq!(sim.run(), RunOutcome::Converged, "bound {bound}");
        assert_eq!(sim.host.executions().len(), 1);
    }
    // With the bound at one and a writer that undoes the first repair, the
    // second observation is the last one and still finds a Variance.
    let mut sim = Sim::new(drifted());
    sim.enforce(plan("p", 1, one_file(), 1));
    let outcome = run_with(&mut sim, |sim, _| sim.host.write(&p(F), d(9)));
    assert_eq!(outcome, RunOutcome::NonConvergent(Nonconvergence::Bound));
}

#[test]
fn a_bound_of_zero_observes_once_and_executes_nothing() {
    let mut sim = Sim::new(drifted());
    sim.enforce(plan("p", 1, one_file(), 0));
    assert_eq!(sim.run(), RunOutcome::NonConvergent(Nonconvergence::Bound));
    assert!(sim.host.executions().is_empty());
}

#[test]
fn a_writer_that_restores_the_old_content_is_oscillation() {
    let mut sim = Sim::new(drifted());
    sim.enforce(plan("p", 1, one_file(), 5));
    let outcome = run_with(&mut sim, |sim, _| sim.host.write(&p(F), d(OLD)));
    assert_eq!(
        outcome,
        RunOutcome::NonConvergent(Nonconvergence::Oscillation)
    );
    assert_eq!(sim.host.executions().len(), 1);
}

#[test]
fn a_writer_that_never_repeats_reaches_the_bound() {
    let mut sim = Sim::new(drifted());
    sim.enforce(plan("p", 1, one_file(), 3));
    let outcome = run_with(&mut sim, |sim, n| sim.host.write(&p(F), d(10 + n as u8)));
    assert_eq!(outcome, RunOutcome::NonConvergent(Nonconvergence::Bound));
    assert_eq!(sim.host.executions().len(), 3);
}

/// Telemetry outside the Canon changes on every observation; the
/// fingerprint ignores it, so the oscillation is still seen.
#[test]
fn unrelated_telemetry_does_not_hide_oscillation() {
    let mut sim = Sim::new(drifted());
    sim.telemetry = Some(p("/var/telemetry"));
    sim.enforce(plan("p", 1, one_file(), 5));
    let outcome = run_with(&mut sim, |sim, _| sim.host.write(&p(F), d(OLD)));
    assert_eq!(
        outcome,
        RunOutcome::NonConvergent(Nonconvergence::Oscillation)
    );
}

/// A slow completion is not oscillation: the host takes most of the
/// receipt deadline, and the loop waits instead of observing.
#[test]
fn a_slow_completion_is_not_oscillation() {
    let mut sim = Sim::new(drifted());
    sim.enforce(plan("p", 1, one_file(), 3));
    loop {
        match sim.queue.front() {
            Some(Item::Execute { .. }) => break,
            Some(_) => {
                sim.next();
            }
            None => panic!("no write issued"),
        }
    }
    let held = sim.queue.pop_front().unwrap();
    sim.tick(4);
    sim.deliver(held);
    assert_eq!(sim.run(), RunOutcome::Converged);
}

#[test]
fn a_denied_read_with_nothing_else_to_do_is_indeterminate() {
    let mut host = drifted();
    host.write(&p(F), d(NEW));
    host.deny(&p(F), true);
    let mut sim = Sim::new(host);
    sim.enforce(plan("p", 1, one_file(), 3));
    assert_eq!(
        sim.run(),
        RunOutcome::Indeterminate(vec![(
            p(F),
            Reason::CollectionFailed(CollectionFailure::PermissionDenied)
        )])
    );
    assert!(sim.host.executions().is_empty());
}

/// A Variance whose prerequisite is unknown is not repaired, and the run is
/// Indeterminate rather than NonConvergent: another iteration would observe
/// the same unknown.
#[test]
fn a_variance_blocked_behind_an_unknown_is_indeterminate() {
    let mut host = drifted();
    host.deny(&p("/etc/app.d"), true);
    let mut sim = Sim::new(host);
    let canon = Canon::new(
        vec![file(F, d(NEW), &[]), file("/etc/app.d", d(3), &[])],
        vec![edge("/etc/app.d", F, EdgeKind::Requires)],
    );
    sim.enforce(plan("p", 1, canon, 3));
    assert_eq!(
        sim.run(),
        RunOutcome::Indeterminate(vec![(
            p("/etc/app.d"),
            Reason::CollectionFailed(CollectionFailure::PermissionDenied)
        )])
    );
    assert!(sim.host.executions().is_empty());
}

/// Converged is never reported with an effect unsettled or an Obligation
/// pending: the run that converges holds no reservation.
#[test]
fn converged_holds_nothing() {
    let mut sim = Sim::new(refresh_host());
    sim.enforce(plan("p", 1, obligation_canon(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    assert!(sim.snapshot.effects().is_empty());
    assert!(sim.snapshot.obligations().is_empty());
    assert_eq!(
        sim.snapshot.released().len(),
        2,
        "the write and the refresh"
    );
}

/// N3, the fixed point of reconciliation.md: enforcing a Canon the host
/// already satisfies, with nothing owed and nothing in flight, mutates
/// nothing.
#[test]
fn re_enforcing_a_converged_canon_mutates_nothing() {
    let mut sim = Sim::new(refresh_host());
    sim.enforce(plan("p1", 1, obligation_canon(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    let executed = sim.host.executions().len();
    sim.enforce(plan("p2", 2, obligation_canon(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    assert_eq!(sim.host.executions().len(), executed);
}
