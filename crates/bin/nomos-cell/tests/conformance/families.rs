//! The Substrate conformance suite per family ([substrate-contract.md], The
//! Clauses per Family): the nine clauses, stated for every family of
//! [resource-families.md] an adapter serves.
//!
//! The file suite in the parent module names files, as the first operation
//! did; this one arranges the world as evidence, so one set of checks runs
//! for every family. The requirements and starting points of each family
//! are listed here from the family's truth table, and core judges every
//! execution on a new Observation (S5), so the suite's oracle is the
//! written table, not the adapter.
//!
//! [substrate-contract.md]: ../../../../../docs/formal/substrate-contract.md
//! [resource-families.md]: ../../../../../docs/formal/resource-families.md

use nomos_core::assessment::{Assessment, assess_collection};
use nomos_core::condition::{
    AccountClass, Activity, Content, DirectoryCondition, Enablement, FileCondition, Metadata,
    PackageCondition, PackageVersion, Requirement, SysctlCondition, SysctlValue, UnitCondition,
    UserCondition,
};
use nomos_core::effect::{Apply, EffectKey, Operation, Receipt};
use nomos_core::observation::{
    Account, ActiveState, Collection, CollectionFailure, DirectoryEvidence, Evidence, FileEvidence,
    Instant, ObservedMetadata, PackageEvidence, SysctlEvidence, UnitEvidence, UnitFileState,
    UserEvidence,
};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{
    AccountName, Digest, Family, Mode, PackageName, ResourceKey, ResourcePath, SysctlKey, UnitName,
};
use nomos_substrate::{Mutate, Observe};

use super::{MockSubject, check};

/// An adapter under test and the world around it, arranged as evidence.
pub trait World: Observe + Mutate {
    /// The families the adapter serves.
    fn families(&self) -> Vec<Family>;
    /// Makes `key` what `evidence` says, as something other than Nomos
    /// would; `None` makes it absent, not installed, or unknown to its
    /// manager.
    fn arrange(&mut self, key: &ResourceKey, evidence: Option<Evidence>);
    /// Makes `key` unreadable to the adapter.
    fn deny(&mut self, key: &ResourceKey);
    /// What is true of `key`, read without the port.
    fn truth(&self, key: &ResourceKey) -> Option<Evidence>;
    /// Makes available what `requirement` needs from outside the host: a
    /// package's version in the repository.
    fn provide(&mut self, key: &ResourceKey, requirement: &Requirement);
    /// Executions started so far.
    fn executions(&self) -> usize;
    /// The adapter's clock now (S9).
    fn now(&mut self) -> Instant;
    /// Requests the adapter must refuse for `family`, with the world
    /// arranged for them.
    fn refusals(&mut self, family: Family) -> Vec<Apply>;
}

// ---------------------------------------------------------------------------
// Names and values

fn path(text: &str) -> ResourcePath {
    ResourcePath::new(text).unwrap()
}

fn account(text: &str) -> AccountName {
    AccountName::new(text).unwrap()
}

fn mode(text: &str) -> Mode {
    Mode::from_octal(text).unwrap()
}

fn version(text: &str) -> PackageVersion {
    PackageVersion::new(text).unwrap()
}

fn d(n: u8) -> Digest {
    Digest::from_bytes([n; 32])
}

/// The resource of `family` the suite names `slot`, in a namespace of its
/// own per clause, so that clauses do not see each other's arrangements.
pub fn resource(family: Family, clause: &str, slot: u32) -> ResourceKey {
    // Lowercase letters, digits, and hyphens: valid in every family.
    let name = format!("{clause}-{slot}");
    match family {
        Family::Directory => ResourceKey::Directory(path(&format!("/srv/{name}"))),
        Family::File => ResourceKey::File(path(&format!("/srv/{name}.conf"))),
        Family::Service => ResourceKey::Service(path(&format!("/run/{name}"))),
        Family::Unit => ResourceKey::Unit(UnitName::new(&format!("{name}.service")).unwrap()),
        Family::Sysctl => ResourceKey::Sysctl(SysctlKey::new(&format!("net.{name}")).unwrap()),
        Family::User => ResourceKey::User(account(&format!("u{name}"))),
        Family::Package => ResourceKey::Package(PackageName::new(&format!("pkg{name}")).unwrap()),
    }
}

