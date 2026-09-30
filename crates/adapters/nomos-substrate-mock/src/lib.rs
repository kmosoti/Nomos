//! # nomos-substrate-mock
//!
//! **Driven adapter** for the `nomos-substrate` port. A deterministic
//! in-memory machine — a first-class backend for testing reconciliation,
//! not throwaway scaffolding. It supplies Observations and effect receipts to
//! the same core semantics the Linux adapter feeds.
//!
//! The machine serves every family of resource-families.md. Its world is a
//! map from resource key to the evidence a collector would report; a key
//! with no entry is absent, not installed, or, for a unit or a kernel
//! parameter, unknown to its manager, which is a failed collection
//! (`Unavailable`). A file and a directory at one path are one place: a key
//! of the other kind there cannot be described (`Unsupported`), and an
//! operation on it is refused.
//!
//! It also keeps the legacy service of `05-transition-kernel`. A service has
//! a configuration file on disk and, separately, the revision it loaded when
//! it last started, so replacing the file does not change what the service
//! runs until it is refreshed. A service is observed through its status
//! path, present while it runs; it can also expose the revision it loaded at
//! a second path, for the design in which that revision is a Condition.
//!
//! Faults are scripted: a denied read, a failed or refused execution, and a
//! writer outside Nomos. Time is set by the caller, never read. Executions
//! are deduplicated by idempotency key, as the port requires.

use std::collections::{BTreeMap, BTreeSet};

use nomos_core::condition::{
    AccountClass, Activity, Content, DirectoryCondition, Enablement, FileCondition, Metadata,
    PackageCondition, PackageVersion, Requirement, SysctlCondition, UnitCondition, UserCondition,
};
use nomos_core::effect::{Apply, EffectKey, Operation, Receipt};
use nomos_core::footprint::Property;
use nomos_core::observation::{
    Account, ActiveState, Collection, CollectionFailure, CollectorId, DirectoryEvidence, Evidence,
    FileEvidence, Instant, Observation, ObservedMetadata, PackageEvidence, Provenance,
    SysctlEvidence, UnitEvidence, UnitFileState, UserEvidence, Window,
};
use nomos_core::resource::{
    Digest, Family, Mode, PackageName, ResourceKey, ResourcePath, UnitName,
};
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
    world: BTreeMap<ResourceKey, Evidence>,
    services: BTreeMap<ResourcePath, Service>,
    repository: BTreeMap<PackageName, Vec<PackageVersion>>,
    denied: BTreeSet<Property>,
    faults: BTreeMap<Property, Fault>,
    ledger: BTreeMap<EffectKey, Vec<Receipt>>,
    executions: Vec<(EffectKey, Operation)>,
    refreshes: Vec<(ResourcePath, Option<Digest>, Option<Digest>)>,
    restarts: BTreeMap<UnitName, usize>,
}

impl Default for MockHost {
    fn default() -> Self {
        MockHost::new()
    }
}

fn file_key(path: &ResourcePath) -> ResourceKey {
    ResourceKey::File(path.clone())
}

/// The first free ID of a class, Debian's ranges: system accounts from 100,
/// regular ones from 1000.
fn first_id(class: AccountClass) -> u32 {
    match class {
        AccountClass::System => 100,
        AccountClass::Regular => 1000,
    }
}

/// Metadata after an operation that states `m`: a stated field is set, an
/// unstated one kept from `before`, or root's default for a new resource.
fn metadata(m: &Metadata, before: Option<&ObservedMetadata>, mode: Mode) -> ObservedMetadata {
    let base = before.cloned().unwrap_or(ObservedMetadata {
        owner: Account::Id(0),
        group: Account::Id(0),
        mode,
    });
    ObservedMetadata {
        owner: m.owner.clone().map_or(base.owner, Account::Named),
        group: m.group.clone().map_or(base.group, Account::Named),
        mode: m.mode.unwrap_or(base.mode),
    }
}

