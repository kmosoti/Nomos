//! # nomos-substrate-mock
//!
//! **Driven adapter** for the `nomos-substrate` port. A deterministic
//! in-memory machine — a first-class backend for testing reconciliation,
//! not throwaway scaffolding. It supplies Observations and effect receipts to
//! the same core semantics the Linux adapter feeds.
//!
//! The machine holds files and services. A service has a configuration file
//! on disk and, separately, the revision it loaded when it last started, so
//! replacing the file does not change what the service runs until it is
//! refreshed (grounding plan, `05-transition-kernel`). A service is observed
//! through its status path, present while it runs; it can also expose the
//! revision it loaded at a second path, for the design in which that
//! revision is a Condition.
//!
//! Faults are scripted: a denied read, a failed or refused execution, and a
//! writer outside Nomos. Time is set by the caller, never read. Executions
//! are deduplicated by idempotency key, as the port requires.

use std::collections::{BTreeMap, BTreeSet};

use nomos_core::condition::{Content, FileCondition};
use nomos_core::effect::{Apply, EffectKey, Operation, Receipt};
use nomos_core::observation::{
    Collection, CollectionFailure, CollectorId, FileEvidence, Instant, Observation, Provenance,
    Window,
};
use nomos_core::resource::{Digest, ResourcePath};
use nomos_substrate::{Mutate, Observe};

/// The digest the mock gives an empty file and a running service's status.
pub const EMPTY: Digest = Digest::from_bytes([0; 32]);

/// A service: where its configuration is, what it loaded, whether it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// The configuration file it reads on start.
    pub config: ResourcePath,
    /// The configuration digest it loaded when it last started.
    pub loaded: Option<Digest>,
    /// Whether it runs.
    pub active: bool,
    /// Where it exposes the revision it loaded, if it does.
    pub loaded_path: Option<ResourcePath>,
}

/// A scripted fault for the next execution on a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The execution runs and fails; nothing changes.
    Fail,
    /// The request is refused; nothing starts.
    Refuse,
}

/// The in-memory machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockHost {
    now: Instant,
    files: BTreeMap<ResourcePath, Digest>,
    services: BTreeMap<ResourcePath, Service>,
    denied: BTreeSet<ResourcePath>,
    faults: BTreeMap<ResourcePath, Fault>,
    ledger: BTreeMap<EffectKey, Vec<Receipt>>,
    executions: Vec<(EffectKey, Operation)>,
    refreshes: Vec<(ResourcePath, Option<Digest>, Option<Digest>)>,
}

impl Default for MockHost {
    fn default() -> Self {
        MockHost::new()
    }
}

impl MockHost {
    /// An empty machine at instant zero.
    pub fn new() -> Self {
        MockHost {
            now: Instant(0),
            files: BTreeMap::new(),
            services: BTreeMap::new(),
            denied: BTreeSet::new(),
            faults: BTreeMap::new(),
            ledger: BTreeMap::new(),
            executions: Vec::new(),
            refreshes: Vec::new(),
        }
    }

    /// Sets the machine's clock.
    pub fn set_time(&mut self, now: Instant) {
        self.now = now;
    }

    /// Writes a file as something other than Nomos would.
    pub fn write(&mut self, path: &ResourcePath, digest: Digest) {
        self.files.insert(path.clone(), digest);
    }

    /// Removes a file as something other than Nomos would.
    pub fn remove(&mut self, path: &ResourcePath) {
        self.files.remove(path);
    }

    /// The digest of the file at `path`, if one exists.
    pub fn file(&self, path: &ResourcePath) -> Option<Digest> {
        self.files.get(path).copied()
    }

    /// Adds a service observed at `status`.
    pub fn add_service(&mut self, status: ResourcePath, service: Service) {
        self.services.insert(status, service);
    }

    /// The service observed at `status`.
    pub fn service(&self, status: &ResourcePath) -> Option<&Service> {
        self.services.get(status)
    }

    /// Denies or allows reads of `path`.
    pub fn deny(&mut self, path: &ResourcePath, denied: bool) {
        if denied {
            self.denied.insert(path.clone());
        } else {
            self.denied.remove(path);
        }
    }

