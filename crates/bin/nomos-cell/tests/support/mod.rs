//! The deterministic simulator: the transition kernel's `step`, an
//! in-memory Event Log with injectable faults, and the mock host, wired so
//! that a test controls every delivery (ADR 0006 §3, grounding plan
//! `05-transition-kernel`).
//!
//! - **Crashes are dropped receipts.** A crash loses every Observation and
//!   receipt in flight, rebuilds the snapshot by replaying the log, and
//!   steps `Recovered`. Work already handed to the host survives the crash,
//!   as an operating-system job survives the process that started it, and
//!   its receipts go nowhere.
//! - **Delays are reordered inputs.** Deliveries wait in a queue; a test may
//!   deliver them in any order, drop them, duplicate them, or let time pass.
//! - **Supersession is a second authority input.**
//! - **Settle-by is honored by the host.** Host work not done by its
//!   settle-by instant is abandoned and never happens, which is the
//!   Substrate contract the kernel's settlement evidence relies on.
//!
//! The simulator performs no semantics of its own: it carries requests to
//! the mock and results back. Everything decided is decided by `step`.

#![allow(dead_code)]

use std::collections::{BTreeSet, VecDeque};

use nomos_app::kernel::{
    Canon, Decision, Event, Input, KernelSnapshot, Managed, Plan, Policy, RunOutcome, replay, step,
};
use nomos_core::condition::{Condition, Content, FileCondition, Requirement};
use nomos_core::effect::{Apply, EffectKey, EffectRequest};
use nomos_core::observation::Instant;
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{Digest, ResourceKey, ResourcePath};
use nomos_store::{EventLog, Full};
use nomos_substrate::{Mutate, Observe};
use nomos_substrate_mock::{MockHost, Service};
use nomos_warp::graph::{ConflictKey, Edge, EdgeKind};

pub fn p(text: &str) -> ResourcePath {
    ResourcePath::new(text).unwrap()
}

/// The resource at `text`: the legacy service under `/run/`, a file
/// anywhere else, as the scenarios of `05-transition-kernel` drew them.
pub fn k(text: &str) -> ResourceKey {
    if text.starts_with("/run/") {
        ResourceKey::Service(p(text))
    } else {
        ResourceKey::File(p(text))
    }
}

fn managed(path: &str, requirement: FileCondition, conflict: &[&str]) -> Managed {
    let key = k(path);
    let requirement = match key {
        ResourceKey::Service(_) => Requirement::Service(requirement),
        _ => Requirement::File(requirement),
    };
    Managed {
        condition: Condition::new(key, requirement).unwrap(),
        keys: keys(conflict),
        disrupts: BTreeSet::new(),
    }
}

pub fn d(n: u8) -> Digest {
    Digest::from_bytes([n; 32])
}

pub fn keys(names: &[&str]) -> BTreeSet<ConflictKey> {
    names.iter().map(|n| ConflictKey::new(n).unwrap()).collect()
}

/// A file that must hold `digest`.
pub fn file(path: &str, digest: Digest, conflict: &[&str]) -> Managed {
    managed(
        path,
        FileCondition::present(Content::Exactly(digest)),
        conflict,
    )
}

/// A service that must run, observed through its status path.
pub fn service(path: &str, conflict: &[&str]) -> Managed {
    managed(path, FileCondition::present(Content::Any), conflict)
}

/// A service whose loaded configuration revision must be `digest`.
pub fn loaded(path: &str, digest: Digest, conflict: &[&str]) -> Managed {
    managed(
        path,
        FileCondition::present(Content::Exactly(digest)),
        conflict,
    )
}

pub fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
    Edge::new(k(from), k(to), kind)
}

/// Receipts due within 5, verification within 5, effects settled by 20.
pub fn policy() -> Policy {
    Policy::new(4, 5, 5, 20)
}

pub fn plan(id: &str, generation: u64, canon: Canon, bound: u32) -> Plan {
    Plan {
        id: PlanId::new(id).unwrap(),
        generation: Generation(generation),
        canon,
        bound,
        policy: policy(),
        expires: None,
    }
}