impl MockHost {
    /// An empty machine at instant zero.
    pub fn new() -> Self {
        MockHost {
            now: Instant(0),
            world: BTreeMap::new(),
            services: BTreeMap::new(),
            repository: BTreeMap::new(),
            denied: BTreeSet::new(),
            faults: BTreeMap::new(),
            ledger: BTreeMap::new(),
            executions: Vec::new(),
            refreshes: Vec::new(),
            restarts: BTreeMap::new(),
        }
    }

    /// Sets the machine's clock.
    pub fn set_time(&mut self, now: Instant) {
        self.now = now;
    }

    /// Writes a file as something other than Nomos would. Its size is
    /// reported as 1: the mock holds digests, not bytes.
    pub fn write(&mut self, path: &ResourcePath, digest: Digest) {
        self.write_sized(path, digest, 1);
    }

    /// Writes a file of `size` bytes whose content has `digest`, as
    /// something other than Nomos would. Its metadata is kept if the file
    /// exists, and root's default otherwise.
    pub fn write_sized(&mut self, path: &ResourcePath, digest: Digest, size: u64) {
        let metadata = match self.world.get(&file_key(path)) {
            Some(Evidence::File(FileEvidence::Present { metadata, .. })) => metadata.clone(),
            _ => ObservedMetadata::root_default(),
        };
        self.world.insert(
            file_key(path),
            Evidence::File(FileEvidence::Present {
                digest,
                size,
                metadata,
            }),
        );
    }

    /// Removes a file as something other than Nomos would.
    pub fn remove(&mut self, path: &ResourcePath) {
        self.world.remove(&file_key(path));
    }

    /// The digest of the file at `path`, if one exists.
    pub fn file(&self, path: &ResourcePath) -> Option<Digest> {
        match self.world.get(&file_key(path)) {
            Some(Evidence::File(FileEvidence::Present { digest, .. })) => Some(*digest),
            _ => None,
        }
    }

    /// Sets what is true of `key`, as something other than Nomos would:
    /// `None` removes it, so that it is absent, not installed, or unknown to
    /// its manager. Evidence of another family than the key's is ignored.
    pub fn set(&mut self, key: &ResourceKey, evidence: Option<Evidence>) {
        match evidence {
            Some(e) if e.family() == key.family() => {
                self.world.insert(key.clone(), e);
            }
            Some(_) => {}
            None => {
                self.world.remove(key);
            }
        }
    }

    /// Every resource the world holds, with its evidence.
    pub fn resources(&self) -> impl Iterator<Item = (&ResourceKey, &Evidence)> {
        self.world.iter()
    }

    /// What is true of `key`, read without the port: `None` when nothing
    /// is recorded for it.
    pub fn get(&self, key: &ResourceKey) -> Option<&Evidence> {
        self.world.get(key)
    }

    /// Offers `version` of `package` for installation; the last version
    /// offered is the candidate.
    pub fn offer(&mut self, package: &PackageName, version: &PackageVersion) {
        let versions = self.repository.entry(package.clone()).or_default();
        versions.retain(|v| v != version);
        versions.push(version.clone());
    }

    /// Adds a service observed at `status`.
    pub fn add_service(&mut self, status: ResourcePath, service: Service) {
        self.services.insert(status, service);
    }

    /// The service observed at `status`.
    pub fn service(&self, status: &ResourcePath) -> Option<&Service> {
        self.services.get(status)
    }

    /// Denies or allows reads of every resource at `path`.
    pub fn deny(&mut self, path: &ResourcePath, denied: bool) {
        self.deny_key(&file_key(path), denied);
    }

    /// Denies or allows reads of `key`; for a path, of every resource there.
    pub fn deny_key(&mut self, key: &ResourceKey, denied: bool) {
        let slot = Property::written(key);
        if denied {
            self.denied.insert(slot);
        } else {
            self.denied.remove(&slot);
        }
    }

