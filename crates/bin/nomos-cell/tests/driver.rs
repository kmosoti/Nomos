//! One engine, two drivers (ADR 0006 §3): the production driver steps the
//! same kernel through the Substrate and Store ports, records before it
//! acts, and stops mutating when the log is full (ADR 0012 §5).

mod support;

use nomos_app::driver::Cell;
use nomos_app::kernel::{Input, KernelSnapshot, RunOutcome, replay};
use nomos_app::trace::trace;
use nomos_core::assessment::Assessment;
use nomos_store::{EventLog, Full};
use support::*;

#[test]
fn the_production_driver_converges_the_same_host() {
    let mut host = refresh_host();
    let mut cell = Cell::open(MemLog::default());
    cell.settle(
        Input::Enforce(plan("p", 1, obligation_canon(), 3)),
        &mut host,
    )
    .unwrap();
    assert_eq!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    assert_eq!(cell.snapshot(), &replay(&cell.log().events()));
    let mut sim = Sim::new(refresh_host());
    sim.enforce(plan("p", 1, obligation_canon(), 3));
    assert_eq!(sim.run(), RunOutcome::Converged);
    assert_eq!(host, sim.host, "both drivers leave the host the same");
    assert_eq!(
        cell.log().events(),
        sim.log.events(),
        "and record the same Events"
    );
}

#[test]
fn reopening_a_cell_replays_its_log() {
    let mut host = refresh_host();
    let mut cell = Cell::open(MemLog::default());
    cell.settle(
        Input::Enforce(plan("p", 1, obligation_canon(), 3)),
        &mut host,
    )
    .unwrap();
    let reopened = Cell::open(cell.log().clone());
    assert_eq!(reopened.snapshot(), cell.snapshot());
}

#[test]
fn a_full_log_stops_mutation() {
    let mut host = refresh_host();
    let before = host.clone();
    let mut log = MemLog::default();
    log.full = true;
    let mut cell = Cell::open(log);
    let result = cell.handle(
        Input::Enforce(plan("p", 1, obligation_canon(), 3)),
        &mut host,
    );
    assert_eq!(result, Err(Full));
    assert_eq!(cell.snapshot(), &KernelSnapshot::new());
    assert_eq!(host, before);
}

/// Trace observes and assesses and changes nothing (N1); that it cannot
/// request a change is the compile-fail case in `nomos-app`.
#[test]
fn trace_changes_nothing() {
    let mut host = refresh_host();
    let before = host.clone();
    let report = trace(&mut host, &obligation_canon());
    let assessments: Vec<&Assessment> = report.entries().iter().map(|(_, a)| a).collect();
    assert!(assessments[0].is_variance());
    assert_eq!(assessments[1], &Assessment::Satisfied);
    assert_eq!(host, before);
}
