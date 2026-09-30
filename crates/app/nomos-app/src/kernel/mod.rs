//! The transition kernel: `step(snapshot, input) = decision` (ADR 0006 §2).
//!
//! `step` composes the transition rules of `nomos-core` (the Action
//! lifecycle, the fence, settlement) with Warp's frontier and selection. It
//! lives here because `nomos-core` may not depend on `nomos-warp`; it
//! performs no I/O, reads no clock, and names no port (ADR 0006 note). Time,
//! policy, and identity arrive as data in the input or the snapshot.
//!
//! **The snapshot is a fold of Events.** Every change to a
//! [`KernelSnapshot`] is made by applying an [`Event`], and a [`Decision`]
//! carries the Events that produced its snapshot. Replaying the Event Log
//! from [`KernelSnapshot::new`] therefore reproduces the snapshot
//! ([event-log.md], control state as a fold), which is what recovery does.
//!
//! **One run at a time.** An accepted [`Plan`] starts a run of the
//! reconciliation loop of [reconciliation.md]: wait until every effect is
//! Settled, observe, assess, stop if Converged or at the bound, plan a round
//! of Actions from the Assessments and the pending Obligations, execute it,
//! and observe again. A newer Plan supersedes the run.
//!
//! [event-log.md]: ../../../../../docs/formal/event-log.md
//! [reconciliation.md]: ../../../../../docs/formal/reconciliation.md

mod apply;
mod artifact;
mod step;
#[cfg(test)]
mod tests;

pub use apply::apply;
pub use artifact::EmptyLabel;
pub use step::step;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use nomos_core::action::Stage;
use nomos_core::assessment::Reason;
use nomos_core::condition::Condition;
use nomos_core::effect::{EffectKey, EffectRequest, Operation, Receipt, SettledBy, Settlement};
use nomos_core::observation::{Collection, Instant, Observation};
use nomos_core::plan::{Fence, FenceError, Generation, PlanId};
use nomos_core::resource::{Family, ResourceKey};
use nomos_warp::budget::{Budget, Node};
use nomos_warp::graph::{ConflictKey, Edge, Graph};

/// One resource the Canon manages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Managed {
    /// The Condition on the resource.
    pub condition: Condition,
    /// The conflict keys its Action holds.
    pub keys: BTreeSet<ConflictKey>,
    /// The nodes its Action would make unavailable.
    pub disrupts: BTreeSet<Node>,
}

impl Managed {
    /// The resource.
    pub fn key(&self) -> &ResourceKey {
        self.condition.key()
    }

    /// What the Action on this resource does, given whether an `on_change`
    /// relation or a pending Obligation asks it to refresh. A unit refreshes
    /// only when asked; the legacy service always does, as it did in
    /// `05-transition-kernel`; every other family converges.
    pub fn operation(&self, refresh: bool) -> Operation {
        let requirement = self.condition.requirement().clone();
        match self.key().family() {
            Family::Service => Operation::Refresh(requirement),
            Family::Unit if refresh => Operation::Refresh(requirement),
            _ => Operation::Converge(requirement),
        }
    }
}

/// The part of a compiled Canon the kernel reads: managed resources and the
/// edges between them. A decoded artifact becomes one through
/// `TryFrom<&nomos_canon::model::Canon>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canon {
    resources: BTreeMap<ResourceKey, Managed>,
    edges: Vec<Edge>,
}

impl Canon {
    /// A Canon of `resources` and `edges`. A later resource with the key of
    /// an earlier one replaces it; the order of either list does not matter.
    pub fn new(resources: Vec<Managed>, edges: Vec<Edge>) -> Self {
        let mut edges = edges;
        edges.sort();
        edges.dedup();
        Canon {
            resources: resources
                .into_iter()
                .map(|m| (m.key().clone(), m))
                .collect(),
            edges,
        }
    }