fn key(resource: &ResourceKey, iteration: u32) -> EffectKey {
    EffectKey::new(
        PlanId::new("families").unwrap(),
        Generation(1),
        iteration,
        resource.clone(),
    )
}

pub fn converge(resource: &ResourceKey, iteration: u32, requirement: Requirement) -> Apply {
    Apply {
        key: key(resource, iteration),
        operation: Operation::Converge(requirement),
        settle_by: Instant(u64::MAX),
    }
}

fn meta(owner: Account, group: Account, bits: &str) -> ObservedMetadata {
    ObservedMetadata {
        owner,
        group,
        mode: mode(bits),
    }
}

fn wanted(owner: Option<&str>, group: Option<&str>, bits: Option<&str>) -> Metadata {
    Metadata {
        owner: owner.map(account),
        group: group.map(account),
        mode: bits.map(mode),
    }
}

/// The starting points of a family: what a resource may be before an
/// execution, `None` for absent or unknown.
pub fn starts(family: Family) -> Vec<Option<Evidence>> {
    match family {
        Family::File => vec![
            None,
            Some(Evidence::File(FileEvidence::present(d(1), 1))),
            Some(Evidence::File(FileEvidence::Present {
                digest: d(2),
                size: 3,
                metadata: meta(Account::Named(account("app")), Account::Id(4), "0600"),
            })),
        ],
        Family::Directory => vec![
            None,
            Some(Evidence::Directory(DirectoryEvidence::Present {
                metadata: meta(Account::Id(0), Account::Id(0), "0755"),
            })),
            Some(Evidence::Directory(DirectoryEvidence::Present {
                metadata: meta(Account::Named(account("app")), Account::Id(5), "0700"),
            })),
        ],
        Family::Unit => {
            let mut all = Vec::new();
            for active in [
                ActiveState::Active,
                ActiveState::Reloading,
                ActiveState::Inactive,
                ActiveState::Failed,
                ActiveState::Activating,
                ActiveState::Deactivating,
            ] {
                for file_state in [
                    UnitFileState::Enabled,
                    UnitFileState::Disabled,
                    UnitFileState::Static,
                ] {
                    all.push(Some(Evidence::Unit(UnitEvidence { active, file_state })));
                }
            }
            all
        }
        Family::Sysctl => ["0", "1 2"]
            .into_iter()
            .map(|v| {
                Some(Evidence::Sysctl(SysctlEvidence {
                    value: SysctlValue::normalized(v),
                }))
            })
            .collect(),
        Family::User => vec![
            None,
            Some(Evidence::User(UserEvidence::Present {
                uid: 150,
                gid: 150,
                home: "/nonexistent".into(),
                shell: "/usr/sbin/nologin".into(),
            })),
            Some(Evidence::User(UserEvidence::Present {
                uid: 1001,
                gid: 1001,
                home: "/home/someone".into(),
                shell: "/bin/bash".into(),
            })),
        ],
        Family::Package => vec![
            None,
            Some(Evidence::Package(PackageEvidence::Installed {
                version: version("1.0-1"),
            })),
            Some(Evidence::Package(PackageEvidence::Broken)),
        ],
        Family::Service => vec![],
    }
}

