//! Experiment `controller-composition` (grounding plan, after
//! `05-transition-kernel`): do explicit footprints with stated guarantees
//! and reliance assumptions detect a harmful composition of two controllers
//! that each converge alone?
//!
//! A controller here is one transition kernel, `nomos_app::kernel::step`,
//! with its own Canon, Event Log, fence, and queue of deliveries. Every
//! controller writes one shared `MockHost`. The harness carries requests to
//! the mock and results back, as the `05-transition-kernel` simulator does;
//! it decides nothing a kernel decides. What it adds is a view across
//! controllers: the joint projection of every composed Canon, taken after
//! each sweep of runs, which the per-run oscillation diagnostic cannot see.
//!
//! The configurations are disjoint, single-owner, and shared, on a file;
//! the check is `nomos_core::footprint::compose` under the working
//! definitions of ADR 0008's 2026-09-29 note. The negative control is the
//! shared configuration: it must oscillate on the mock, and the check must
//! reject it.
//!
//! This file does not use `tests/support/mod.rs`: its simulator drives one
//! kernel, and the helpers it needs are few enough to repeat.

use std::collections::{BTreeSet, VecDeque};

use nomos_app::kernel::{
    Canon, Event, Input, KernelSnapshot, Kind, Managed, Plan, Policy, RunOutcome, replay, step,
};
use nomos_core::action::{Failure, Stage};
use nomos_core::condition::{Condition, Content, FileCondition};
use nomos_core::effect::{Apply, EffectRequest, Operation};
use nomos_core::footprint::{ControllerId, Footprint, Interference, Property, compose};
use nomos_core::observation::{Collection, Instant};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{Digest, ResourcePath};
use nomos_substrate::{Mutate, Observe};
use nomos_substrate_mock::{MockHost, Service};

// ---------------------------------------------------------------------------
// Scenario

const SHARED: &str = "/etc/shared.conf";
const A_ONLY: &str = "/etc/a.conf";
const B_ONLY: &str = "/etc/b.conf";
const SVC: &str = "/run/svc";
const OLD: u8 = 1;
const A_WANTS: u8 = 2;
const B_WANTS: u8 = 3;
/// The most executions one run may perform.
const BOUND: u32 = 3;
/// The most sweeps the harness runs before it reports the bound.
const SWEEPS: usize = 6;
/// Controller B's start is delayed by 0 to `OFFSETS` harness steps.
const OFFSETS: usize = 24;

fn p(text: &str) -> ResourcePath {
    ResourcePath::new(text).unwrap()
}

fn d(n: u8) -> Digest {
    Digest::from_bytes([n; 32])
}

fn id(name: &str) -> ControllerId {
    ControllerId::new(name).unwrap()
}

/// A file that must hold `digest`.
fn file(path: &str, digest: u8) -> Managed {
    Managed {
        condition: Condition::file(
            p(path),
            FileCondition::Present {
                content: Content::Exactly(d(digest)),
            },
        ),
        kind: Kind::File,
        keys: BTreeSet::new(),
        disrupts: BTreeSet::new(),
    }
}

/// A service that must run, observed through its status path; its Action
/// starts it, which loads its configuration file.
fn running(path: &str) -> Managed {
    Managed {
        condition: Condition::file(
            p(path),
            FileCondition::Present {
                content: Content::Any,
            },
        ),
        kind: Kind::Service,
        keys: BTreeSet::new(),
        disrupts: BTreeSet::new(),
    }
}

/// The property an Action on `path` of `kind` writes.
fn written(path: &ResourcePath, kind: Kind) -> Property {
    match kind {
        Kind::File => Property::file_content(path),
        Kind::Service => Property::new(&format!("service:{path}")).unwrap(),
    }
}

/// The property an execution wrote, from what the mock performed.
fn executed(apply: &Apply) -> Property {
    let kind = match apply.operation {
        Operation::Replace(_) => Kind::File,
        Operation::Refresh => Kind::Service,
    };
    written(apply.key.resource(), kind)
}

