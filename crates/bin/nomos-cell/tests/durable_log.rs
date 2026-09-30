//! Experiment `durable-log` (Phase 1 plan, `10-durable-cell`): does
//! recovery from disk restore every Obligation and Event that was
//! acknowledged, and refuse a torn or corrupted log?
//!
//! The Cell journals its inputs in a `FileLog` (ADR 0017 §3) and recomputes
//! its snapshot by stepping them. The refresh scenario of
//! `05-transition-kernel` runs once without interruption, which gives the
//! snapshot after every input; then, for every input, a Cell is killed right
//! after it, reopened from the file alone, and must hold exactly that
//! snapshot, its pending Obligations included, and then still converge
//! with the refresh done.

mod scratch;
mod support;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use nomos_app::driver::{Cell, JournaledCell};
use nomos_app::kernel::{Input, KernelSnapshot, RunOutcome, step};
use nomos_core::observation::Instant;
use nomos_store::EventLog;
use nomos_store_fs::{FileLog, LogError};
use support::*;

fn journal(name: &str) -> PathBuf {
    scratch::dir("durable", name).join("journal")
}

fn open(path: &Path) -> JournaledCell<FileLog<Input>> {
    let (log, _) = FileLog::open(path).unwrap();
    JournaledCell::open(log)
}

/// The inputs of the refresh scenario, handled one at a time: the input,
/// and the snapshot after it.
fn reference() -> Vec<(Input, KernelSnapshot)> {
    let mut host = refresh_host();
    let mut cell = open(&journal("reference"));
    let mut queue = VecDeque::from([Input::Enforce(plan("p1", 1, obligation_canon(), 3))]);
    let mut steps = Vec::new();
    while let Some(input) = queue.pop_front() {
        queue.extend(cell.handle(input.clone(), &mut host).unwrap());
        steps.push((input, cell.snapshot().clone()));
    }
    assert_eq!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    steps
}

/// The Cell killed after each input recovers exactly the snapshot it had,
/// and finishes the refresh.
#[test]
fn a_cell_killed_after_every_input_recovers_its_snapshot() {
    let steps = reference();
    let owed = steps
        .iter()
        .filter(|(_, s)| !s.obligations().is_empty())
        .count();
    assert!(owed > 0, "the scenario never owed a refresh");
    for kill in 1..=steps.len() {
        let path = journal(&format!("kill-{kill}"));
        let mut host = refresh_host();
        let mut cell = open(&path);
        let mut queue = VecDeque::from([steps[0].0.clone()]);
        for _ in 0..kill {
            let input = queue.pop_front().unwrap();
            queue.extend(cell.handle(input, &mut host).unwrap());
        }
        // The process dies: its queue and snapshot are gone; the host keeps
        // what was done to it, and the file keeps what was appended.
        drop(cell);
        let mut cell = open(&path);
        assert_eq!(
            cell.snapshot(),
            &steps[kill - 1].1,
            "after input {kill}: the recovered snapshot differs"
        );
        assert_eq!(
            cell.snapshot().obligations(),
            steps[kill - 1].1.obligations(),
            "after input {kill}: an Obligation was lost"
        );
        cell.settle(Input::Recovered, &mut host).unwrap();
        // Time passes past every settle-by instant, on the host's clock and
        // the kernel's, so that what was in flight is Settled by time.
        host.set_time(Instant(1_000));
        cell.settle(Input::Tick(Instant(1_000)), &mut host).unwrap();
        cell.settle(
            Input::Enforce(plan("p2", 2, obligation_canon(), 3)),
            &mut host,
        )
        .unwrap();
        assert_eq!(
            cell.snapshot().outcome(),
            Some(&RunOutcome::Converged),
            "killed after input {kill}"
        );
        let service = host.service(&p(SVC)).unwrap();
        assert_eq!(
            service.loaded,
            host.file(&p(CONF)),
            "killed after input {kill}: the service runs the old configuration"
        );
        assert!(cell.snapshot().obligations().is_empty());
    }
    println!(
        "killed after each of {} inputs; {owed} of them with a refresh owed",
        steps.len()
    );
}