/// The requirements of a family, from its truth table.
pub fn requirements(family: Family) -> Vec<Requirement> {
    match family {
        Family::File => vec![
            Requirement::File(FileCondition::Absent),
            Requirement::File(FileCondition::present(Content::Any)),
            Requirement::File(FileCondition::present(Content::Exactly(d(1)))),
            Requirement::File(FileCondition::Present {
                content: Content::Exactly(d(3)),
                metadata: wanted(Some("app"), None, Some("0640")),
            }),
            Requirement::File(FileCondition::Present {
                content: Content::Any,
                metadata: wanted(None, Some("adm"), None),
            }),
        ],
        Family::Directory => vec![
            Requirement::Directory(DirectoryCondition::Absent),
            Requirement::Directory(DirectoryCondition::Present {
                metadata: Metadata::any(),
            }),
            Requirement::Directory(DirectoryCondition::Present {
                metadata: wanted(Some("app"), Some("app"), Some("0750")),
            }),
            Requirement::Directory(DirectoryCondition::Present {
                metadata: wanted(None, None, Some("0700")),
            }),
        ],
        Family::Unit => {
            let mut all = Vec::new();
            for activity in [Activity::Active, Activity::Inactive, Activity::Any] {
                for enablement in [Enablement::Enabled, Enablement::Disabled, Enablement::Any] {
                    all.push(Requirement::Unit(UnitCondition {
                        activity,
                        enablement,
                    }));
                }
            }
            all
        }
        Family::Sysctl => ["1 2", "0"]
            .into_iter()
            .map(|v| {
                Requirement::Sysctl(SysctlCondition {
                    value: SysctlValue::normalized(v),
                })
            })
            .collect(),
        Family::User => vec![
            Requirement::User(UserCondition::Absent),
            Requirement::User(UserCondition::Present {
                class: AccountClass::System,
                home: None,
                shell: None,
            }),
            Requirement::User(UserCondition::Present {
                class: AccountClass::System,
                home: Some(path("/var/lib/app")),
                shell: Some(path("/bin/sh")),
            }),
            Requirement::User(UserCondition::Present {
                class: AccountClass::Regular,
                home: None,
                shell: Some(path("/bin/bash")),
            }),
        ],
        Family::Package => vec![
            Requirement::Package(PackageCondition::Absent),
            Requirement::Package(PackageCondition::Installed { version: None }),
            Requirement::Package(PackageCondition::Installed {
                version: Some(version("1.0-1")),
            }),
            Requirement::Package(PackageCondition::Installed {
                version: Some(version("2.0-1")),
            }),
        ],
        Family::Service => vec![],
    }
}

/// Whether the family's contract refuses `requirement` from `start`
/// (S7): a user in the other class, or enabling or disabling a unit whose
/// file state is fixed. These pairs are the refusal checks' and not S5's.
pub fn refused(start: &Option<Evidence>, requirement: &Requirement) -> bool {
    match (start, requirement) {
        (
            Some(Evidence::User(UserEvidence::Present { uid, .. })),
            Requirement::User(UserCondition::Present { class, .. }),
        ) => AccountClass::of(*uid) != *class,
        (Some(Evidence::Unit(u)), Requirement::Unit(c)) => {
            u.file_state == UnitFileState::Static && c.enablement != Enablement::Any
        }
        _ => false,
    }
}

/// The absence evidence of a family, or `None` where the family has none
/// and an unknown resource is a failed collection.
fn absent(family: Family) -> Option<Evidence> {
    match family {
        Family::File => Some(Evidence::File(FileEvidence::Absent)),
        Family::Directory => Some(Evidence::Directory(DirectoryEvidence::Absent)),
        Family::User => Some(Evidence::User(UserEvidence::Absent)),
        Family::Package => Some(Evidence::Package(PackageEvidence::NotInstalled)),
        Family::Unit | Family::Sysctl | Family::Service => None,
    }
}

/// What observing an arranged `start` must collect.
fn expected(family: Family, start: &Option<Evidence>) -> Collection {
    match (start, absent(family)) {
        (Some(e), _) => Collection::Collected(e.clone()),
        (None, Some(e)) => Collection::Collected(e),
        (None, None) => Collection::Failed(CollectionFailure::Unavailable),
    }
}

fn observe_one(w: &mut impl World, key: &ResourceKey) -> Result<Collection, String> {
    let observed = w.observe(std::slice::from_ref(key));
    observed
        .iter()
        .find(|o| o.key() == key)
        .map(|o| o.collection().clone())
        .ok_or_else(|| format!("no Observation of {key}"))
}

// ---------------------------------------------------------------------------
// The clauses