/// The host every configuration starts from: three files at the old
/// revision, and a stopped service that reads the shared file.
fn host() -> MockHost {
    let mut host = MockHost::new();
    for path in [SHARED, A_ONLY, B_ONLY] {
        host.write(&p(path), d(OLD));
    }
    host.add_service(
        p(SVC),
        Service {
            config: p(SHARED),
            loaded: None,
            active: false,
            loaded_path: None,
        },
    );
    host
}

/// Controller `name` over `resources`. Its guarantee is derived from the
/// Canon: every managed resource's property. `reads` and `relies` are
/// declared by hand.
fn controller(name: &str, resources: Vec<Managed>, reads: &[&str], relies: &[&str]) -> Controller {
    let canon = Canon::new(resources, vec![]);
    let mut footprint = Footprint::new(id(name));
    for managed in canon.resources().values() {
        footprint = footprint.writing(written(managed.path(), managed.kind));
    }
    for path in reads {
        footprint = footprint.reading(Property::file_content(&p(path)));
    }
    for path in relies {
        footprint = footprint.relying_on(Property::file_content(&p(path)));
    }
    Controller {
        name: name.to_owned(),
        canon,
        footprint,
        snapshot: KernelSnapshot::new(),
        log: Vec::new(),
        queue: VecDeque::new(),
        generation: 0,
        runs: Vec::new(),
    }
}

/// A manages its own file; B manages another.
fn disjoint() -> Vec<Controller> {
    vec![
        controller("a", vec![file(A_ONLY, A_WANTS)], &[], &[]),
        controller("b", vec![file(B_ONLY, B_WANTS)], &[], &[]),
    ]
}

/// A owns the shared file; B keeps the service running, and starting it
/// reads the shared file.
fn single_owner() -> Vec<Controller> {
    vec![
        controller("a", vec![file(SHARED, A_WANTS)], &[], &[]),
        controller("b", vec![running(SVC)], &[SHARED], &[]),
    ]
}

/// A and B both manage the shared file, with different content.
fn shared() -> Vec<Controller> {
    vec![
        controller("a", vec![file(SHARED, A_WANTS)], &[], &[]),
        controller("b", vec![file(SHARED, B_WANTS)], &[], &[]),
    ]
}

fn footprints(controllers: &[Controller]) -> Vec<Footprint> {
    controllers.iter().map(|c| c.footprint.clone()).collect()
}

// ---------------------------------------------------------------------------
// The harness

#[derive(Debug, Clone)]
enum Item {
    Deliver(Input),
    Execute(Apply),
    Read(Vec<ResourcePath>),
}

struct Controller {
    name: String,
    canon: Canon,
    footprint: Footprint,
    snapshot: KernelSnapshot,
    log: Vec<Event>,
    queue: VecDeque<Item>,
    generation: u64,
    /// How each finished run ended, in order.
    runs: Vec<RunOutcome>,
}

impl Controller {
    fn ended(&self) -> Option<RunOutcome> {
        if !self.queue.is_empty() {
            return None;
        }
        self.snapshot.outcome().cloned()
    }
}

/// How a sequence of sweeps ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Joint {
    /// A sweep in which every run ended Converged and nothing executed:
    /// every Canon holds on the host at once.
    FixedPoint { sweeps: usize },
    /// A sweep that executed returned the joint projection to one an
    /// earlier sweep ended with.
    Oscillation { sweeps: usize },
    /// Neither, within the bound.
    Bound,
}

/// Controllers sharing one mock host.
struct Composition {
    host: MockHost,
    now: u64,
    controllers: Vec<Controller>,
    /// Every execution the mock performed: which controller, what property.
    writes: Vec<(usize, Property)>,
}

impl Composition {
    /// The controllers on `host`, with no check.
    fn unchecked(host: MockHost, controllers: Vec<Controller>) -> Self {
        Composition {
            host,
            now: 0,
            controllers,
            writes: Vec::new(),
        }
    }