    /// Scripts a fault for the next execution on `path`.
    pub fn fault(&mut self, path: &ResourcePath, fault: Fault) {
        self.faults.insert(path.clone(), fault);
    }

    /// Every execution performed, in order. A deduplicated request is not
    /// one.
    pub fn executions(&self) -> &[(EffectKey, Operation)] {
        &self.executions
    }

    /// Every refresh performed: the service, what it had loaded before, and
    /// what it loaded.
    pub fn refreshes(&self) -> &[(ResourcePath, Option<Digest>, Option<Digest>)] {
        &self.refreshes
    }

    fn provenance(&self) -> Provenance {
        let collector = CollectorId::new("mock").expect("a collector name is not empty");
        let window = Window::new(self.now, self.now).expect("a window may start and end together");
        Provenance::new(collector, window)
    }

    fn evidence(&self, path: &ResourcePath) -> Collection {
        if self.denied.contains(path) {
            return Collection::Failed(CollectionFailure::PermissionDenied);
        }
        if let Some(service) = self.services.get(path) {
            return Collection::Collected(if service.active {
                FileEvidence::Present {
                    digest: EMPTY,
                    size: 0,
                }
            } else {
                FileEvidence::Absent
            });
        }
        if let Some(service) = self
            .services
            .values()
            .find(|s| s.loaded_path.as_ref() == Some(path))
        {
            return Collection::Collected(match (service.active, service.loaded) {
                (true, Some(digest)) => FileEvidence::Present { digest, size: 1 },
                _ => FileEvidence::Absent,
            });
        }
        Collection::Collected(match self.files.get(path) {
            Some(digest) => FileEvidence::Present {
                digest: *digest,
                size: 1,
            },
            None => FileEvidence::Absent,
        })
    }

    fn execute(&mut self, path: &ResourcePath, operation: &Operation) -> Receipt {
        match operation {
            Operation::Replace(requirement) => {
                let before = self.files.get(path).copied();
                let after = match requirement {
                    FileCondition::Absent => None,
                    FileCondition::Present {
                        content: Content::Exactly(digest),
                    } => Some(*digest),
                    FileCondition::Present {
                        content: Content::Any,
                    } => Some(before.unwrap_or(EMPTY)),
                };
                match after {
                    Some(digest) => self.files.insert(path.clone(), digest),
                    None => self.files.remove(path),
                };
                Receipt::Completed {
                    changed: before != after,
                }
            }
            Operation::Refresh => {
                let status = self
                    .services
                    .iter()
                    .find(|(status, s)| *status == path || s.loaded_path.as_ref() == Some(path))
                    .map(|(status, _)| status.clone());
                let Some(status) = status else {
                    return Receipt::Failed;
                };
                let config = self.services[&status].config.clone();
                let disk = self.files.get(&config).copied();
                if let Some(service) = self.services.get_mut(&status) {
                    let before = service.loaded;
                    service.loaded = disk;
                    service.active = true;
                    self.refreshes.push((status, before, disk));
                }
                Receipt::Completed { changed: true }
            }
        }
    }
}

impl Observe for MockHost {
    fn observe(&mut self, resources: &[ResourcePath]) -> Vec<Observation> {
        resources
            .iter()
            .map(|path| Observation::file(path.clone(), self.evidence(path), self.provenance()))
            .collect()
    }
}

impl Mutate for MockHost {
    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        if let Some(receipts) = self.ledger.get(&request.key) {
            return receipts.clone();
        }
        let path = request.key.resource().clone();
        let refreshable = self
            .services
            .iter()
            .any(|(status, s)| *status == path || s.loaded_path.as_ref() == Some(&path));
        if request.operation == Operation::Refresh && !refreshable {
            // Nothing to refresh: an operation the machine cannot perform is
            // refused before any effect (substrate-contract.md, S7).
            self.ledger
                .insert(request.key.clone(), vec![Receipt::Refused]);
            return vec![Receipt::Refused];
        }
        let receipts = match self.faults.remove(&path) {
            Some(Fault::Refuse) => vec![Receipt::Refused],
            Some(Fault::Fail) => vec![Receipt::Accepted, Receipt::Started, Receipt::Failed],
            None => {
                self.executions
                    .push((request.key.clone(), request.operation.clone()));
                let done = self.execute(&path, &request.operation);
                vec![Receipt::Accepted, Receipt::Started, done]
            }
        };
        self.ledger.insert(request.key.clone(), receipts.clone());
        receipts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomos_core::plan::{Generation, PlanId};

