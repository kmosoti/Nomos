//! `step`: one input in, one Decision out.
//!
//! Every handler below reads the snapshot and emits Events; `emit` applies
//! each Event as it goes, so a handler always reads the snapshot its earlier
//! Events produced, and the Decision's snapshot is the fold of its Events.
//! After the input's own handler, [`progress`] carries the run as far as it
//! can go without another input: releasing settled reservations, requesting
//! an observation once everything is Settled, admitting Ready Actions, and
//! closing a finished round.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use nomos_core::action::{Signal, Stage, advance, holds_reservation, signals_for, verify};
use nomos_core::assessment::{Assessment, Report};
use nomos_core::effect::{Apply, EffectKey, EffectRequest, Receipt, SettledBy, Settlement};
use nomos_core::observation::{Instant, Observation};
use nomos_core::plan::Acceptance;
use nomos_core::resource::ResourcePath;
use nomos_warp::budget::Budgets;
use nomos_warp::frontier::{Frontier, Outcome, Progress, Resolution, frontier};
use nomos_warp::graph::{ConflictKey, EdgeKind, Graph, Vertex};
use nomos_warp::select::{Reserved, select};

use super::{
    ActionRecord, Decision, EffectRecord, Event, Fingerprint, Ignored, Input, KernelSnapshot,
    Nonconvergence, Obligation, Phase, Plan, Rejection, Round, Run, RunOutcome, apply,
};

/// The transition function (ADR 0006 §2).
pub fn step(snapshot: &KernelSnapshot, input: Input) -> Decision {
    let mut draft = Draft {
        snapshot: snapshot.clone(),
        events: Vec::new(),
        effects: Vec::new(),
    };
    match input {
        Input::Tick(now) => tick(&mut draft, now),
        Input::Enforce(plan) => enforce(&mut draft, plan),
        Input::Observed(observations) => observed(&mut draft, &observations),
        Input::Receipt(key, receipt) => received(&mut draft, &key, receipt),
        Input::Recovered => recovered(&mut draft),
    }
    progress(&mut draft);
    Decision {
        snapshot: draft.snapshot,
        events: draft.events,
        effects: draft.effects,
    }
}

struct Draft {
    snapshot: KernelSnapshot,
    events: Vec<Event>,
    effects: Vec<EffectRequest>,
}

impl Draft {
    fn emit(&mut self, event: Event) {
        let snapshot = core::mem::take(&mut self.snapshot);
        self.snapshot = apply(snapshot, &event);
        self.events.push(event);
    }

    fn request(&mut self, effect: EffectRequest) {
        self.effects.push(effect);
    }

    fn now(&self) -> Instant {
        self.snapshot.now
    }

    fn run(&self) -> Option<&Run> {
        self.snapshot.run.as_ref()
    }

    fn round(&self) -> Option<&Round> {
        self.snapshot.round()
    }
}

fn later(now: Instant, by: u64) -> Instant {
    Instant(now.0.saturating_add(by))
}

// ---------------------------------------------------------------------------
// Inputs