/// S1: one Observation or more of each requested resource, of its family,
/// and none of a resource not requested.
pub fn s1_coverage(w: &mut impl World, family: Family) -> Result<(), String> {
    let starts = starts(family);
    let asked: Vec<ResourceKey> = (0..3).map(|i| resource(family, "s1", i)).collect();
    w.arrange(&asked[0], starts.last().cloned().flatten());
    w.arrange(&asked[1], None);
    let unrequested = resource(family, "s1", 9);
    w.arrange(&unrequested, starts.last().cloned().flatten());
    let observed = w.observe(&asked);
    for key in &asked {
        check(observed.iter().any(|o| o.key() == key), || {
            format!("no Observation of {key}")
        })?;
    }
    check(observed.iter().all(|o| asked.contains(o.key())), || {
        format!("an Observation of a resource not requested: {observed:?}")
    })?;
    check(
        observed.iter().all(|o| match o.collection() {
            Collection::Collected(e) => e.family() == family,
            Collection::Failed(_) => true,
        }),
        || format!("evidence of another family: {observed:?}"),
    )
}

/// S2: absence only when the family's evidence says so; an unknown unit
/// or parameter, and a denied read, are failed collections.
pub fn s2_truthful_absence(w: &mut impl World, family: Family) -> Result<(), String> {
    let none = resource(family, "s2", 0);
    w.arrange(&none, None);
    let got = observe_one(w, &none)?;
    let want = expected(family, &None);
    check(got == want, || {
        format!("{none}, arranged absent, observed as {got:?}, not {want:?}")
    })?;
    let secret = resource(family, "s2", 1);
    w.arrange(&secret, starts(family).last().cloned().flatten());
    w.deny(&secret);
    let got = observe_one(w, &secret)?;
    check(
        got == Collection::Failed(CollectionFailure::PermissionDenied),
        || format!("a denied read of {secret} observed as {got:?}"),
    )
}

/// S3: the evidence the family's table lists, as arranged.
pub fn s3_evidence(w: &mut impl World, family: Family) -> Result<(), String> {
    for (i, start) in starts(family).into_iter().enumerate() {
        let key = resource(family, "s3", i as u32);
        w.arrange(&key, start.clone());
        let got = observe_one(w, &key)?;
        let want = expected(family, &start);
        check(got == want, || {
            format!("{key} observed as {got:?}, not {want:?}")
        })?;
    }
    Ok(())
}

/// S4: observing changes nothing, however often it is repeated.
pub fn s4_observation_does_not_mutate(w: &mut impl World, family: Family) -> Result<(), String> {
    let keys: Vec<ResourceKey> = starts(family)
        .into_iter()
        .enumerate()
        .map(|(i, start)| {
            let key = resource(family, "s4", i as u32);
            w.arrange(&key, start);
            key
        })
        .collect();
    let before: Vec<Option<Evidence>> = keys.iter().map(|k| w.truth(k)).collect();
    let executions = w.executions();
    for _ in 0..3 {
        w.observe(&keys);
    }
    let after: Vec<Option<Evidence>> = keys.iter().map(|k| w.truth(k)).collect();
    check(before == after && w.executions() == executions, || {
        format!("observation changed the world: {before:?} became {after:?}")
    })
}

/// S5 and S8: every requirement from every starting point the contract
/// does not refuse completes, settles, reports `changed` exactly when the
/// evidence changed, and is judged Satisfied by core on a new Observation.
pub fn s5_postconditions_are_cores(w: &mut impl World, family: Family) -> Result<(), String> {
    let mut n = 0u32;
    for requirement in requirements(family) {
        for start in starts(family) {
            if refused(&start, &requirement) {
                continue;
            }
            n += 1;
            let key = resource(family, "s5", n);
            w.arrange(&key, start.clone());
            w.provide(&key, &requirement);
            let before = w.truth(&key);
            let receipts = w.apply(&converge(&key, n, requirement.clone()));
            let after = w.truth(&key);
            check(receipts.last().is_some_and(Receipt::settles), || {
                format!("{key} from {start:?} to {requirement:?} ended unsettled: {receipts:?}")
            })?;
            let Some(Receipt::Completed { changed }) = receipts.last() else {
                return Err(format!(
                    "{key} from {start:?} to {requirement:?}: receipts {receipts:?}"
                ));
            };
            check(*changed == (before != after), || {
                format!("{key}: changed = {changed}, but {before:?} became {after:?}")
            })?;
            let assessed = assess_collection(&requirement, &observe_one(w, &key)?);
            check(assessed == Assessment::Satisfied, || {
                format!("{key} from {start:?} to {requirement:?}: core assessed {assessed:?}")
            })?;
        }
    }
    check(n > 0, || format!("no case for {family:?}"))
}