    /// Scripts a fault for the next execution on `path`.
    pub fn fault(&mut self, path: &ResourcePath, fault: Fault) {
        self.fault_key(&file_key(path), fault);
    }

    /// Scripts a fault for the next execution on `key`.
    pub fn fault_key(&mut self, key: &ResourceKey, fault: Fault) {
        self.faults.insert(Property::written(key), fault);
    }

    /// Every execution performed, in order. A deduplicated request is not
    /// one.
    pub fn executions(&self) -> &[(EffectKey, Operation)] {
        &self.executions
    }

    /// Every refresh of a legacy service performed: the service, what it
    /// had loaded before, and what it loaded.
    pub fn refreshes(&self) -> &[(ResourcePath, Option<Digest>, Option<Digest>)] {
        &self.refreshes
    }

    /// How often the unit `name` was restarted by a refresh.
    pub fn restarts(&self, name: &UnitName) -> usize {
        self.restarts.get(name).copied().unwrap_or(0)
    }

    fn provenance(&self) -> Provenance {
        let collector = CollectorId::new("mock").expect("a collector name is not empty");
        let window = Window::new(self.now, self.now).expect("a window may start and end together");
        Provenance::new(collector, window)
    }

    /// The other kind of resource at `key`'s path, if one is there: a
    /// directory where a file is asked about, or a file where a directory is.
    fn occupied(&self, key: &ResourceKey) -> bool {
        match key {
            ResourceKey::File(p) => self.world.contains_key(&ResourceKey::Directory(p.clone())),
            ResourceKey::Directory(p) => self.world.contains_key(&file_key(p)),
            _ => false,
        }
    }

    /// Whether anything is recorded beneath the directory at `path`.
    fn has_entries(&self, path: &ResourcePath) -> bool {
        let prefix = format!("{}/", path.as_str());
        self.world
            .keys()
            .filter_map(ResourceKey::path)
            .any(|p| p.as_str().starts_with(&prefix))
    }

    fn service_status(&self, path: &ResourcePath) -> Option<ResourcePath> {
        self.services
            .iter()
            .find(|(status, s)| *status == path || s.loaded_path.as_ref() == Some(path))
            .map(|(status, _)| status.clone())
    }

    fn legacy_evidence(&self, path: &ResourcePath) -> FileEvidence {
        if let Some(service) = self.services.get(path) {
            return if service.active {
                FileEvidence::present(EMPTY, 0)
            } else {
                FileEvidence::Absent
            };
        }
        if let Some(service) = self
            .services
            .values()
            .find(|s| s.loaded_path.as_ref() == Some(path))
        {
            return match (service.active, service.loaded) {
                (true, Some(digest)) => FileEvidence::present(digest, 1),
                _ => FileEvidence::Absent,
            };
        }
        match self.world.get(&file_key(path)) {
            Some(Evidence::File(f)) => f.clone(),
            _ => FileEvidence::Absent,
        }
    }

    fn evidence(&self, key: &ResourceKey) -> Collection {
        if self.denied.contains(&Property::written(key)) {
            return Collection::Failed(CollectionFailure::PermissionDenied);
        }
        if self.occupied(key) {
            return Collection::Failed(CollectionFailure::Unsupported);
        }
        if let ResourceKey::Service(path) = key {
            return Collection::Collected(Evidence::Service(self.legacy_evidence(path)));
        }
        if let Some(e) = self.world.get(key) {
            return Collection::Collected(e.clone());
        }
        Collection::Collected(match key.family() {
            Family::File => Evidence::File(FileEvidence::Absent),
            Family::Directory => Evidence::Directory(DirectoryEvidence::Absent),
            Family::User => Evidence::User(UserEvidence::Absent),
            Family::Package => Evidence::Package(PackageEvidence::NotInstalled),
            Family::Unit | Family::Sysctl | Family::Service => {
                return Collection::Failed(CollectionFailure::Unavailable);
            }
        })
    }