    /// The managed resources, by key.
    pub fn resources(&self) -> &BTreeMap<ResourceKey, Managed> {
        &self.resources
    }

    /// The edges, sorted.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Every managed resource's key, in order.
    pub fn keys(&self) -> Vec<ResourceKey> {
        self.resources.keys().cloned().collect()
    }

    /// Every Condition, in key order.
    pub fn conditions(&self) -> Vec<Condition> {
        self.resources
            .values()
            .map(|m| m.condition.clone())
            .collect()
    }
}

/// Timing, capacity, and budgets for a run. Durations are in the units of
/// [`Instant`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// How many Actions may hold reservations at once.
    pub capacity: usize,
    /// How long after dispatch a receipt may take before the Action is
    /// TimedOut.
    pub receipt_timeout: u64,
    /// How long after completion verification may take.
    pub verify_timeout: u64,
    /// How long after dispatch the Substrate guarantees the effect can cause
    /// no further change: the settle-by instant is dispatch plus this.
    pub settle_after: u64,
    /// Failure-domain budgets (N9).
    pub budgets: Vec<Budget>,
    /// The nodes unavailable in the budget snapshot.
    pub unavailable: BTreeSet<Node>,
    /// When the budget snapshot was taken.
    pub budget_observed_at: Instant,
    /// How old the budget snapshot may be at admission.
    pub budget_max_age: u64,
}

impl Policy {
    /// A policy with `capacity`, the given timeouts, and no budgets.
    pub fn new(
        capacity: usize,
        receipt_timeout: u64,
        verify_timeout: u64,
        settle_after: u64,
    ) -> Self {
        Policy {
            capacity,
            receipt_timeout,
            verify_timeout,
            settle_after,
            budgets: Vec::new(),
            unavailable: BTreeSet::new(),
            budget_observed_at: Instant(0),
            budget_max_age: u64::MAX,
        }
    }
}

/// An authority's Plan: enforce this Canon, within this bound, under this
/// generation. Its Actions are planned per iteration from fresh Assessments
/// (ADR 0010 §5), and every one of them is fenced by the Plan's generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The Plan's identifier.
    pub id: PlanId,
    /// The authority's generation.
    pub generation: Generation,
    /// What to enforce.
    pub canon: Canon,
    /// The most executions the run may perform: it observes at most
    /// `bound + 1` times.
    pub bound: u32,
    /// Timing, capacity, and budgets.
    pub policy: Policy,
    /// The lease: past this instant the Plan is not accepted.
    pub expires: Option<Instant>,
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// An authority presents a Plan.
    Enforce(Plan),
    /// Observations arrive.
    Observed(Vec<Observation>),
    /// A receipt arrives for the execution `key`.
    Receipt(EffectKey, Receipt),
    /// Time is now this instant.
    Tick(Instant),
    /// The process restarted and replayed its Event Log.
    Recovered,
}

/// Why a Plan was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// The fence refused it.
    Fence(FenceError),
    /// Its lease had expired.
    Expired,
    /// Its Canon does not compile into a graph.
    InvalidCanon,
}

/// Why a Canon did not converge within its bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nonconvergence {
    /// The last permitted observation still found something to do.
    Bound,
    /// An observation repeated an earlier one, every effect Settled.
    Oscillation,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// Every Condition Satisfied, no Obligation pending, every effect Settled.
    Converged,
    /// Stopped at the bound or on a repeated observation.
    NonConvergent(Nonconvergence),
    /// Only unknowns remain, with every reason.
    Indeterminate(Vec<(ResourceKey, Reason)>),
    /// Something failed. Known failures and unknown outcomes are kept apart:
    /// a Failed or Rejected Action is known, a TimedOut one is unknown.
    Failed {
        /// Actions whose failure is known.
        failed: Vec<ResourceKey>,
        /// Actions whose outcome is unknown (N10).
        unknown: Vec<ResourceKey>,
    },
    /// A newer Plan replaced the run. Not an Enforce outcome.
    Superseded,
}