fn tick(draft: &mut Draft, now: Instant) {
    if now > draft.now() {
        draft.emit(Event::Clock(now));
    }
    let now = draft.now();
    let expired: Vec<(ResourcePath, Stage)> = draft
        .round()
        .map(|round| {
            round
                .actions
                .iter()
                .filter(|(_, a)| a.stage.is_live() && a.deadline.is_some_and(|d| now >= d))
                .map(|(r, a)| (r.clone(), a.stage.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (resource, stage) in expired {
        if let Ok(next) = advance(&stage, Signal::Deadline) {
            draft.emit(Event::ActionAdvanced {
                resource,
                stage: next,
                deadline: None,
            });
        }
    }
    let settling: Vec<EffectKey> = draft
        .snapshot
        .effects
        .iter()
        .filter(|(_, e)| !e.settlement.is_settled() && e.settlement.at(now).is_settled())
        .map(|(k, _)| k.clone())
        .collect();
    for key in settling {
        draft.emit(Event::EffectSettled {
            key,
            by: SettledBy::Deadline,
        });
    }
}

fn enforce(draft: &mut Draft, plan: Plan) {
    let rejected = |draft: &mut Draft, plan: &Plan, reason| {
        draft.emit(Event::PlanRejected {
            plan: plan.id.clone(),
            generation: plan.generation,
            reason,
        });
    };
    if plan.expires.is_some_and(|expires| draft.now() >= expires) {
        rejected(draft, &plan, Rejection::Expired);
        return;
    }
    let vertices: Vec<Vertex> = plan
        .canon
        .paths()
        .into_iter()
        .map(|p| Vertex::action(p, BTreeSet::new()))
        .collect();
    if Graph::compile(vertices, plan.canon.edges().to_vec()).is_err() {
        rejected(draft, &plan, Rejection::InvalidCanon);
        return;
    }
    match draft.snapshot.fence.accept(plan.generation, &plan.id) {
        Err(error) => rejected(draft, &plan, Rejection::Fence(error)),
        Ok((_, Acceptance::Redelivered)) => draft.emit(Event::PlanRedelivered { plan: plan.id }),
        Ok((_, Acceptance::New)) => {
            supersede(draft);
            draft.emit(Event::PlanAccepted(plan));
        }
    }
}

/// Ends the current run for a newer Plan: its Actions not yet dispatched are
/// Cancelled; dispatched ones keep running and keep their reservations
/// (ADR 0010 note, draining).
fn supersede(draft: &mut Draft) {
    let Some(run) = draft.run() else { return };
    if matches!(run.phase, Phase::Ended(_)) {
        return;
    }
    let prepared: Vec<ResourcePath> = draft
        .round()
        .map(|round| {
            round
                .actions
                .iter()
                .filter(|(_, a)| a.stage == Stage::Prepared)
                .map(|(r, _)| r.clone())
                .collect()
        })
        .unwrap_or_default();
    for resource in prepared {
        draft.emit(Event::ActionAdvanced {
            resource,
            stage: Stage::Cancelled,
            deadline: None,
        });
    }
    draft.emit(Event::RunEnded(RunOutcome::Superseded));
}

fn observed(draft: &mut Draft, observations: &[Observation]) {
    verify_actions(draft, observations);
    let Some(run) = draft.run() else { return };
    let Phase::Observing { since } = run.phase else {
        return;
    };
    let paths = run.plan.canon.paths();
    let fresh: Vec<Observation> = observations
        .iter()
        .filter(|o| o.provenance().window().start() >= since && paths.contains(o.path()))
        .cloned()
        .collect();
    if paths.iter().all(|p| fresh.iter().any(|o| o.path() == p)) {
        decide(draft, &fresh);
    }
}

/// Verifies every Verifying Action that `observations` holds fresh evidence
/// about (N6).
fn verify_actions(draft: &mut Draft, observations: &[Observation]) {
    let (Some(run), Some(round)) = (draft.run(), draft.round()) else {
        return;
    };
    let mut verdicts = Vec::new();
    for (resource, action) in &round.actions {
        let (Stage::Verifying { .. }, Some(since)) = (&action.stage, action.completed_at) else {
            continue;
        };
        let fresh = observations
            .iter()
            .any(|o| o.path() == resource && o.provenance().window().start() >= since);
        let Some(managed) = run.plan.canon.resources().get(resource) else {
            continue;
        };
        if fresh {
            let verdict = verify(&managed.condition, observations, since);
            verdicts.push((resource.clone(), action.clone(), verdict));
        }
    }
    for (resource, action, verdict) in verdicts {
        let Ok(next) = advance(&action.stage, Signal::Verify(verdict)) else {
            continue;
        };
        if next == action.stage {
            continue;
        }
        let succeeded = match &next {
            Stage::Succeeded { changed, .. } => Some(*changed),
            _ => None,
        };
        draft.emit(Event::ActionAdvanced {
            resource,
            stage: next,
            deadline: None,
        });
        if let Some(changed) = succeeded {
            let pending = draft.snapshot.obligations.clone();
            for obligation in action.discharges.intersection(&pending) {
                draft.emit(Event::ObligationDischarged(obligation.clone()));
            }
            if !changed {
                withdraw(draft, &action.key);
            }
        }
    }
}

fn withdraw(draft: &mut Draft, cause: &EffectKey) {
    let caused: Vec<Obligation> = draft
        .snapshot
        .obligations
        .iter()
        .filter(|o| &o.cause == cause)
        .cloned()
        .collect();
    for obligation in caused {
        draft.emit(Event::ObligationWithdrawn(obligation));
    }
}

fn received(draft: &mut Draft, key: &EffectKey, receipt: Receipt) {
    let Some(record) = draft.snapshot.effects.get(key) else {
        let why = if draft.snapshot.released.contains(key) {
            Ignored::Duplicate
        } else {
            Ignored::Unknown
        };
        draft.emit(Event::ReceiptIgnored {
            key: key.clone(),
            receipt,
            why,
        });
        return;
    };
    let mut changed_anything = false;
    if receipt.settles() && !record.settlement.is_settled() {
        draft.emit(Event::EffectSettled {
            key: key.clone(),
            by: SettledBy::Receipt,
        });
        changed_anything = true;
    }
    if receipt == Receipt::Refused {
        withdraw(draft, key);
    }
    let action = draft.round().and_then(|round| {
        round
            .actions
            .iter()
            .find(|(_, a)| &a.key == key)
            .map(|(r, a)| (r.clone(), a.clone()))
    });
    let signals = action
        .as_ref()
        .map(|(_, a)| signals_for(&a.stage, &receipt))
        .unwrap_or_default();
    match action {
        Some((resource, action)) if !signals.is_empty() => {
            let mut stage = action.stage.clone();
            for signal in signals {
                match advance(&stage, signal) {
                    Ok(next) => stage = next,
                    Err(_) => break,
                }
            }
            let verify_timeout = draft.run().map_or(0, |run| run.plan.policy.verify_timeout);
            let deadline = match stage {
                Stage::Verifying { .. } => Some(later(draft.now(), verify_timeout)),
                ref live if live.is_live() => action.deadline,
                _ => None,
            };
            let verifying = matches!(stage, Stage::Verifying { .. });
            draft.emit(Event::ActionAdvanced {
                resource: resource.clone(),
                stage,
                deadline,
            });
            if verifying {
                draft.request(EffectRequest::Observe(alloc::vec![resource]));
            }
        }
        _ if !changed_anything => draft.emit(Event::ReceiptIgnored {
            key: key.clone(),
            receipt,
            why: Ignored::Late,
        }),
        _ => {}
    }
}

/// After a restart: an Action whose effect was dispatched and not reported
/// complete has an unknown outcome (ADR 0012 §1, N10). A completed one only
/// lost its verification, so it is observed again.
fn recovered(draft: &mut Draft) {
    draft.emit(Event::Recovered);
    let actions: Vec<(ResourcePath, Stage)> = draft
        .round()
        .map(|round| {
            round
                .actions
                .iter()
                .map(|(r, a)| (r.clone(), a.stage.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (resource, stage) in actions {
        match stage {
            Stage::Dispatched | Stage::Accepted | Stage::Running => {
                if let Ok(next) = advance(&stage, Signal::Deadline) {
                    draft.emit(Event::ActionAdvanced {
                        resource,
                        stage: next,
                        deadline: None,
                    });
                }
            }
            Stage::Verifying { .. } => {
                draft.request(EffectRequest::Observe(alloc::vec![resource]));
            }
            _ => {}
        }
    }
    if let Some(run) = draft.run()
        && matches!(run.phase, Phase::Observing { .. })
    {
        let paths = run.plan.canon.paths();
        draft.request(EffectRequest::Observe(paths));
    }
}

// ---------------------------------------------------------------------------
// The loop

/// Decides an iteration from a complete, fresh observation
/// (reconciliation.md, Algorithm).
fn decide(draft: &mut Draft, fresh: &[Observation]) {
    let Some(run) = draft.run() else { return };
    let canon = &run.plan.canon;
    let report = Report::assess(&canon.conditions(), fresh);
    let fingerprint: Fingerprint = canon
        .paths()
        .into_iter()
        .map(|p| {
            let mut collections: Vec<_> = fresh
                .iter()
                .filter(|o| o.path() == &p)
                .map(|o| *o.collection())
                .collect();
            collections.sort();
            collections.dedup();
            (p, collections)
        })
        .collect();
    let owed: BTreeSet<ResourcePath> = draft
        .snapshot
        .obligations
        .iter()
        .filter(|o| canon.resources().contains_key(&o.target))
        .map(|o| o.target.clone())
        .collect();
    let settled = draft
        .snapshot
        .effects
        .values()
        .all(|e| e.settlement.is_settled());
    let outcome = if report.all_satisfied() && owed.is_empty() && settled {
        Some(RunOutcome::Converged)
    } else if run.iteration >= run.plan.bound {
        Some(RunOutcome::NonConvergent(Nonconvergence::Bound))
    } else if report.variances().next().is_none() && owed.is_empty() {
        Some(RunOutcome::Indeterminate(
            report
                .indeterminate()
                .map(|(c, r)| (c.path().clone(), *r))
                .collect(),
        ))
    } else if run.history.contains(&fingerprint) {
        Some(RunOutcome::NonConvergent(Nonconvergence::Oscillation))
    } else {
        None
    };
    match outcome {
        Some(outcome) => draft.emit(Event::RunEnded(outcome)),
        None => match plan_round(run, &report, &owed) {
            Some(round) => draft.emit(Event::RoundPlanned { round, fingerprint }),
            None => draft.emit(Event::RunEnded(RunOutcome::Failed {
                failed: Vec::new(),
                unknown: Vec::new(),
            })),
        },
    }
}

/// Plans one round: an Action for every resource with a Variance or a
/// pending Obligation, and for every `on_change` target of an Action; an
/// anchor for everything else (ADR 0009 note).
fn plan_round(run: &Run, report: &Report, owed: &BTreeSet<ResourcePath>) -> Option<Round> {
    let canon = &run.plan.canon;
    let assessment: BTreeMap<&ResourcePath, &Assessment> = report
        .entries()
        .iter()
        .map(|(c, a)| (c.path(), a))
        .collect();
    let indeterminate =
        |r: &ResourcePath| matches!(assessment.get(r), Some(Assessment::Indeterminate(_)) | None);
    let mut acting: BTreeSet<ResourcePath> = canon
        .paths()
        .into_iter()
        .filter(|r| !indeterminate(r))
        .filter(|r| owed.contains(r) || matches!(assessment.get(r), Some(Assessment::Variance(_))))
        .collect();
    loop {
        let anticipated: Vec<ResourcePath> = canon
            .edges()
            .iter()
            .filter(|e| e.kind() == EdgeKind::OnChange)
            .filter(|e| acting.contains(e.source()) && !acting.contains(e.target()))
            .filter(|e| !indeterminate(e.target()))
            .map(|e| e.target().clone())
            .collect();
        if anticipated.is_empty() {
            break;
        }
        acting.extend(anticipated);
    }
    let mut vertices = Vec::new();
    let mut actions = BTreeMap::new();
    for (path, managed) in canon.resources() {
        let vertex = if indeterminate(path) {
            Vertex::indeterminate_anchor(path.clone())
        } else if acting.contains(path) {
            // A refresh reads the files its `on_change` sources write, so it
            // holds their keys too (ADR 0009 note, footprint).
            let mut keys: BTreeSet<ConflictKey> = managed.keys.clone();
            for edge in canon.edges() {
                if edge.kind() == EdgeKind::OnChange
                    && edge.target() == path
                    && let Some(source) = canon.resources().get(edge.source())
                {
                    keys.extend(source.keys.iter().cloned());
                }
            }
            actions.insert(
                path.clone(),
                ActionRecord {
                    key: EffectKey::new(
                        run.plan.id.clone(),
                        run.plan.generation,
                        run.iteration,
                        path.clone(),
                    ),
                    operation: managed.operation(),
                    stage: Stage::Prepared,
                    deadline: None,
                    completed_at: None,
                    discharges: BTreeSet::new(),
                },
            );
            let vertex = if owed.contains(path) {
                Vertex::owed(path.clone(), keys)
            } else {
                Vertex::action(path.clone(), keys)
            };
            vertex.disrupting(managed.disrupts.clone())
        } else {
            Vertex::anchor(path.clone())
        };
        vertices.push(vertex);
    }
    let graph = Graph::compile(vertices, canon.edges().to_vec()).ok()?;
    Some(Round {
        iteration: run.iteration,
        graph,
        actions,
        indeterminate: report
            .indeterminate()
            .map(|(c, r)| (c.path().clone(), *r))
            .collect(),
    })
}

/// Carries the run forward until it needs another input.
fn progress(draft: &mut Draft) {
    loop {
        release(draft);
        let Some(run) = draft.run() else { return };
        match &run.phase {
            Phase::Awaiting => {
                let settled = draft
                    .snapshot
                    .effects
                    .values()
                    .all(|e| e.settlement.is_settled());
                if settled {
                    let paths = run.plan.canon.paths();
                    let since = draft.now();
                    draft.emit(Event::ObservationRequested { since });
                    draft.request(EffectRequest::Observe(paths));
                }
                return;
            }
            Phase::Observing { .. } | Phase::Ended(_) => return,
            Phase::Executing(_) => {
                admit(draft);
                if !finish_round(draft) {
                    return;
                }
            }
        }
    }
}

/// Releases the reservation of every effect the reservation rule no longer
/// holds: Settled, and its Action not live (`holds_reservation`).
fn release(draft: &mut Draft) {
    let stages: BTreeMap<EffectKey, Stage> = draft
        .round()
        .map(|round| {
            round
                .actions
                .values()
                .map(|a| (a.key.clone(), a.stage.clone()))
                .collect()
        })
        .unwrap_or_default();
    let releasable: Vec<EffectKey> = draft
        .snapshot
        .effects
        .iter()
        .filter(|(k, e)| !holds_reservation(stages.get(*k), &e.settlement))
        .map(|(k, _)| k.clone())
        .collect();
    for key in releasable {
        draft.emit(Event::EffectReleased { key });
    }
}

fn outcome_of(stage: &Stage) -> Option<Outcome> {
    match stage {
        Stage::Succeeded { changed, .. } => Some(Outcome::Succeeded { changed: *changed }),
        Stage::Failed(_) => Some(Outcome::Failed),
        Stage::TimedOut => Some(Outcome::TimedOut),
        Stage::Cancelled => Some(Outcome::Cancelled),
        Stage::Rejected => Some(Outcome::Rejected),
        _ => None,
    }
}

fn round_frontier(round: &Round) -> Frontier {
    let progress: BTreeMap<ResourcePath, Progress> = round
        .actions
        .iter()
        .filter(|(_, a)| a.stage != Stage::Prepared)
        .map(|(r, a)| {
            let p = outcome_of(&a.stage).map_or(Progress::Running, Progress::Done);
            (r.clone(), p)
        })
        .collect();
    frontier(&round.graph, &progress)
}

/// Admits the Ready Actions that fit, in one serialized selection, with the
/// fence re-checked in the same step (N5, ADR 0010 §3).
fn admit(draft: &mut Draft) {
    let (Some(run), Some(round)) = (draft.run(), draft.round()) else {
        return;
    };
    if !draft
        .snapshot
        .fence
        .permits(run.plan.generation, &run.plan.id)
    {
        return;
    }
    let now = draft.now();
    let policy = &run.plan.policy;
    let reserved = Reserved {
        keys: draft
            .snapshot
            .effects
            .values()
            .flat_map(|e| e.keys.iter().cloned())
            .collect(),
        nodes: draft
            .snapshot
            .effects
            .values()
            .flat_map(|e| e.disrupts.iter().cloned())
            .collect(),
        count: draft.snapshot.effects.len(),
    };
    let fresh = now.0.saturating_sub(policy.budget_observed_at.0) <= policy.budget_max_age;
    let budgets = Budgets::new(policy.budgets.clone(), policy.unavailable.clone(), fresh);
    let ready = round_frontier(round);
    let selection = select(&round.graph, &ready, &reserved, policy.capacity, &budgets);
    let mut dispatches = Vec::new();
    for resource in selection.chosen() {
        let (Some(action), Some(vertex)) =
            (round.actions.get(resource), round.graph.vertex(resource))
        else {
            continue;
        };
        if action.stage != Stage::Prepared {
            continue;
        }
        let targets: Vec<ResourcePath> = run
            .plan
            .canon
            .edges()
            .iter()
            .filter(|e| e.kind() == EdgeKind::OnChange && e.source() == resource)
            .map(|e| e.target().clone())
            .collect();
        dispatches.push((
            resource.clone(),
            action.clone(),
            vertex.keys().clone(),
            vertex.disrupts().clone(),
            targets,
        ));
    }
    let receipt_timeout = policy.receipt_timeout;
    let settle_after = policy.settle_after;
    for (resource, action, keys, disrupts, targets) in dispatches {
        // The Obligation is recorded before the change that may create it
        // (ADR 0009 §4): its Event precedes the effect request.
        for target in targets {
            draft.emit(Event::ObligationRecorded(Obligation {
                target,
                cause: action.key.clone(),
            }));
        }
        let discharges: BTreeSet<Obligation> = draft
            .snapshot
            .obligations
            .iter()
            .filter(|o| o.target == resource)
            .cloned()
            .collect();
        draft.emit(Event::ActionDispatched {
            resource: resource.clone(),
            deadline: later(now, receipt_timeout),
            discharges,
        });
        let settle_by = later(now, settle_after);
        draft.emit(Event::EffectRequested {
            key: action.key.clone(),
            record: EffectRecord {
                keys,
                disrupts,
                settlement: Settlement::new(settle_by),
            },
        });
        draft.request(EffectRequest::Apply(Apply {
            key: action.key,
            operation: action.operation,
            settle_by,
        }));
    }
}

/// Closes the round when no Action can still run, and says whether it did.
fn finish_round(draft: &mut Draft) -> bool {
    let Some(round) = draft.round() else {
        return false;
    };
    let resolutions = round_frontier(round);
    let open = round.actions.iter().any(|(r, a)| {
        a.stage.is_live()
            || (a.stage == Stage::Prepared
                && !matches!(
                    resolutions.resolution(r),
                    Some(Resolution::Blocked | Resolution::Skipped)
                ))
    });
    if open {
        return false;
    }
    let failed: Vec<ResourcePath> = round
        .actions
        .iter()
        .filter(|(_, a)| matches!(a.stage, Stage::Failed(_) | Stage::Rejected))
        .map(|(r, _)| r.clone())
        .collect();
    let unknown: Vec<ResourcePath> = round
        .actions
        .iter()
        .filter(|(_, a)| a.stage == Stage::TimedOut)
        .map(|(r, _)| r.clone())
        .collect();
    let succeeded = round
        .actions
        .values()
        .any(|a| matches!(a.stage, Stage::Succeeded { .. }));
    let indeterminate = round.indeterminate.clone();
    if !failed.is_empty() || !unknown.is_empty() {
        draft.emit(Event::RunEnded(RunOutcome::Failed { failed, unknown }));
    } else if !succeeded {
        // Everything left is Blocked behind an unknown: another iteration
        // would observe the same unknown (reconciliation.md, transition
        // kernel note).
        draft.emit(Event::RunEnded(RunOutcome::Indeterminate(indeterminate)));
    } else {
        draft.emit(Event::IterationAdvanced);
    }
    true
}