    fn p(text: &str) -> ResourcePath {
        ResourcePath::new(text).unwrap()
    }

    fn key(resource: &str, iteration: u32) -> EffectKey {
        EffectKey::new(
            PlanId::new("p").unwrap(),
            Generation(1),
            iteration,
            p(resource),
        )
    }

    fn host() -> MockHost {
        let mut host = MockHost::new();
        host.write(&p("/etc/svc.conf"), Digest::from_bytes([1; 32]));
        host.add_service(
            p("/run/svc"),
            Service {
                config: p("/etc/svc.conf"),
                loaded: Some(Digest::from_bytes([1; 32])),
                active: true,
                loaded_path: Some(p("/run/svc.loaded")),
            },
        );
        host
    }

    #[test]
    fn replacing_the_configuration_does_not_change_what_the_service_loaded() {
        let mut host = host();
        let new = Digest::from_bytes([2; 32]);
        let receipts = host.apply(&Apply {
            key: key("/etc/svc.conf", 0),
            operation: Operation::Replace(FileCondition::Present {
                content: Content::Exactly(new),
            }),
            settle_by: Instant(10),
        });
        assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
        assert_eq!(host.file(&p("/etc/svc.conf")), Some(new));
        assert_eq!(
            host.service(&p("/run/svc")).unwrap().loaded,
            Some(Digest::from_bytes([1; 32]))
        );
        host.apply(&Apply {
            key: key("/run/svc", 0),
            operation: Operation::Refresh,
            settle_by: Instant(10),
        });
        assert_eq!(host.service(&p("/run/svc")).unwrap().loaded, Some(new));
        assert_eq!(host.refreshes().len(), 1);
    }

    #[test]
    fn a_key_is_executed_once() {
        let mut host = host();
        let request = Apply {
            key: key("/run/svc", 0),
            operation: Operation::Refresh,
            settle_by: Instant(10),
        };
        let first = host.apply(&request);
        assert_eq!(host.apply(&request), first);
        assert_eq!(host.executions().len(), 1);
        let later = Apply {
            key: key("/run/svc", 1),
            ..request
        };
        host.apply(&later);
        assert_eq!(host.executions().len(), 2);
    }

    #[test]
    fn a_denied_read_is_a_failed_collection_not_an_absent_file() {
        let mut host = host();
        host.deny(&p("/etc/svc.conf"), true);
        let observed = host.observe(&[p("/etc/svc.conf"), p("/run/svc"), p("/run/svc.loaded")]);
        assert_eq!(
            observed[0].collection(),
            &Collection::Failed(CollectionFailure::PermissionDenied)
        );
        assert!(matches!(
            observed[1].collection(),
            Collection::Collected(FileEvidence::Present { .. })
        ));
        assert_eq!(
            observed[2].collection(),
            &Collection::Collected(FileEvidence::Present {
                digest: Digest::from_bytes([1; 32]),
                size: 1
            })
        );
    }

    #[test]
    fn scripted_faults_apply_once() {
        let mut host = host();
        host.fault(&p("/etc/svc.conf"), Fault::Refuse);
        let write = |i| Apply {
            key: key("/etc/svc.conf", i),
            operation: Operation::Replace(FileCondition::Absent),
            settle_by: Instant(10),
        };
        assert_eq!(host.apply(&write(0)), vec![Receipt::Refused]);
        host.fault(&p("/etc/svc.conf"), Fault::Fail);
        assert_eq!(host.apply(&write(1)).last(), Some(&Receipt::Failed));
        assert!(host.file(&p("/etc/svc.conf")).is_some());
        assert_eq!(
            host.apply(&write(2)).last(),
            Some(&Receipt::Completed { changed: true })
        );
        assert!(host.file(&p("/etc/svc.conf")).is_none());
    }
}
