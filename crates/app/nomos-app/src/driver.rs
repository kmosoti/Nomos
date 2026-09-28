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