/// S6: a key seen before returns its receipts again and changes nothing,
/// even after a foreign change undid the effect.
pub fn s6_once_per_key(w: &mut impl World, family: Family) -> Result<(), String> {
    let key = resource(family, "s6", 0);
    let starts = starts(family);
    let first_start = starts.first().cloned().flatten();
    w.arrange(&key, first_start.clone());
    let requirement = requirements(family)
        .into_iter()
        .find(|r| !refused(&first_start, r))
        .ok_or("no requirement")?;
    w.provide(&key, &requirement);
    let request = converge(&key, 0, requirement);
    let first = w.apply(&request);
    let executions = w.executions();
    let foreign = starts.last().cloned().flatten();
    w.arrange(&key, foreign.clone());
    let again = w.apply(&request);
    check(again == first, || {
        format!("a repeated key gave {again:?}, then {first:?}")
    })?;
    check(w.executions() == executions, || {
        "a repeated key executed again".into()
    })?;
    check(w.truth(&key) == foreign, || {
        "a repeated key changed the resource".into()
    })
}

/// S7 and S8: what the family's contract refuses is refused, settles, and
/// changes nothing.
pub fn s7_refusal_before_effect(w: &mut impl World, family: Family) -> Result<(), String> {
    let requests = w.refusals(family);
    check(!requests.is_empty(), || {
        format!("no refusal for {family:?}")
    })?;
    for request in requests {
        let key = request.key.resource().clone();
        let before = w.truth(&key);
        let executions = w.executions();
        let receipts = w.apply(&request);
        check(receipts == vec![Receipt::Refused], || {
            format!("{request:?} gave {receipts:?}")
        })?;
        check(
            w.truth(&key) == before && w.executions() == executions,
            || format!("a refused {request:?} changed something"),
        )?;
    }
    Ok(())
}

/// S9: collection windows are on the adapter's clock.
pub fn s9_one_clock(w: &mut impl World, family: Family) -> Result<(), String> {
    let key = resource(family, "s9", 0);
    w.arrange(&key, starts(family).last().cloned().flatten());
    let before = w.now();
    let observed = w.observe(std::slice::from_ref(&key));
    let after = w.now();
    for o in &observed {
        let win = o.provenance().window();
        check(before <= win.start() && win.end() <= after, || {
            format!("window {win:?} outside [{before:?}, {after:?}]")
        })?;
    }
    Ok(())
}

/// Every clause for `family`, in order.
pub fn all(w: &mut impl World, family: Family) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("S1", s1_coverage(w, family)),
        ("S2", s2_truthful_absence(w, family)),
        ("S3", s3_evidence(w, family)),
        ("S4", s4_observation_does_not_mutate(w, family)),
        ("S5", s5_postconditions_are_cores(w, family)),
        ("S6", s6_once_per_key(w, family)),
        ("S7", s7_refusal_before_effect(w, family)),
        ("S9", s9_one_clock(w, family)),
    ]
}

/// The families of Phase 1: every family but the legacy service, whose
/// clauses are the file suite's and the transition kernel's.
pub const PHASE_1: [Family; 6] = [
    Family::Directory,
    Family::File,
    Family::Package,
    Family::Sysctl,
    Family::Unit,
    Family::User,
];

/// Requests the contract refuses for any family: a requirement of another
/// family than the key's.
pub fn wrong_family(key: &ResourceKey, iteration: u32) -> Apply {
    let other = if key.family() == Family::File {
        Requirement::Sysctl(SysctlCondition {
            value: SysctlValue::normalized("1"),
        })
    } else {
        Requirement::File(FileCondition::Absent)
    };
    converge(key, iteration, other)
}