    /// The controllers on `host`, if their footprints compose. A rejected
    /// composition never reaches the host.
    fn admitted(host: MockHost, controllers: Vec<Controller>) -> Result<Self, Vec<Interference>> {
        compose(&footprints(&controllers))?;
        Ok(Composition::unchecked(host, controllers))
    }

    fn step(&mut self, c: usize, input: Input) {
        let controller = &mut self.controllers[c];
        let decision = step(&controller.snapshot, input);
        controller.log.extend(decision.events.iter().cloned());
        controller.snapshot = decision.snapshot;
        assert_eq!(
            controller.snapshot,
            replay(&controller.log),
            "the snapshot is the fold of the log"
        );
        for effect in decision.effects {
            controller.queue.push_back(match effect {
                EffectRequest::Observe(paths) => Item::Read(paths),
                EffectRequest::Apply(request) => Item::Execute(request),
            });
        }
    }

    fn deliver(&mut self, c: usize, item: Item) {
        match item {
            Item::Deliver(input) => self.step(c, input),
            Item::Execute(request) => {
                if Instant(self.now) >= request.settle_by {
                    return;
                }
                let before = self.host.executions().len();
                let receipts = self.host.apply(&request);
                if self.host.executions().len() > before {
                    self.writes.push((c, executed(&request)));
                }
                for receipt in receipts {
                    self.controllers[c]
                        .queue
                        .push_back(Item::Deliver(Input::Receipt(request.key.clone(), receipt)));
                }
            }
            Item::Read(paths) => {
                let observed = self.host.observe(&paths);
                self.controllers[c]
                    .queue
                    .push_back(Item::Deliver(Input::Observed(observed)));
            }
        }
    }

    fn tick(&mut self) {
        self.now += 1;
        self.host.set_time(Instant(self.now));
        for c in 0..self.controllers.len() {
            self.step(c, Input::Tick(Instant(self.now)));
        }
    }

    /// Presents controller `c` a new Plan for its Canon.
    fn enforce(&mut self, c: usize) {
        let controller = &mut self.controllers[c];
        controller.generation += 1;
        let plan = Plan {
            id: PlanId::new(&format!("{}-{}", controller.name, controller.generation)).unwrap(),
            generation: Generation(controller.generation),
            canon: controller.canon.clone(),
            bound: BOUND,
            policy: Policy::new(4, 5, 5, 20),
            expires: None,
        };
        self.step(c, Input::Enforce(plan));
    }

    fn finish(&mut self, c: usize) -> RunOutcome {
        let outcome = self.controllers[c].ended().unwrap();
        self.controllers[c].runs.push(outcome.clone());
        outcome
    }

    /// Runs controller `c`'s current run to its end, with every other
    /// controller idle.
    fn run_alone(&mut self, c: usize) -> RunOutcome {
        for _ in 0..10_000 {
            if self.controllers[c].ended().is_some() {
                return self.finish(c);
            }
            match self.controllers[c].queue.pop_front() {
                Some(item) => self.deliver(c, item),
                None => self.tick(),
            }
        }
        panic!("run of {} did not end", self.controllers[c].name);
    }

    /// Starts controller 0, then every other controller `offset` harness
    /// steps later, and delivers one item per controller in turn until every
    /// run has ended. Also says whether the runs overlapped: whether the
    /// others started before controller 0's run ended.
    fn run_together(&mut self, offset: usize) -> (Vec<RunOutcome>, bool) {
        let n = self.controllers.len();
        self.enforce(0);
        let mut started = false;
        let mut overlapped = false;
        for steps in 0..10_000 {
            if !started && steps >= offset {
                overlapped = self.controllers[0].ended().is_none();
                for c in 1..n {
                    self.enforce(c);
                }
                started = true;
            }
            if started && self.controllers.iter().all(|c| c.ended().is_some()) {
                return ((0..n).map(|c| self.finish(c)).collect(), overlapped);
            }
            let mut delivered = false;
            for c in 0..n {
                if let Some(item) = self.controllers[c].queue.pop_front() {
                    self.deliver(c, item);
                    delivered = true;
                }
            }
            if !delivered {
                self.tick();
            }
        }
        panic!("the runs did not end");
    }