    /// Why `operation` on `key` cannot be performed, decided before any
    /// effect (substrate-contract.md, S7).
    fn refusal(&self, key: &ResourceKey, operation: &Operation) -> bool {
        let requirement = operation.requirement();
        if requirement.family() != key.family() || self.occupied(key) {
            return true;
        }
        let known = self.world.get(key);
        match (key, operation) {
            (ResourceKey::Service(path), Operation::Refresh(_)) => {
                self.service_status(path).is_none()
            }
            (ResourceKey::Service(_), Operation::Converge(_)) => true,
            (ResourceKey::Unit(_), op) => match (known, op.requirement()) {
                (Some(Evidence::Unit(u)), Requirement::Unit(c)) => {
                    let fixed = matches!(
                        u.file_state,
                        UnitFileState::Static | UnitFileState::Masked | UnitFileState::Other
                    );
                    let asked = match c.enablement {
                        Enablement::Enabled => UnitFileState::Enabled,
                        Enablement::Disabled => UnitFileState::Disabled,
                        Enablement::Any => u.file_state,
                    };
                    fixed && asked != u.file_state
                }
                _ => true,
            },
            (_, Operation::Refresh(_)) => true,
            (ResourceKey::Sysctl(_), _) => known.is_none(),
            (ResourceKey::Directory(path), _) => {
                matches!(
                    requirement,
                    Requirement::Directory(DirectoryCondition::Absent)
                ) && self.has_entries(path)
            }
            (ResourceKey::User(_), _) => {
                // An account whose ID another account holds: an alias.
                let alias = match known {
                    Some(Evidence::User(UserEvidence::Present { uid, .. })) => {
                        self.world.iter().any(|(k, e)| {
                            k != key
                                && matches!(e, Evidence::User(UserEvidence::Present { uid: other, .. }) if other == uid)
                        })
                    }
                    _ => false,
                };
                alias
                    || match (known, requirement) {
                        (
                            Some(Evidence::User(UserEvidence::Present { uid, .. })),
                            Requirement::User(UserCondition::Present { class, .. }),
                        ) => AccountClass::of(*uid) != *class,
                        _ => false,
                    }
            }
            _ => false,
        }
    }

    /// Performs `operation` on `key`, which `refusal` allowed; `None` is an
    /// execution that ran and failed.
    fn execute(&mut self, key: &ResourceKey, operation: &Operation) -> Option<bool> {
        let before = self.world.get(key).cloned();
        let after: Option<Evidence> = match (key, operation) {
            (ResourceKey::Service(path), Operation::Refresh(_)) => {
                let status = self.service_status(path)?;
                let config = self.services[&status].config.clone();
                let disk = self.file(&config);
                if let Some(service) = self.services.get_mut(&status) {
                    let before = service.loaded;
                    service.loaded = disk;
                    service.active = true;
                    self.refreshes.push((status, before, disk));
                }
                return Some(true);
            }
            (ResourceKey::Unit(name), Operation::Refresh(Requirement::Unit(c))) => {
                let mut u = match &before {
                    Some(Evidence::Unit(u)) => *u,
                    _ => return None,
                };
                u.file_state = enablement(c, u.file_state);
                u.active = match c.activity {
                    Activity::Inactive => ActiveState::Inactive,
                    Activity::Active | Activity::Any => ActiveState::Active,
                };
                *self.restarts.entry(name.clone()).or_default() += 1;
                self.world.insert(key.clone(), Evidence::Unit(u));
                return Some(true);
            }
            (_, Operation::Converge(requirement)) => {
                self.converged(key, requirement, before.as_ref())?
            }
            _ => return None,
        };
        let changed = before != after;
        match after {
            Some(e) => self.world.insert(key.clone(), e),
            None => self.world.remove(key),
        };
        Some(changed)
    }