// ---------------------------------------------------------------------------
// The mock as a world

impl World for MockSubject {
    fn families(&self) -> Vec<Family> {
        PHASE_1.to_vec()
    }
    fn arrange(&mut self, key: &ResourceKey, evidence: Option<Evidence>) {
        self.host.set(key, evidence);
    }
    fn deny(&mut self, key: &ResourceKey) {
        self.host.deny_key(key, true);
    }
    fn truth(&self, key: &ResourceKey) -> Option<Evidence> {
        self.host.get(key).cloned()
    }
    fn provide(&mut self, key: &ResourceKey, _requirement: &Requirement) {
        if let ResourceKey::Package(name) = key {
            self.host.offer(name, &version("1.0-1"));
            self.host.offer(name, &version("2.0-1"));
        }
    }
    fn executions(&self) -> usize {
        self.host.executions().len()
    }
    fn now(&mut self) -> Instant {
        super::Subject::now(self)
    }
    fn refusals(&mut self, family: Family) -> Vec<Apply> {
        let at = |i| resource(family, "s7", i);
        let mut requests = vec![wrong_family(&at(0), 70)];
        match family {
            Family::Directory => {
                // A directory with an entry, asked to be absent.
                let dir = at(1);
                self.arrange(&dir, starts(family)[1].clone());
                let entry = ResourceKey::File(path(&format!("{}/entry", dir.name())));
                self.arrange(&entry, starts(Family::File)[1].clone());
                requests.push(converge(
                    &dir,
                    71,
                    Requirement::Directory(DirectoryCondition::Absent),
                ));
                // A directory asked for where a file is.
                let file = ResourceKey::File(path("/srv/s7-file-here"));
                self.arrange(&file, starts(Family::File)[1].clone());
                requests.push(converge(
                    &ResourceKey::Directory(path("/srv/s7-file-here")),
                    72,
                    requirements(family)[1].clone(),
                ));
            }
            Family::File => {
                // A file asked for where a directory is, and a refresh.
                let dir = ResourceKey::Directory(path("/srv/s7-dir-here"));
                self.arrange(&dir, starts(Family::Directory)[1].clone());
                requests.push(converge(
                    &ResourceKey::File(path("/srv/s7-dir-here")),
                    73,
                    requirements(family)[1].clone(),
                ));
                let file = at(2);
                self.arrange(&file, starts(family)[1].clone());
                requests.push(Apply {
                    operation: Operation::Refresh(requirements(family)[1].clone()),
                    ..converge(&file, 74, requirements(family)[1].clone())
                });
            }
            Family::Unit => {
                // A unit systemd does not know, and a static one asked to
                // be enabled.
                let unknown = at(1);
                self.arrange(&unknown, None);
                requests.push(converge(&unknown, 75, requirements(family)[0].clone()));
                let fixed = at(2);
                self.arrange(
                    &fixed,
                    Some(Evidence::Unit(UnitEvidence {
                        active: ActiveState::Active,
                        file_state: UnitFileState::Static,
                    })),
                );
                requests.push(converge(&fixed, 76, requirements(family)[0].clone()));
            }
            Family::Sysctl => {
                // A parameter the kernel does not have.
                let unknown = at(1);
                self.arrange(&unknown, None);
                requests.push(converge(&unknown, 77, requirements(family)[0].clone()));
            }
            Family::User => {
                // A system account asked to be a regular one.
                let user = at(1);
                self.arrange(&user, starts(family)[1].clone());
                requests.push(converge(&user, 78, requirements(family)[3].clone()));
            }
            Family::Package => {
                // A refresh, which a package does not have.
                let pkg = at(1);
                self.arrange(&pkg, starts(family)[1].clone());
                requests.push(Apply {
                    operation: Operation::Refresh(requirements(family)[1].clone()),
                    ..converge(&pkg, 79, requirements(family)[1].clone())
                });
            }
            Family::Service => {}
        }
        requests
    }
}