    /// Every composed Canon's resources, observed: the fingerprint across
    /// controllers.
    fn projection(&mut self) -> Vec<(ResourcePath, Collection)> {
        let paths: BTreeSet<ResourcePath> = self
            .controllers
            .iter()
            .flat_map(|c| c.canon.paths())
            .collect();
        let paths: Vec<ResourcePath> = paths.into_iter().collect();
        self.host
            .observe(&paths)
            .iter()
            .map(|o| (o.path().clone(), *o.collection()))
            .collect()
    }

    /// Sweeps: each controller in `order` enforces a new Plan and runs it to
    /// its end alone. Stops at a joint fixed point, at a repeated joint
    /// projection after a sweep that executed, or after `max` sweeps.
    fn settle(&mut self, order: &[usize], max: usize) -> Joint {
        let mut seen = vec![self.projection()];
        for sweep in 1..=max {
            let before = self.writes.len();
            let mut converged = true;
            for &c in order {
                self.enforce(c);
                converged &= self.run_alone(c) == RunOutcome::Converged;
            }
            let executed = self.writes.len() > before;
            if converged && !executed {
                return Joint::FixedPoint { sweeps: sweep };
            }
            let now = self.projection();
            if executed && seen.contains(&now) {
                return Joint::Oscillation { sweeps: sweep };
            }
            seen.push(now);
        }
        Joint::Bound
    }

    /// Whether every execution wrote a property its controller's guarantee
    /// names.
    fn guarantees_held(&self) -> bool {
        self.writes
            .iter()
            .all(|(c, property)| self.controllers[*c].footprint.writes().contains(property))
    }

    /// Who wrote, in order.
    fn writers(&self) -> Vec<&str> {
        self.writes
            .iter()
            .map(|(c, _)| self.controllers[*c].name.as_str())
            .collect()
    }

    /// Every run outcome of every controller.
    fn outcomes(&self) -> Vec<RunOutcome> {
        self.controllers
            .iter()
            .flat_map(|c| c.runs.iter().cloned())
            .collect()
    }
}

/// Whether the log of controller `c` records an Action that failed because
/// its postcondition did not hold at verification.
fn failed_on_postcondition(composition: &Composition, c: usize) -> bool {
    composition.controllers[c].log.iter().any(|e| {
        matches!(
            e,
            Event::ActionAdvanced {
                stage: Stage::Failed(Failure::Postcondition(_)),
                ..
            }
        )
    })
}

// ---------------------------------------------------------------------------
// Each controller alone