/// The journal stands for the Event Log: the Events it recomputes are the
/// ones the in-memory driver records, and so is the snapshot.
#[test]
fn the_journal_recomputes_the_event_log() {
    let mut host = refresh_host();
    let mut journaled = open(&journal("agree"));
    journaled
        .settle(
            Input::Enforce(plan("p", 1, obligation_canon(), 3)),
            &mut host,
        )
        .unwrap();
    let mut other = refresh_host();
    let mut cell = Cell::open(MemLog::default());
    cell.settle(
        Input::Enforce(plan("p", 1, obligation_canon(), 3)),
        &mut other,
    )
    .unwrap();
    assert_eq!(journaled.events(), cell.log().events());
    assert_eq!(journaled.snapshot(), cell.snapshot());
    assert_eq!(host, other);
}

/// Every input the scenario produced reads back as itself.
#[test]
fn every_journaled_input_reads_back_as_itself() {
    for (input, _) in reference() {
        let bytes = nomos_app::journal::encode(&input);
        assert_eq!(nomos_app::journal::decode(&bytes), Some(input));
    }
}

/// A journal changed on disk is refused, so the Cell does not start with a
/// forgotten Obligation; one cut short at its end starts without the cut
/// batch, which was never acknowledged.
#[test]
fn a_changed_journal_is_refused_and_a_torn_one_is_repaired() {
    let path = journal("damage");
    let mut host = refresh_host();
    let mut cell = open(&path);
    cell.settle(
        Input::Enforce(plan("p", 1, obligation_canon(), 3)),
        &mut host,
    )
    .unwrap();
    let whole = cell.journal().events();
    drop(cell);
    let bytes = std::fs::read(&path).unwrap();

    let mut changed = bytes.clone();
    let middle = changed.len() / 2;
    changed[middle] ^= 0x40;
    std::fs::write(&path, &changed).unwrap();
    assert!(matches!(
        FileLog::<Input>::open(&path),
        Err(LogError::Corrupt { .. })
    ));

    let mut torn = bytes.clone();
    torn.truncate(bytes.len() - 5);
    std::fs::write(&path, &torn).unwrap();
    let (log, recovery) = FileLog::<Input>::open(&path).unwrap();
    assert!(recovery.truncated > 0);
    assert_eq!(log.events(), whole[..whole.len() - 1].to_vec());
}

/// A journal that accepts `n` appends and refuses every later one.
#[derive(Debug, Default)]
struct RefuseAfter(usize, Vec<Input>);

impl EventLog<Input> for RefuseAfter {
    fn append(&mut self, batch: &[Input]) -> Result<(), nomos_store::Full> {
        if self.1.len() >= self.0 {
            return Err(nomos_store::Full);
        }
        self.1.extend(batch.iter().cloned());
        Ok(())
    }
    fn events(&self) -> Vec<Input> {
        self.1.clone()
    }
}

/// ADR 0012 §5 through the journal: the input whose Decision would change
/// the host cannot be journaled, so no effect is issued and the snapshot
/// does not move; nothing happens that the Cell could not recover.
#[test]
fn a_journal_that_refuses_stops_mutation() {
    let mut host = refresh_host();
    let before = host.clone();
    // Accept the Enforce, whose Decision only asks for Observations, and
    // refuse the Observations, whose Decision dispatches the write.
    let mut cell = JournaledCell::open(RefuseAfter(1, Vec::new()));
    let next = cell
        .handle(
            Input::Enforce(plan("p", 1, obligation_canon(), 3)),
            &mut host,
        )
        .unwrap();
    let observed = next.into_iter().next().unwrap();
    assert!(matches!(observed, Input::Observed(_)));
    let snapshot = cell.snapshot().clone();
    let reference = step(&snapshot, observed.clone());
    assert!(
        reference
            .effects
            .iter()
            .any(|e| matches!(e, nomos_core::effect::EffectRequest::Apply(_))),
        "the refused input would have changed the host"
    );
    assert_eq!(cell.handle(observed, &mut host), Err(nomos_store::Full));
    assert_eq!(cell.snapshot(), &snapshot);
    assert_eq!(host, before);
    assert!(host.executions().is_empty());
}