/// The refresh scenario's paths and revisions.
pub const CONF: &str = "/etc/svc.conf";
pub const SVC: &str = "/run/svc";
pub const SVC_LOADED: &str = "/run/svc.loaded";
pub const OLD: u8 = 1;
pub const NEW: u8 = 2;

/// A host whose service runs the old configuration, which is on disk, and
/// exposes the revision it loaded.
pub fn refresh_host() -> MockHost {
    let mut host = MockHost::new();
    host.write(&p(CONF), d(OLD));
    host.add_service(
        p(SVC),
        Service {
            config: p(CONF),
            loaded: Some(d(OLD)),
            active: true,
            loaded_path: Some(p(SVC_LOADED)),
        },
    );
    host
}

/// Design 1: the refresh is an Obligation behind an `on_change` edge.
pub fn obligation_canon() -> Canon {
    Canon::new(
        vec![
            file(CONF, d(NEW), &["file:/etc/svc.conf"]),
            service(SVC, &["systemd:svc"]),
        ],
        vec![edge(CONF, SVC, EdgeKind::OnChange)],
    )
}

/// Design 2: the service's loaded revision is a Condition.
pub fn loaded_revision_canon() -> Canon {
    Canon::new(
        vec![
            file(CONF, d(NEW), &["file:/etc/svc.conf"]),
            loaded(SVC_LOADED, d(NEW), &["systemd:svc"]),
        ],
        vec![edge(CONF, SVC_LOADED, EdgeKind::Requires)],
    )
}

/// An in-memory Event Log. `transient` stores no Obligation Events, which
/// turns the durable design into the transient one: the Obligation lives in
/// memory only and dies with the process. `full` refuses every append.
#[derive(Debug, Default, Clone)]
pub struct MemLog {
    events: Vec<Event>,
    pub transient: bool,
    pub full: bool,
}

impl EventLog<Event> for MemLog {
    fn append(&mut self, batch: &[Event]) -> Result<(), Full> {
        if self.full {
            return Err(Full);
        }
        self.events.extend(
            batch
                .iter()
                .filter(|e| {
                    !(self.transient
                        && matches!(
                            e,
                            Event::ObligationRecorded(_)
                                | Event::ObligationWithdrawn(_)
                                | Event::ObligationDischarged(_)
                        ))
                })
                .cloned(),
        );
        Ok(())
    }

    fn events(&self) -> Vec<Event> {
        self.events.clone()
    }
}

/// Something in flight.
#[derive(Debug, Clone)]
pub enum Item {
    /// An input for the kernel.
    Deliver(Input),
    /// Work handed to the host by the process of `epoch`.
    Execute { request: Apply, epoch: u32 },
    /// A read requested by the process of `epoch`.
    Read { paths: Vec<ResourceKey>, epoch: u32 },
}

pub struct Sim {
    pub host: MockHost,
    pub log: MemLog,
    pub snapshot: KernelSnapshot,
    pub now: u64,
    pub queue: VecDeque<Item>,
    pub epoch: u32,
    pub issued: Vec<EffectKey>,
    pub decisions: Vec<Decision>,
    pub crashes: u32,
    /// A resource outside the Canon whose Observation changes every time,
    /// added to every read: telemetry the fingerprint must ignore.
    pub telemetry: Option<ResourcePath>,
}

impl Sim {
    pub fn new(host: MockHost) -> Self {
        Sim {
            host,
            log: MemLog::default(),
            snapshot: KernelSnapshot::new(),
            now: 0,
            queue: VecDeque::new(),
            epoch: 0,
            issued: Vec::new(),
            decisions: Vec::new(),
            crashes: 0,
            telemetry: None,
        }
    }

    /// Steps the kernel, appends its Events, then queues its effects.
    pub fn step(&mut self, input: Input) {
        self.step_with(input, true);
    }