/// A refresh owed because a change may have happened (ADR 0009 §4).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Obligation {
    /// The resource to refresh.
    pub target: ResourceKey,
    /// The execution whose change may require it.
    pub cause: EffectKey,
}

/// What the kernel knows about one dispatched effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRecord {
    /// The keys it holds.
    pub keys: BTreeSet<ConflictKey>,
    /// The nodes it disrupts.
    pub disrupts: BTreeSet<Node>,
    /// Whether it can still cause change.
    pub settlement: Settlement,
}

/// One Action of a round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRecord {
    /// Its execution.
    pub key: EffectKey,
    /// What it does.
    pub operation: Operation,
    /// Where it is in its lifecycle.
    pub stage: Stage,
    /// When the current stage times out, if it is live.
    pub deadline: Option<Instant>,
    /// When its effect was reported complete, if it was.
    pub completed_at: Option<Instant>,
    /// The Obligations pending on its resource when it was dispatched, which
    /// its verified success discharges.
    pub discharges: BTreeSet<Obligation>,
}

/// The Actions planned for one iteration, and the graph that orders them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// The iteration.
    pub iteration: u32,
    /// The compiled graph.
    pub graph: Graph,
    /// Every Action vertex's record, by resource.
    pub actions: BTreeMap<ResourceKey, ActionRecord>,
    /// The Indeterminate Assessments the round was planned with.
    pub indeterminate: Vec<(ResourceKey, Reason)>,
}

/// An observation reduced to what the Canon expresses: the collections of
/// the managed resources, sorted. Telemetry outside the Canon never enters.
pub type Fingerprint = BTreeMap<ResourceKey, Vec<Collection>>;

/// Where a run is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for every effect to be Settled before observing.
    Awaiting,
    /// Waiting for Observations collected at or after `since`.
    Observing {
        /// The instant the observation was requested.
        since: Instant,
    },
    /// Executing a round.
    Executing(Round),
    /// Finished.
    Ended(RunOutcome),
}

/// One run of the reconciliation loop for an accepted Plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The Plan.
    pub plan: Plan,
    /// The current iteration, from zero.
    pub iteration: u32,
    /// Where the run is.
    pub phase: Phase,
    /// The fingerprint of every planned observation.
    pub history: Vec<Fingerprint>,
}

/// Why a receipt changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// No execution with this key was ever dispatched.
    Unknown,
    /// The execution's effect is already Settled and released.
    Duplicate,
    /// The receipt implies nothing from the Action's stage.
    Late,
}

/// A fact the kernel records. Applying the Events of a Decision to the
/// previous snapshot gives the next snapshot, and nothing else changes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Time advanced.
    Clock(Instant),
    /// A Plan was not accepted.
    PlanRejected {
        /// The Plan.
        plan: PlanId,
        /// Its generation.
        generation: Generation,
        /// Why.
        reason: Rejection,
    },
    /// The accepted Plan was delivered again.
    PlanRedelivered {
        /// The Plan.
        plan: PlanId,
    },
    /// A Plan was accepted: the fence moves and a new run starts.
    PlanAccepted(Plan),
    /// The run ended.
    RunEnded(RunOutcome),
    /// The run asks for Observations at or after `since`.
    ObservationRequested {
        /// The instant of the request.
        since: Instant,
    },
    /// A round was planned from an observation with this fingerprint.
    RoundPlanned {
        /// The round.
        round: Round,
        /// What was observed.
        fingerprint: Fingerprint,
    },
    /// The round is done; the next iteration waits for settlement.
    IterationAdvanced,
    /// An Obligation was recorded, ahead of the change that may create it.
    ObligationRecorded(Obligation),
    /// An Obligation's cause was verified unchanged or refused.
    ObligationWithdrawn(Obligation),
    /// A verified refresh discharged an Obligation.
    ObligationDischarged(Obligation),
    /// An Action was dispatched.
    ActionDispatched {
        /// The resource.
        resource: ResourceKey,
        /// When a receipt is due.
        deadline: Instant,
        /// The Obligations its success will discharge.
        discharges: BTreeSet<Obligation>,
    },
    /// An Action moved to `stage`.
    ActionAdvanced {
        /// The resource.
        resource: ResourceKey,
        /// The new stage.
        stage: Stage,
        /// The new deadline, if the stage is live.
        deadline: Option<Instant>,
    },
    /// An effect was requested; its intent is recorded before it is issued.
    EffectRequested {
        /// The execution.
        key: EffectKey,
        /// Its record.
        record: EffectRecord,
    },
    /// An effect became Settled.
    EffectSettled {
        /// The execution.
        key: EffectKey,
        /// The evidence.
        by: SettledBy,
    },
    /// A Settled effect whose Action is not live released its reservation.
    EffectReleased {
        /// The execution.
        key: EffectKey,
    },
    /// A receipt changed nothing.
    ReceiptIgnored {
        /// The execution it named.
        key: EffectKey,
        /// The receipt.
        receipt: Receipt,
        /// Why.
        why: Ignored,
    },
    /// The process restarted and replayed its log.
    Recovered,
}