/// Every controller of every configuration converges alone: its first sweep
/// executes, its second executes nothing.
#[test]
fn each_controller_converges_alone() {
    for configuration in [disjoint(), single_owner(), shared()] {
        for controller in configuration {
            let name = controller.name.clone();
            let mut alone = Composition::unchecked(host(), vec![controller]);
            assert_eq!(
                alone.settle(&[0], SWEEPS),
                Joint::FixedPoint { sweeps: 2 },
                "{name}"
            );
            assert_eq!(alone.writes.len(), 1, "{name}");
            assert!(alone.guarantees_held(), "{name}");
            assert!(
                alone.outcomes().iter().all(|o| *o == RunOutcome::Converged),
                "{name}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The safe compositions

/// Disjoint: accepted, and in either order the pair reaches a joint fixed
/// point in two sweeps, every run Converged.
#[test]
fn the_disjoint_configuration_is_accepted_and_converges_together() {
    assert_eq!(compose(&footprints(&disjoint())), Ok(()));
    for order in [[0, 1], [1, 0]] {
        let mut together = Composition::admitted(host(), disjoint()).unwrap();
        assert_eq!(
            together.settle(&order, SWEEPS),
            Joint::FixedPoint { sweeps: 2 }
        );
        assert_eq!(together.writes.len(), 2);
        assert!(together.guarantees_held());
        assert!(
            together
                .outcomes()
                .iter()
                .all(|o| *o == RunOutcome::Converged)
        );
        assert_eq!(together.host.file(&p(A_ONLY)), Some(d(A_WANTS)));
        assert_eq!(together.host.file(&p(B_ONLY)), Some(d(B_WANTS)));
    }
}

/// Single-owner: A writes the shared file, B only reads it by starting the
/// service. Accepted, and in either order a joint fixed point in two
/// sweeps. The order decides which revision the service loaded, which
/// neither Canon expresses.
#[test]
fn the_single_owner_configuration_is_accepted_and_converges_together() {
    assert_eq!(compose(&footprints(&single_owner())), Ok(()));
    for (order, loaded) in [([0, 1], A_WANTS), ([1, 0], OLD)] {
        let mut together = Composition::admitted(host(), single_owner()).unwrap();
        assert_eq!(
            together.settle(&order, SWEEPS),
            Joint::FixedPoint { sweeps: 2 },
            "{order:?}"
        );
        assert_eq!(together.writes.len(), 2);
        assert!(together.guarantees_held());
        assert!(
            together
                .outcomes()
                .iter()
                .all(|o| *o == RunOutcome::Converged)
        );
        assert_eq!(together.host.file(&p(SHARED)), Some(d(A_WANTS)));
        assert_eq!(
            together.host.service(&p(SVC)).unwrap().loaded,
            Some(d(loaded)),
            "{order:?}"
        );
    }
}

/// Both accepted configurations, run concurrently with B's start delayed by
/// every offset from 0 to `OFFSETS`: every run ends Converged, and a
/// following sweep executes nothing.
#[test]
fn accepted_configurations_converge_at_every_start_offset() {
    for (label, configuration) in [
        ("disjoint", disjoint as fn() -> Vec<Controller>),
        ("single-owner", single_owner),
    ] {
        let mut overlapping = 0;
        for offset in 0..=OFFSETS {
            let mut together = Composition::admitted(host(), configuration()).unwrap();
            let (outcomes, overlapped) = together.run_together(offset);
            overlapping += usize::from(overlapped);
            assert_eq!(
                outcomes,
                vec![RunOutcome::Converged, RunOutcome::Converged],
                "{label}, offset {offset}"
            );
            assert_eq!(
                together.settle(&[0, 1], SWEEPS),
                Joint::FixedPoint { sweeps: 1 },
                "{label}, offset {offset}"
            );
            assert!(together.guarantees_held());
        }
        println!("{label}, offsets 0..={OFFSETS}: overlapping {overlapping}");
        assert_eq!(
            overlapping, 10,
            "{label}: offsets 0 to 9 start B before A's run ends"
        );
    }
}

// ---------------------------------------------------------------------------
// The counterexample

/// The negative control, and semantic mutant `SM-COMPOSE-001`'s named test.
/// Shared: the check rejects the pair before execution with a shared write
/// on the file. Run without the check, the pair oscillates: every sweep
/// rewrites the file twice, the joint projection repeats at the second
/// sweep, and every one of the four runs ends Converged, so no kernel sees
/// the oscillation.
#[test]
fn the_shared_configuration_oscillates_and_the_check_rejects_it() {
    let rejected = Composition::admitted(host(), shared()).err();
    assert_eq!(
        rejected,
        Some(vec![Interference::SharedWrite {
            property: Property::file_content(&p(SHARED)),
            writers: (id("a"), id("b")),
        }]),
        "the check rejects the shared configuration"
    );

    let mut together = Composition::unchecked(host(), shared());
    assert_eq!(
        together.settle(&[0, 1], SWEEPS),
        Joint::Oscillation { sweeps: 2 }
    );
    assert_eq!(together.writers(), ["a", "b", "a", "b"]);
    assert!(
        together.guarantees_held(),
        "each writes only what it declared"
    );
    assert_eq!(
        together.outcomes(),
        vec![RunOutcome::Converged; 4],
        "each run ends Converged: the per-run diagnostic sees nothing"
    );
    assert_eq!(together.host.file(&p(SHARED)), Some(d(B_WANTS)));
}

/// Shared, concurrently, with B's start delayed by every offset from 0 to
/// `OFFSETS`. Measured, and pinned as a regression: what each offset's runs
/// report, and that the sweeps that follow oscillate every time. No run
/// ends NonConvergent: where the kernel sees the other writer at all, it
/// sees a postcondition that did not hold at verification.
#[test]
fn shared_runs_oscillate_at_every_start_offset() {
    let mut a_failed = 0;
    let mut b_failed = 0;
    let mut both_converged = 0;
    let mut overlapping = 0;
    for offset in 0..=OFFSETS {
        let mut together = Composition::unchecked(host(), shared());
        let (outcomes, overlapped) = together.run_together(offset);
        overlapping += usize::from(overlapped);
        for (c, outcome) in outcomes.iter().enumerate() {
            match outcome {
                RunOutcome::Converged => {}
                RunOutcome::Failed { failed, unknown } => {
                    assert_eq!(failed, &vec![p(SHARED)], "offset {offset}");
                    assert!(unknown.is_empty(), "offset {offset}");
                    assert!(failed_on_postcondition(&together, c), "offset {offset}");
                }
                other => panic!("offset {offset}: controller {c} ended {other:?}"),
            }
        }
        match (&outcomes[0], &outcomes[1]) {
            (RunOutcome::Converged, RunOutcome::Converged) => both_converged += 1,
            (RunOutcome::Failed { .. }, RunOutcome::Converged) => a_failed += 1,
            (RunOutcome::Converged, RunOutcome::Failed { .. }) => b_failed += 1,
            (a, b) => panic!("offset {offset}: {a:?}, {b:?}"),
        }
        let joint = together.settle(&[0, 1], SWEEPS);
        assert!(
            matches!(joint, Joint::Oscillation { sweeps } if sweeps <= 2),
            "offset {offset}: {joint:?}"
        );
    }
    println!(
        "shared, offsets 0..={OFFSETS}: overlapping {overlapping}, a failed {a_failed}, \
         b failed {b_failed}, both converged {both_converged}"
    );
    assert_eq!(a_failed + b_failed + both_converged, OFFSETS + 1);
    assert_eq!((a_failed, b_failed, both_converged), (4, 2, 19));
    assert_eq!(
        overlapping, 10,
        "offsets 0 to 9 start B before A's run ends"
    );
}

// ---------------------------------------------------------------------------
// What the check reads

/// The check is only as complete as the declarations. B in the
/// single-owner configuration, declaring reliance on the shared file
/// instead of reading it, is rejected, although the same pair converges
/// together on the mock.
#[test]
fn a_declared_reliance_rejects_a_pair_that_converges() {
    let relying = || {
        vec![
            controller("a", vec![file(SHARED, A_WANTS)], &[], &[]),
            controller("b", vec![running(SVC)], &[], &[SHARED]),
        ]
    };
    assert_eq!(
        compose(&footprints(&relying())),
        Err(vec![Interference::RelianceViolated {
            property: Property::file_content(&p(SHARED)),
            relying: id("b"),
            writer: id("a"),
        }])
    );
    let mut together = Composition::unchecked(host(), relying());
    assert_eq!(
        together.settle(&[0, 1], SWEEPS),
        Joint::FixedPoint { sweeps: 2 }
    );
}

/// The check reads declarations, not effects. B's footprint omits the
/// shared file its Canon writes, so the check accepts the pair; on the mock
/// the pair oscillates, and only the harness's audit of executions against
/// guarantees sees that B broke its guarantee.
#[test]
fn an_undeclared_write_passes_the_check_and_the_execution_audit_sees_it() {
    let mut lying = shared();
    lying[1].footprint = Footprint::new(id("b")).writing(Property::file_content(&p(B_ONLY)));
    assert_eq!(compose(&footprints(&lying)), Ok(()));
    let mut together = Composition::unchecked(host(), lying);
    assert_eq!(
        together.settle(&[0, 1], SWEEPS),
        Joint::Oscillation { sweeps: 2 }
    );
    assert!(!together.guarantees_held());
}