    /// Steps the kernel and appends its Events; issues the effects only if
    /// `issue`. A crash between append and issue is `issue = false`.
    pub fn step_with(&mut self, input: Input, issue: bool) {
        let decision = step(&self.snapshot, input);
        if self.log.append(&decision.events).is_err() {
            return;
        }
        self.snapshot = decision.snapshot.clone();
        if !self.log.transient {
            assert_eq!(
                self.snapshot,
                replay(&self.log.events()),
                "the snapshot is the fold of the log"
            );
        }
        if issue {
            for effect in &decision.effects {
                match effect {
                    EffectRequest::Observe(paths) => self.queue.push_back(Item::Read {
                        paths: paths.clone(),
                        epoch: self.epoch,
                    }),
                    EffectRequest::Apply(request) => {
                        assert!(
                            !self.issued.contains(&request.key),
                            "an execution is requested once"
                        );
                        self.issued.push(request.key.clone());
                        self.queue.push_back(Item::Execute {
                            request: request.clone(),
                            epoch: self.epoch,
                        });
                    }
                }
            }
        }
        self.decisions.push(decision);
    }

    /// Delivers one item.
    pub fn deliver(&mut self, item: Item) {
        match item {
            Item::Deliver(input) => self.step(input),
            Item::Execute { request, epoch } => {
                if Instant(self.now) >= request.settle_by {
                    return;
                }
                let receipts = self.host.apply(&request);
                if epoch == self.epoch {
                    for receipt in receipts {
                        self.queue
                            .push_back(Item::Deliver(Input::Receipt(request.key.clone(), receipt)));
                    }
                }
            }
            Item::Read { paths, epoch } => {
                if epoch == self.epoch {
                    let mut observed = self.host.observe(&paths);
                    if let Some(path) = self.telemetry.clone() {
                        self.host
                            .write(&path, Digest::from_bytes([(self.now % 250) as u8; 32]));
                        observed.extend(self.host.observe(&[ResourceKey::File(path)]));
                    }
                    self.queue
                        .push_back(Item::Deliver(Input::Observed(observed)));
                }
            }
        }
    }

    /// Delivers the oldest item; false when nothing is in flight.
    pub fn next(&mut self) -> bool {
        match self.queue.pop_front() {
            Some(item) => {
                self.deliver(item);
                true
            }
            None => false,
        }
    }

    /// Lets `dt` pass.
    pub fn tick(&mut self, dt: u64) {
        self.now += dt;
        self.host.set_time(Instant(self.now));
        self.step(Input::Tick(Instant(self.now)));
    }

    pub fn enforce(&mut self, plan: Plan) {
        self.step(Input::Enforce(plan));
    }

    /// How the current run ended, if it has.
    pub fn outcome(&self) -> Option<RunOutcome> {
        self.snapshot.outcome().cloned()
    }

    /// Delivers everything, letting time pass when nothing is in flight,
    /// until the run ends.
    pub fn run(&mut self) -> RunOutcome {
        for _ in 0..10_000 {
            if let Some(outcome) = self.outcome()
                && self.queue.is_empty()
            {
                return outcome;
            }
            if !self.next() {
                self.tick(1);
            }
        }
        panic!(
            "the run did not end: {:?}",
            self.snapshot.run().map(|r| &r.phase)
        );
    }

    /// The process restarts: everything in flight to it is lost, the host's
    /// work goes on, and the snapshot is rebuilt from the log.
    pub fn crash(&mut self) {
        self.crashes += 1;
        self.epoch += 1;
        self.queue
            .retain(|item| matches!(item, Item::Execute { .. }));
        self.snapshot = replay(&self.log.events());
        self.step(Input::Recovered);
    }

    /// Lets every effect's settle-by instant pass and the host finish.
    pub fn quiesce(&mut self) {
        for _ in 0..30 {
            self.tick(1);
            while self.next() {}
        }
    }

    /// Whether the service loaded what is on disk.
    pub fn refresh_consumed(&self) -> bool {
        let service = self.host.service(&p(SVC)).unwrap();
        service.loaded == self.host.file(&p(CONF))
    }

    /// Refreshes that reloaded what the service had already loaded.
    pub fn unnecessary_refreshes(&self) -> usize {
        self.host
            .refreshes()
            .iter()
            .filter(|(_, before, after)| before == after)
            .count()
    }
}
