//! The production driver of [`crate::kernel::step`] (ADR 0006 §3).
//!
//! A [`Cell`] owns the kernel snapshot and an Event Log. For each input it
//! steps the kernel, appends the Decision's Events, and only then issues the
//! Decision's effects through the Substrate port, turning what comes back
//! into the next inputs (ADR 0012 §1). If the append fails, the Decision is
//! dropped whole: the snapshot does not move and no effect is issued, so a
//! full log stops mutation rather than losing its record (ADR 0012 §5).
//!
//! The deterministic simulator drives the same `step` with scripted inputs;
//! there is no second engine.

use std::collections::VecDeque;

use nomos_core::effect::EffectRequest;
use nomos_store::{EventLog, Full};
use nomos_substrate::{Mutate, Observe};

use crate::kernel::{Event, Input, KernelSnapshot, replay, step};

/// A kernel, its Event Log, and nothing else.
#[derive(Debug)]
pub struct Cell<L> {
    snapshot: KernelSnapshot,
    log: L,
}

impl<L: EventLog<Event>> Cell<L> {
    /// A Cell over `log`, its snapshot replayed from the log. A Cell that
    /// restarts is built this way and then handles [`Input::Recovered`].
    pub fn open(log: L) -> Self {
        let snapshot = replay(&log.events());
        Cell { snapshot, log }
    }

    /// The current snapshot.
    pub fn snapshot(&self) -> &KernelSnapshot {
        &self.snapshot
    }

    /// The Event Log.
    pub fn log(&self) -> &L {
        &self.log
    }

    /// Steps one input, records its Events, then performs its effects, and
    /// returns the inputs they produced.
    pub fn handle<S: Observe + Mutate>(
        &mut self,
        input: Input,
        substrate: &mut S,
    ) -> Result<Vec<Input>, Full> {
        let decision = step(&self.snapshot, input);
        self.log.append(&decision.events)?;
        self.snapshot = decision.snapshot;
        let mut next = Vec::new();
        for effect in decision.effects {
            match effect {
                EffectRequest::Observe(paths) => {
                    next.push(Input::Observed(substrate.observe(&paths)));
                }
                EffectRequest::Apply(request) => {
                    for receipt in substrate.apply(&request) {
                        next.push(Input::Receipt(request.key.clone(), receipt));
                    }
                }
            }
        }
        Ok(next)
    }

    /// Handles `input` and everything it leads to, in order, until nothing
    /// is left to handle.
    pub fn settle<S: Observe + Mutate>(
        &mut self,
        input: Input,
        substrate: &mut S,
    ) -> Result<(), Full> {
        let mut queue = VecDeque::from([input]);
        while let Some(input) = queue.pop_front() {
            queue.extend(self.handle(input, substrate)?);
        }
        Ok(())
    }
}

/// The production driver over a durable journal of inputs (ADR 0017 §3).
///
/// It is [`Cell`] with one difference: the journal holds each input, and
/// the snapshot is recomputed by stepping the journaled inputs from the
/// initial snapshot when the driver opens. The input is appended before
/// any effect of its Decision is issued, so the order of ADR 0012 §1 holds;
/// if the append fails, the Decision is dropped whole.
#[derive(Debug)]
pub struct JournaledCell<J> {
    snapshot: KernelSnapshot,
    journal: J,
}

impl<J: EventLog<Input>> JournaledCell<J> {
    /// A Cell over `journal`, its snapshot recomputed from every input the
    /// journal holds. A Cell that restarts is built this way and then
    /// handles [`Input::Recovered`].
    pub fn open(journal: J) -> Self {
        let mut snapshot = KernelSnapshot::new();
        for input in journal.events() {
            snapshot = step(&snapshot, input).snapshot;
        }
        JournaledCell { snapshot, journal }
    }

    /// The current snapshot.
    pub fn snapshot(&self) -> &KernelSnapshot {
        &self.snapshot
    }

    /// The journal.
    pub fn journal(&self) -> &J {
        &self.journal
    }

    /// Every Event the journaled inputs produce, in order: the Event Log
    /// the journal stands for.
    pub fn events(&self) -> Vec<Event> {
        let mut snapshot = KernelSnapshot::new();
        let mut events = Vec::new();
        for input in self.journal.events() {
            let decision = step(&snapshot, input);
            events.extend(decision.events);
            snapshot = decision.snapshot;
        }
        events
    }

    /// Steps one input, journals it, then performs its effects, and returns
    /// the inputs they produced.
    pub fn handle<S: Observe + Mutate>(
        &mut self,
        input: Input,
        substrate: &mut S,
    ) -> Result<Vec<Input>, Full> {
        let decision = step(&self.snapshot, input.clone());
        self.journal.append(&[input])?;
        self.snapshot = decision.snapshot;
        let mut next = Vec::new();
        for effect in decision.effects {
            match effect {
                EffectRequest::Observe(keys) => {
                    next.push(Input::Observed(substrate.observe(&keys)));
                }
                EffectRequest::Apply(request) => {
                    for receipt in substrate.apply(&request) {
                        next.push(Input::Receipt(request.key.clone(), receipt));
                    }
                }
            }
        }
        Ok(next)
    }

    /// Handles `input` and everything it leads to, in order, until nothing
    /// is left to handle.
    pub fn settle<S: Observe + Mutate>(
        &mut self,
        input: Input,
        substrate: &mut S,
    ) -> Result<(), Full> {
        let mut queue = VecDeque::from([input]);
        while let Some(input) = queue.pop_front() {
            queue.extend(self.handle(input, substrate)?);
        }
        Ok(())
    }
}