/// The control state: what the Cell knows, never what the host is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelSnapshot {
    now: Instant,
    fence: Fence,
    run: Option<Run>,
    effects: BTreeMap<EffectKey, EffectRecord>,
    released: BTreeSet<EffectKey>,
    obligations: BTreeSet<Obligation>,
}

impl KernelSnapshot {
    /// The initial snapshot: nothing accepted, nothing pending.
    pub fn new() -> Self {
        KernelSnapshot {
            now: Instant(0),
            fence: Fence::new(),
            run: None,
            effects: BTreeMap::new(),
            released: BTreeSet::new(),
            obligations: BTreeSet::new(),
        }
    }

    /// The latest instant seen.
    pub fn now(&self) -> Instant {
        self.now
    }

    /// The fence.
    pub fn fence(&self) -> &Fence {
        &self.fence
    }

    /// The current or last run.
    pub fn run(&self) -> Option<&Run> {
        self.run.as_ref()
    }

    /// How the current run ended, if it has.
    pub fn outcome(&self) -> Option<&RunOutcome> {
        match &self.run.as_ref()?.phase {
            Phase::Ended(outcome) => Some(outcome),
            _ => None,
        }
    }

    /// Every effect that still holds a reservation.
    pub fn effects(&self) -> &BTreeMap<EffectKey, EffectRecord> {
        &self.effects
    }

    /// The keys of every effect that was released: the idempotency ledger.
    pub fn released(&self) -> &BTreeSet<EffectKey> {
        &self.released
    }

    /// The pending Obligations.
    pub fn obligations(&self) -> &BTreeSet<Obligation> {
        &self.obligations
    }

    /// The Actions of the current round, if one is executing.
    pub fn round(&self) -> Option<&Round> {
        match &self.run.as_ref()?.phase {
            Phase::Executing(round) => Some(round),
            _ => None,
        }
    }
}

impl Default for KernelSnapshot {
    fn default() -> Self {
        KernelSnapshot::new()
    }
}

/// What `step` decided: the Events to record, the effects to request, and
/// the snapshot the Events produce. A driver appends the Events before it
/// issues any effect (ADR 0012 §1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The next snapshot.
    pub snapshot: KernelSnapshot,
    /// The Events, in order.
    pub events: Vec<Event>,
    /// The effect requests, in order.
    pub effects: Vec<EffectRequest>,
}

/// Replays `events` from the initial snapshot. Recovery calls this with the
/// Event Log and then steps [`Input::Recovered`].
pub fn replay<'a>(events: impl IntoIterator<Item = &'a Event>) -> KernelSnapshot {
    events.into_iter().fold(KernelSnapshot::new(), apply)
}
