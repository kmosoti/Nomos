//! Experiment `refresh-recovery` (grounding plan, `05-transition-kernel`):
//! which design keeps a service refresh across a crash between the
//! configuration replacement and the restart?
//!
//! Three designs run the same script with a crash at every step boundary:
//! a durable Obligation recorded before the replacement (design 1, ADR 0009
//! §4), the service's loaded revision as a Condition (design 2), and the
//! draft's transient `on_change` (design 3), which is design 1 with the
//! Obligation Events never persisted. The negative control is `lost-refresh`:
//! design 3 must lose the refresh at some crash point, and the harness must
//! see it.

mod support;

use nomos_app::kernel::{Canon, Input, RunOutcome};
use support::*;

/// What one crash point did to one design.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Tally {
    runs: usize,
    lost: usize,
    unnecessary: usize,
    unknown: usize,
    not_converged: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Crash {
    /// The Decision's Events are appended; its effects are never issued.
    BeforeEffects,
    /// The Decision's effects are issued; what comes back is lost.
    AfterEffects,
}

/// Runs the script with a crash at the `n`th Decision, then lets every
/// effect settle and, unless the first run converged, runs a second Plan.
fn crash_at(canon: &Canon, transient: bool, n: usize, mode: Crash) -> (Sim, Tally) {
    let mut sim = Sim::new(refresh_host());
    sim.log.transient = transient;
    let mut crashed = false;
    let mut pending = Some(Input::Enforce(plan("p1", 1, canon.clone(), 3)));
    for _ in 0..10_000 {
        let input = match pending.take() {
            Some(input) => Some(input),
            None => match sim.queue.pop_front() {
                Some(Item::Deliver(input)) => Some(input),
                Some(other) => {
                    sim.deliver(other);
                    None
                }
                None if sim.outcome().is_some() => break,
                None => {
                    sim.now += 1;
                    sim.host.set_time(nomos_core::observation::Instant(sim.now));
                    Some(Input::Tick(nomos_core::observation::Instant(sim.now)))
                }
            },
        };
        let Some(input) = input else { continue };
        if !crashed && sim.decisions.len() + 1 == n {
            crashed = true;
            sim.step_with(input, mode == Crash::AfterEffects);
            sim.crash();
        } else {
            sim.step(input);
        }
    }
    let mut tally = Tally {
        runs: 1,
        ..Tally::default()
    };
    if let Some(RunOutcome::Failed { unknown, .. }) = sim.outcome() {
        tally.unknown += usize::from(!unknown.is_empty());
    }
    if sim.outcome() != Some(RunOutcome::Converged) {
        sim.quiesce();
        sim.enforce(plan("p2", 2, canon.clone(), 3));
        sim.run();
        tally.runs += 1;
    }
    tally.not_converged = usize::from(sim.outcome() != Some(RunOutcome::Converged));
    tally.lost =
        usize::from(sim.outcome() == Some(RunOutcome::Converged) && !sim.refresh_consumed());
    tally.unnecessary = sim.unnecessary_refreshes();
    (sim, tally)
}

fn decisions_without_a_crash(canon: &Canon) -> usize {
    let mut sim = Sim::new(refresh_host());
    sim.enforce(plan("p1", 1, canon.clone(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    sim.decisions.len()
}

fn every_crash_point(canon: &Canon, transient: bool) -> Tally {
    let n = decisions_without_a_crash(canon);
    let mut total = Tally::default();
    for point in 1..=n {
        for mode in [Crash::BeforeEffects, Crash::AfterEffects] {
            let (_, t) = crash_at(canon, transient, point, mode);
            total.runs += t.runs;
            total.lost += t.lost;
            total.unnecessary += t.unnecessary;
            total.unknown += t.unknown;
            total.not_converged += t.not_converged;
        }
    }
    total
}

#[test]
fn without_a_crash_each_design_refreshes_once() {
    for canon in [obligation_canon(), loaded_revision_canon()] {
        let mut sim = Sim::new(refresh_host());
        sim.enforce(plan("p1", 1, canon, 3));
        assert_eq!(sim.run(), RunOutcome::Converged);
        assert!(sim.refresh_consumed());
        assert_eq!(sim.host.refreshes().len(), 1);
        assert!(sim.snapshot.obligations().is_empty());
    }
}

/// The exit criterion, and semantic mutant `SM-TRANSITION-004`'s named
/// test: the configuration is replaced, the process crashes before the
/// receipt arrives, and the refresh still happens.
#[test]
fn a_crash_after_replacement_cannot_lose_the_refresh() {
    let mut sim = Sim::new(refresh_host());
    sim.enforce(plan("p1", 1, obligation_canon(), 3));
    // Deliver until the replacement has been performed by the host.
    while sim.host.file(&p(CONF)) != Some(d(NEW)) {
        assert!(sim.next(), "the replacement was never issued");
    }
    assert!(
        !sim.snapshot.obligations().is_empty(),
        "recorded before the change"
    );
    sim.crash();
    let first = sim.run();
    assert_eq!(
        first,
        RunOutcome::Failed {
            failed: vec![],
            unknown: vec![p(CONF)]
        }
    );
    assert!(!sim.refresh_consumed());
    assert_eq!(
        sim.snapshot.obligations().len(),
        1,
        "the Obligation survived"
    );
    sim.quiesce();
    sim.enforce(plan("p2", 2, obligation_canon(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    assert!(sim.refresh_consumed(), "the refresh was not lost");
    assert!(sim.snapshot.obligations().is_empty());
}

#[test]
fn the_durable_obligation_loses_no_refresh_at_any_crash_point() {
    let tally = every_crash_point(&obligation_canon(), false);
    assert_eq!(tally.lost, 0, "{tally:?}");
    assert_eq!(tally.not_converged, 0, "{tally:?}");
    println!("design 1, durable Obligation: {tally:?}");
}

#[test]
fn the_loaded_revision_condition_loses_no_refresh_at_any_crash_point() {
    let tally = every_crash_point(&loaded_revision_canon(), false);
    assert_eq!(tally.lost, 0, "{tally:?}");
    assert_eq!(tally.not_converged, 0, "{tally:?}");
    println!("design 2, loaded-revision Condition: {tally:?}");
}

/// The negative control `lost-refresh`.
#[test]
fn the_transient_design_loses_the_refresh_and_the_harness_sees_it() {
    let tally = every_crash_point(&obligation_canon(), true);
    assert!(tally.lost > 0, "{tally:?}");
    println!("design 3, transient on_change: {tally:?}");
}
