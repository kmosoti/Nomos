//! Applying one Event to a snapshot. This is the only function that changes
//! a [`KernelSnapshot`], so the Event Log determines the snapshot.
//!
//! `apply` trusts its Event: `step` emits only Events that make sense for the
//! snapshot it emits them on. An Event that names something the snapshot
//! does not have, which a hand-edited log could contain, changes nothing.

use super::{Event, KernelSnapshot, Phase, Run};
use nomos_core::effect::Settlement;

/// The snapshot after `event`.
pub fn apply(mut snapshot: KernelSnapshot, event: &Event) -> KernelSnapshot {
    match event {
        Event::Clock(now) => {
            if *now > snapshot.now {
                snapshot.now = *now;
            }
        }
        Event::PlanRejected { .. }
        | Event::PlanRedelivered { .. }
        | Event::ReceiptIgnored { .. }
        | Event::Recovered => {}
        Event::PlanAccepted(plan) => {
            if let Ok((fence, _)) = snapshot.fence.accept(plan.generation, &plan.id) {
                snapshot.fence = fence;
                snapshot.run = Some(Run {
                    plan: plan.clone(),
                    iteration: 0,
                    phase: Phase::Awaiting,
                    history: alloc::vec::Vec::new(),
                });
            }
        }
        Event::RunEnded(outcome) => {
            if let Some(run) = snapshot.run.as_mut() {
                run.phase = Phase::Ended(outcome.clone());
            }
        }
        Event::ObservationRequested { since } => {
            if let Some(run) = snapshot.run.as_mut() {
                run.phase = Phase::Observing { since: *since };
            }
        }
        Event::RoundPlanned { round, fingerprint } => {
            if let Some(run) = snapshot.run.as_mut() {
                run.history.push(fingerprint.clone());
                run.phase = Phase::Executing(round.clone());
            }
        }
        Event::IterationAdvanced => {
            if let Some(run) = snapshot.run.as_mut() {
                run.iteration += 1;
                run.phase = Phase::Awaiting;
            }
        }
        Event::ObligationRecorded(obligation) => {
            snapshot.obligations.insert(obligation.clone());
        }
        Event::ObligationWithdrawn(obligation) | Event::ObligationDischarged(obligation) => {
            snapshot.obligations.remove(obligation);
        }
        Event::ActionDispatched {
            resource,
            deadline,
            discharges,
        } => {
            if let Some(action) = action_mut(&mut snapshot, resource) {
                action.stage = nomos_core::action::Stage::Dispatched;
                action.deadline = Some(*deadline);
                action.discharges = discharges.clone();
            }
        }
        Event::ActionAdvanced {
            resource,
            stage,
            deadline,
        } => {
            let now = snapshot.now;
            if let Some(action) = action_mut(&mut snapshot, resource) {
                let completing = matches!(stage, nomos_core::action::Stage::Verifying { .. })
                    && !matches!(action.stage, nomos_core::action::Stage::Verifying { .. });
                if completing {
                    action.completed_at = Some(now);
                }
                action.stage = stage.clone();
                action.deadline = *deadline;
            }
        }
        Event::EffectRequested { key, record } => {
            snapshot.effects.insert(key.clone(), record.clone());
        }
        Event::EffectSettled { key, by } => {
            if let Some(record) = snapshot.effects.get_mut(key)
                && !record.settlement.is_settled()
            {
                record.settlement = Settlement::Settled(*by);
            }
        }
        Event::EffectReleased { key } => {
            if snapshot.effects.remove(key).is_some() {
                snapshot.released.insert(key.clone());
            }
        }
    }
    snapshot
}

fn action_mut<'a>(
    snapshot: &'a mut KernelSnapshot,
    resource: &nomos_core::resource::ResourceKey,
) -> Option<&'a mut super::ActionRecord> {
    match &mut snapshot.run.as_mut()?.phase {
        Phase::Executing(round) => round.actions.get_mut(resource),
        _ => None,
    }
}
