//! Regression tests verifying pre-execution `settle_by` deadline checks in `nomos-app::driver` (#8).

use std::collections::VecDeque;

use nomos_app::driver::{Cell, JournaledCell};
use nomos_app::kernel::{Canon, Event, Input, Managed, Plan, Policy, RunOutcome};
use nomos_core::condition::{Condition, Content, FileCondition, Metadata};
use nomos_core::effect::{Apply, Receipt};
use nomos_core::observation::{
    Collection, Evidence, FileEvidence, Instant, Observation, Provenance, Window,
};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{Digest, ResourceKey, ResourcePath};
use nomos_store::{EventLog, Full};
use nomos_substrate::{Mutate, Observe};

struct MemLog<E>(Vec<E>);

impl<E> Default for MemLog<E> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<E: Clone> EventLog<E> for MemLog<E> {
    fn append(&mut self, events: &[E]) -> Result<(), Full> {
        self.0.extend_from_slice(events);
        Ok(())
    }

    fn events(&self) -> Vec<E> {
        self.0.clone()
    }
}

struct AdvancingClockSubstrate {
    clock_sequence: VecDeque<Instant>,
    current: Instant,
    applied: Vec<Apply>,
}

impl AdvancingClockSubstrate {
    fn new(first: Instant, later: Vec<Instant>) -> Self {
        Self {
            clock_sequence: VecDeque::from(later),
            current: first,
            applied: Vec::new(),
        }
    }
}

impl Observe for AdvancingClockSubstrate {
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        resources
            .iter()
            .filter_map(|key| {
                let win = Window::new(self.current, self.current).ok()?;
                let prov = Provenance::new(nomos_core::observation::CollectorId::new("test")?, win);
                Observation::new(
                    key.clone(),
                    Collection::Collected(Evidence::File(FileEvidence::Absent)),
                    prov,
                )
            })
            .collect()
    }
}

impl Mutate for AdvancingClockSubstrate {
    fn current_instant(&self) -> Option<Instant> {
        Some(self.current)
    }

    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        self.applied.push(request.clone());
        if let Some(next_now) = self.clock_sequence.pop_front() {
            self.current = next_now;
        }
        vec![
            Receipt::Accepted,
            Receipt::Started,
            Receipt::Completed { changed: true },
        ]
    }
}

fn two_independent_files_plan() -> Plan {
    let f1 = ResourcePath::new("/etc/nomos/one.conf").unwrap();
    let f2 = ResourcePath::new("/etc/nomos/two.conf").unwrap();
    let cond = |p: ResourcePath| {
        Condition::file(
            p,
            FileCondition::Present {
                content: Content::Exactly(Digest::from_bytes([0xAB; 32])),
                metadata: Metadata::any(),
            },
        )
    };
    let m1 = Managed {
        condition: cond(f1),
        keys: Default::default(),
        disrupts: Default::default(),
    };
    let m2 = Managed {
        condition: cond(f2),
        keys: Default::default(),
        disrupts: Default::default(),
    };
    Plan {
        id: PlanId::new("deadline-plan").unwrap(),
        generation: Generation(1),
        canon: Canon::new(vec![m1, m2], Vec::new()),
        bound: 2,
        policy: Policy::new(4, 100, 100, 100),
        expires: None,
    }
}

#[test]
fn driver_refuses_second_effect_in_batch_when_live_clock_crosses_settle_by() {
    let start = Instant(1_000);
    // First apply completes and moves live substrate clock past settle_by = 1_100.
    let mut sub = AdvancingClockSubstrate::new(start, vec![Instant(1_200)]);
    let mut cell = Cell::open(MemLog::<Event>::default());
    cell.handle(Input::Tick(start), &mut sub).unwrap();

    let next_inputs = cell
        .handle(Input::Enforce(two_independent_files_plan()), &mut sub)
        .unwrap();
    // Round 0 requests observation first, then planning step executes the two Applies.
    assert!(matches!(next_inputs.first(), Some(Input::Observed(_))));
    let after_obs = cell
        .handle(next_inputs.into_iter().next().unwrap(), &mut sub)
        .unwrap();

    assert_eq!(
        sub.applied.len(),
        1,
        "second effect MUST NOT execute once live clock reaches settle_by"
    );
    assert!(
        after_obs
            .iter()
            .any(|inp| matches!(inp, Input::Receipt(_, Receipt::Refused))),
        "refused receipt must be returned for skipped expired effect"
    );

    // JournaledCell behaves identically.
    let mut jsub = AdvancingClockSubstrate::new(start, vec![Instant(1_200)]);
    let mut jcell = JournaledCell::open(MemLog::<Input>::default());
    jcell.handle(Input::Tick(start), &mut jsub).unwrap();
    let j_obs = jcell
        .handle(Input::Enforce(two_independent_files_plan()), &mut jsub)
        .unwrap();
    let j_receipts = jcell
        .handle(j_obs.into_iter().next().unwrap(), &mut jsub)
        .unwrap();
    assert_eq!(jsub.applied.len(), 1);
    assert!(
        j_receipts
            .iter()
            .any(|inp| matches!(inp, Input::Receipt(_, Receipt::Refused)))
    );
    let _ = RunOutcome::Converged;
}