    /// What `key` is after it is made to satisfy `requirement`, from
    /// `before`; `None` inside when it is absent afterward, and `None`
    /// outside for an execution that fails.
    fn converged(
        &self,
        key: &ResourceKey,
        requirement: &Requirement,
        before: Option<&Evidence>,
    ) -> Option<Option<Evidence>> {
        Some(match requirement {
            Requirement::File(FileCondition::Absent)
            | Requirement::Directory(DirectoryCondition::Absent)
            | Requirement::User(UserCondition::Absent)
            | Requirement::Package(PackageCondition::Absent) => None,
            Requirement::File(FileCondition::Present {
                content,
                metadata: m,
            }) => {
                let old = match before {
                    Some(Evidence::File(FileEvidence::Present {
                        digest,
                        size,
                        metadata,
                    })) => Some((*digest, *size, metadata)),
                    _ => None,
                };
                let (digest, size) = match (content, old) {
                    (Content::Exactly(d), Some((digest, size, _))) if *d == digest => {
                        (digest, size)
                    }
                    (Content::Exactly(d), _) => (*d, 1),
                    (Content::Any, Some((digest, size, _))) => (digest, size),
                    (Content::Any, None) => (EMPTY, 0),
                };
                let metadata = metadata(m, old.map(|o| o.2), Mode::DEFAULT_FILE);
                Some(Evidence::File(FileEvidence::Present {
                    digest,
                    size,
                    metadata,
                }))
            }
            Requirement::Directory(DirectoryCondition::Present { metadata: m }) => {
                let old = match before {
                    Some(Evidence::Directory(DirectoryEvidence::Present { metadata })) => {
                        Some(metadata)
                    }
                    _ => None,
                };
                Some(Evidence::Directory(DirectoryEvidence::Present {
                    metadata: metadata(m, old, Mode::DEFAULT_DIRECTORY),
                }))
            }
            Requirement::Unit(c) => {
                let Some(Evidence::Unit(u)) = before else {
                    return None;
                };
                Some(Evidence::Unit(unit(c, *u)))
            }
            Requirement::Sysctl(SysctlCondition { value }) => {
                Some(Evidence::Sysctl(SysctlEvidence {
                    value: value.clone(),
                }))
            }
            Requirement::User(UserCondition::Present { class, home, shell }) => {
                let ResourceKey::User(name) = key else {
                    return None;
                };
                let (uid, gid, old_home, old_shell) = match before {
                    Some(Evidence::User(UserEvidence::Present {
                        uid,
                        gid,
                        home,
                        shell,
                    })) => (*uid, *gid, home.clone(), shell.clone()),
                    _ => {
                        let uid = self.free_id(*class);
                        let (home, shell) = match class {
                            AccountClass::System => {
                                ("/nonexistent".into(), "/usr/sbin/nologin".into())
                            }
                            AccountClass::Regular => {
                                (format!("/home/{}", name.as_str()), "/bin/sh".into())
                            }
                        };
                        (uid, uid, home, shell)
                    }
                };
                Some(Evidence::User(UserEvidence::Present {
                    uid,
                    gid,
                    home: home.as_ref().map_or(old_home, |h| h.as_str().into()),
                    shell: shell.as_ref().map_or(old_shell, |s| s.as_str().into()),
                }))
            }
            Requirement::Package(PackageCondition::Installed { version }) => {
                let ResourceKey::Package(name) = key else {
                    return None;
                };
                let offered = self.repository.get(name)?;
                let installed = match (version, before) {
                    (None, Some(Evidence::Package(PackageEvidence::Installed { version }))) => {
                        version.clone()
                    }
                    (None, _) => offered.last()?.clone(),
                    (Some(v), _) if offered.contains(v) => v.clone(),
                    (Some(_), _) => return None,
                };
                Some(Evidence::Package(PackageEvidence::Installed {
                    version: installed,
                }))
            }
            Requirement::Service(_) => return None,
        })
    }

    fn free_id(&self, class: AccountClass) -> u32 {
        let taken: BTreeSet<u32> = self
            .world
            .values()
            .filter_map(|e| match e {
                Evidence::User(UserEvidence::Present { uid, .. }) => Some(*uid),
                _ => None,
            })
            .collect();
        (first_id(class)..)
            .find(|id| !taken.contains(id))
            .unwrap_or(u32::MAX)
    }
}

fn enablement(c: &UnitCondition, state: UnitFileState) -> UnitFileState {
    match c.enablement {
        Enablement::Enabled => UnitFileState::Enabled,
        Enablement::Disabled => UnitFileState::Disabled,
        Enablement::Any => state,
    }
}

/// A unit made to satisfy `c`: started or stopped only when the requirement
/// says so and the unit is not already in a state that satisfies it.
fn unit(c: &UnitCondition, u: UnitEvidence) -> UnitEvidence {
    let active = match c.activity {
        Activity::Active if !matches!(u.active, ActiveState::Active | ActiveState::Reloading) => {
            ActiveState::Active
        }
        Activity::Inactive if !matches!(u.active, ActiveState::Inactive | ActiveState::Failed) => {
            ActiveState::Inactive
        }
        _ => u.active,
    };
    UnitEvidence {
        active,
        file_state: enablement(c, u.file_state),
    }
}

impl Observe for MockHost {
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        resources
            .iter()
            .filter_map(|key| Observation::new(key.clone(), self.evidence(key), self.provenance()))
            .collect()
    }
}

impl Mutate for MockHost {
    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        if let Some(receipts) = self.ledger.get(&request.key) {
            return receipts.clone();
        }
        let key = request.key.resource().clone();
        if self.refusal(&key, &request.operation) {
            self.ledger
                .insert(request.key.clone(), vec![Receipt::Refused]);
            return vec![Receipt::Refused];
        }
        let receipts = match self.faults.remove(&Property::written(&key)) {
            Some(Fault::Refuse) => vec![Receipt::Refused],
            Some(Fault::Fail) => vec![Receipt::Accepted, Receipt::Started, Receipt::Failed],
            None => {
                self.executions
                    .push((request.key.clone(), request.operation.clone()));
                let done = match self.execute(&key, &request.operation) {
                    Some(changed) => Receipt::Completed { changed },
                    None => Receipt::Failed,
                };
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

    /// The resource at `text`: the legacy service under `/run/`, a file
    /// anywhere else.
    fn k(text: &str) -> ResourceKey {
        if text.starts_with("/run/") {
            ResourceKey::Service(p(text))
        } else {
            ResourceKey::File(p(text))
        }
    }

    fn refresh() -> Operation {
        Operation::Refresh(Requirement::Service(FileCondition::present(Content::Any)))
    }

    fn converge(requirement: FileCondition) -> Operation {
        Operation::Converge(Requirement::File(requirement))
    }

    fn key(resource: &str, iteration: u32) -> EffectKey {
        EffectKey::new(
            PlanId::new("p").unwrap(),
            Generation(1),
            iteration,
            k(resource),
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
            operation: converge(FileCondition::present(Content::Exactly(new))),
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
            operation: refresh(),
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
            operation: refresh(),
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
        let observed = host.observe(&[k("/etc/svc.conf"), k("/run/svc"), k("/run/svc.loaded")]);
        assert_eq!(
            observed[0].collection(),
            &Collection::Failed(CollectionFailure::PermissionDenied)
        );
        assert!(matches!(
            observed[1].collection(),
            Collection::Collected(Evidence::Service(FileEvidence::Present { .. }))
        ));
        assert_eq!(
            observed[2].collection(),
            &Collection::Collected(Evidence::Service(FileEvidence::present(
                Digest::from_bytes([1; 32]),
                1
            )))
        );
    }

    #[test]
    fn scripted_faults_apply_once() {
        let mut host = host();
        host.fault(&p("/etc/svc.conf"), Fault::Refuse);
        let write = |i| Apply {
            key: key("/etc/svc.conf", i),
            operation: converge(FileCondition::Absent),
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
