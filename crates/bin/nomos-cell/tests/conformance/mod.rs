//! The Substrate conformance suite ([substrate-contract.md]): one check per
//! clause, generic over the adapter under test, and two wrappers that break
//! a clause on purpose.
//!
//! Each check returns `Err` with what it saw rather than panicking, so that
//! a negative control can assert that a check fails. A test of a conforming
//! adapter unwraps it.
//!
//! [substrate-contract.md]: ../../../../../docs/formal/substrate-contract.md

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nomos_core::assessment::{Assessment, assess};
use nomos_core::condition::{Condition, Content, FileCondition, Requirement};
use nomos_core::effect::{Apply, EffectKey, Operation, Receipt};
use nomos_core::observation::{
    Collection, CollectionFailure, Evidence, FileEvidence, Instant, Observation,
};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{Digest, ResourceKey, ResourcePath};
use nomos_substrate::{Mutate, Observe};
use nomos_substrate_linux::LinuxHost;
use nomos_substrate_mock::MockHost;

pub mod families;

pub fn p(text: &str) -> ResourcePath {
    ResourcePath::new(text).unwrap()
}

/// The file at `text`: the clauses below name files.
pub fn f(text: &str) -> ResourceKey {
    ResourceKey::File(p(text))
}

fn file_keys(paths: &[ResourcePath]) -> Vec<ResourceKey> {
    paths.iter().cloned().map(ResourceKey::File).collect()
}

/// A file evidence, collected.
fn collected(evidence: FileEvidence) -> Collection {
    Collection::Collected(Evidence::File(evidence))
}

/// The digest the suite expects for `bytes`: the Secure Hash Algorithm
/// (SHA) 256 of them, computed here, not by the adapter.
pub fn sha(bytes: &[u8]) -> Digest {
    Digest::from_bytes(nomos_canon::sha256::digest(bytes))
}

/// What is at a path, read outside the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truth {
    Absent,
    File(Digest),
    Other,
}

/// An adapter under test and the world around it.
pub trait Subject: Observe + Mutate {
    /// Writes `bytes` at `path`, as a writer other than Nomos would.
    fn put(&mut self, path: &ResourcePath, bytes: &[u8]);
    /// Removes the file at `path`, as a writer other than Nomos would.
    fn remove(&mut self, path: &ResourcePath);
    /// Makes the file at `path` unreadable to the adapter.
    fn deny(&mut self, path: &ResourcePath);
    /// What is at `path`, read without the port.
    fn truth(&self, path: &ResourcePath) -> Truth;
    /// Makes `bytes` available to an exact requirement; returns the digest
    /// that names them.
    fn content(&mut self, bytes: &[u8]) -> Digest;
    /// The digest the adapter gives a file holding `bytes`.
    fn digest(&self, bytes: &[u8]) -> Digest;
    /// Executions started so far.
    fn executions(&self) -> usize;
    /// The adapter's clock now, the one its windows are on (S9).
    fn now(&mut self) -> Instant;
    /// Requests the adapter cannot perform, arranged in the world as needed.
    fn unsupported(&mut self) -> Vec<Apply>;
}

pub fn key(resource: &str, iteration: u32) -> EffectKey {
    EffectKey::new(
        PlanId::new("conformance").unwrap(),
        Generation(1),
        iteration,
        f(resource),
    )
}

pub fn replace(resource: &str, iteration: u32, requirement: FileCondition) -> Apply {
    Apply {
        key: key(resource, iteration),
        operation: Operation::Converge(Requirement::File(requirement)),
        settle_by: Instant(u64::MAX),
    }
}

fn exact(digest: Digest) -> FileCondition {
    FileCondition::present(Content::Exactly(digest))
}

fn collection_of<'a>(
    observed: &'a [Observation],
    path: &ResourcePath,
) -> Result<&'a Collection, String> {
    observed
        .iter()
        .find(|o| {
            o.key().path() == Some(path) && o.key().family() == nomos_core::resource::Family::File
        })
        .map(Observation::collection)
        .ok_or_else(|| format!("no Observation of {}", path.as_str()))
}

fn check(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

// ---------------------------------------------------------------------------
// The clauses

/// S1: at least one Observation of each requested resource, and none of a
/// resource not requested.
pub fn s1_coverage(s: &mut impl Subject) -> Result<(), String> {
    s.put(&p("/etc/s1/present"), b"one");
    s.put(&p("/etc/s1/unrequested"), b"two");
    let asked = [p("/etc/s1/present"), p("/etc/s1/missing"), p("/s1-top")];
    let observed = s.observe(&file_keys(&asked));
    for path in &asked {
        collection_of(&observed, path)?;
    }
    check(
        observed.iter().all(|o| file_keys(&asked).contains(o.key())),
        || format!("an Observation of a resource not requested: {observed:?}"),
    )
}

/// S2: absent only when no file exists; a denied read is a failed
/// collection.
pub fn s2_truthful_absence(s: &mut impl Subject) -> Result<(), String> {
    s.put(&p("/etc/s2/secret"), b"hidden");
    s.deny(&p("/etc/s2/secret"));
    let observed = s.observe(&[f("/etc/s2/secret"), f("/etc/s2/none")]);
    let denied = collection_of(&observed, &p("/etc/s2/secret"))?;
    check(
        *denied == Collection::Failed(CollectionFailure::PermissionDenied),
        || format!("a denied read observed as {denied:?}"),
    )?;
    let none = collection_of(&observed, &p("/etc/s2/none"))?;
    check(*none == collected(FileEvidence::Absent), || {
        format!("a missing file observed as {none:?}")
    })
}

/// S3: a present file with the digest of its content and its size, for
/// an empty file, a small one, and one larger than any single read.
pub fn s3_evidence(s: &mut impl Subject) -> Result<(), String> {
    let large: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let files: [(&str, &[u8]); 3] = [
        ("/etc/s3/empty", b""),
        ("/etc/s3/small", b"evidence, not a verdict"),
        ("/etc/s3/large", &large),
    ];
    for (path, bytes) in files {
        s.put(&p(path), bytes);
        let observed = s.observe(&[f(path)]);
        let c = collection_of(&observed, &p(path))?;
        let want = s.digest(bytes);
        let len = bytes.len() as u64;
        check(
            matches!(
                c,
                Collection::Collected(Evidence::File(FileEvidence::Present { digest, size, .. }))
                    if *digest == want && *size == len
            ),
            || format!("{path}, {len} bytes, observed as {c:?}"),
        )?;
    }
    Ok(())
}

/// S4: observing changes nothing, however often it is repeated.
pub fn s4_observation_does_not_mutate(s: &mut impl Subject) -> Result<(), String> {
    s.put(&p("/etc/s4/a"), b"a");
    let paths = [p("/etc/s4/a"), p("/etc/s4/b"), p("/etc/s4")];
    let before: Vec<Truth> = paths.iter().map(|x| s.truth(x)).collect();
    let executions = s.executions();
    for _ in 0..3 {
        s.observe(&file_keys(&paths));
    }
    let after: Vec<Truth> = paths.iter().map(|x| s.truth(x)).collect();
    check(before == after && s.executions() == executions, || {
        format!("observation changed the world: {before:?} became {after:?}")
    })
}

/// S5: each requirement, from each starting point, is judged by core on a
/// new Observation, and `changed` says whether the file changed.
pub fn s5_postconditions_are_cores(s: &mut impl Subject) -> Result<(), String> {
    let wanted = s.content(b"wanted");
    let cases: [(&str, Option<&[u8]>, FileCondition); 6] = [
        (
            "/etc/s5/absent-from-present",
            Some(b"x"),
            FileCondition::Absent,
        ),
        ("/etc/s5/absent-from-absent", None, FileCondition::Absent),
        (
            "/etc/s5/any-from-absent",
            None,
            FileCondition::present(Content::Any),
        ),
        (
            "/etc/s5/any-from-present",
            Some(b"x"),
            FileCondition::present(Content::Any),
        ),
        ("/etc/s5/exact-from-other", Some(b"x"), exact(wanted)),
        ("/etc/s5/exact-from-same", Some(b"wanted"), exact(wanted)),
    ];
    s.put(&p("/etc/s5/.keep"), b"");
    for (i, (path, start, requirement)) in cases.iter().enumerate() {
        let path = p(path);
        match start {
            Some(bytes) => s.put(&path, bytes),
            None => s.remove(&path),
        }
        let before = s.truth(&path);
        let receipts = s.apply(&replace(path.as_str(), i as u32, requirement.clone()));
        let after = s.truth(&path);
        let Some(Receipt::Completed { changed }) = receipts.last() else {
            return Err(format!("{}: receipts {receipts:?}", path.as_str()));
        };
        check(*changed == (before != after), || {
            format!(
                "{}: changed = {changed}, but {before:?} became {after:?}",
                path.as_str()
            )
        })?;
        let condition = Condition::file(path.clone(), requirement.clone());
        let assessed = assess(&condition, &s.observe(&[ResourceKey::File(path.clone())]));
        check(assessed == Assessment::Satisfied, || {
            format!(
                "{}: core assessed {assessed:?} after the execution",
                path.as_str()
            )
        })?;
    }
    Ok(())
}

/// S6: a key seen before returns its receipts again and changes nothing,
/// even after a foreign writer undid the effect.
pub fn s6_once_per_key(s: &mut impl Subject) -> Result<(), String> {
    let wanted = s.content(b"once");
    s.put(&p("/etc/s6/.keep"), b"");
    let request = replace("/etc/s6/f", 0, exact(wanted));
    let first = s.apply(&request);
    let executions = s.executions();
    s.put(&p("/etc/s6/f"), b"foreign");
    let again = s.apply(&request);
    check(again == first, || {
        format!("a repeated key gave {again:?}, then {first:?}")
    })?;
    check(s.executions() == executions, || {
        "a repeated key executed again".into()
    })?;
    check(
        s.truth(&p("/etc/s6/f")) == Truth::File(s.digest(b"foreign")),
        || "a repeated key changed the file".into(),
    )
}

/// S7: what the adapter cannot perform is refused, and nothing changes.
pub fn s7_refusal_before_effect(s: &mut impl Subject) -> Result<(), String> {
    for request in s.unsupported() {
        let path = request
            .key
            .resource()
            .path()
            .cloned()
            .expect("the file suite's refusals name paths");
        let before = s.truth(&path);
        let executions = s.executions();
        let receipts = s.apply(&request);
        check(receipts == vec![Receipt::Refused], || {
            format!("{request:?} gave {receipts:?}")
        })?;
        check(
            s.truth(&path) == before && s.executions() == executions,
            || format!("a refused {request:?} changed something"),
        )?;
    }
    Ok(())
}

/// S8: every receipt sequence of a completed execution ends settled.
pub fn s8_settlement(s: &mut impl Subject) -> Result<(), String> {
    let wanted = s.content(b"settled");
    s.put(&p("/etc/s8/.keep"), b"");
    let mut requests = vec![
        replace("/etc/s8/f", 0, exact(wanted)),
        replace("/etc/s8/f", 1, FileCondition::Absent),
    ];
    requests.extend(s.unsupported());
    for request in requests {
        let receipts = s.apply(&request);
        check(receipts.last().is_some_and(Receipt::settles), || {
            format!("{request:?} ended unsettled: {receipts:?}")
        })?;
    }
    Ok(())
}

/// S9: collection windows are on the adapter's clock.
pub fn s9_one_clock(s: &mut impl Subject) -> Result<(), String> {
    s.put(&p("/etc/s9"), b"t");
    let before = s.now();
    let observed = s.observe(&[f("/etc/s9")]);
    let after = s.now();
    for o in &observed {
        let w = o.provenance().window();
        check(before <= w.start() && w.end() <= after, || {
            format!("window {w:?} outside [{before:?}, {after:?}]")
        })?;
    }
    Ok(())
}

/// Every clause, in order.
pub fn all(s: &mut impl Subject) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("S1", s1_coverage(s)),
        ("S2", s2_truthful_absence(s)),
        ("S3", s3_evidence(s)),
        ("S4", s4_observation_does_not_mutate(s)),
        ("S5", s5_postconditions_are_cores(s)),
        ("S6", s6_once_per_key(s)),
        ("S7", s7_refusal_before_effect(s)),
        ("S8", s8_settlement(s)),
        ("S9", s9_one_clock(s)),
    ]
}

// ---------------------------------------------------------------------------
// The subjects

/// The mock host.
pub struct MockSubject {
    pub host: MockHost,
    now: u64,
}

impl MockSubject {
    pub fn new() -> Self {
        MockSubject {
            host: MockHost::new(),
            now: 0,
        }
    }
}

impl Observe for MockSubject {
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        self.host.observe(resources)
    }
}

impl Mutate for MockSubject {
    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        self.host.apply(request)
    }
}

impl Subject for MockSubject {
    fn put(&mut self, path: &ResourcePath, bytes: &[u8]) {
        self.host.write_sized(path, sha(bytes), bytes.len() as u64);
    }
    fn remove(&mut self, path: &ResourcePath) {
        self.host.remove(path);
    }
    fn deny(&mut self, path: &ResourcePath) {
        self.host.deny(path, true);
    }
    fn truth(&self, path: &ResourcePath) -> Truth {
        self.host.file(path).map_or(Truth::Absent, Truth::File)
    }
    fn content(&mut self, bytes: &[u8]) -> Digest {
        sha(bytes)
    }
    fn digest(&self, bytes: &[u8]) -> Digest {
        sha(bytes)
    }
    fn executions(&self) -> usize {
        self.host.executions().len()
    }
    fn now(&mut self) -> Instant {
        // The mock's clock is set, never read; each reading advances it by
        // one, so that a window can be seen to lie between two readings.
        self.now += 1;
        self.host.set_time(Instant(self.now));
        Instant(self.now)
    }
    fn unsupported(&mut self) -> Vec<Apply> {
        self.put(&p("/etc/mock-plain"), b"not a service");
        vec![Apply {
            key: key("/etc/mock-plain", 90),
            operation: Operation::Refresh(Requirement::File(FileCondition::present(Content::Any))),
            settle_by: Instant(u64::MAX),
        }]
    }
}

/// The Linux host, rooted at a scratch directory. Constructing one drops
/// this thread's capability to override file permissions, so that a denied
/// read is denied even to a test running as root (substrate-contract.md,
/// Failure Injection on Linux).
pub struct LinuxSubject {
    pub host: LinuxHost,
    pub root: PathBuf,
    unknown: Digest,
}

impl LinuxSubject {
    pub fn new(name: &str) -> Self {
        drop_permission_override();
        let root =
            std::env::temp_dir().join(format!("nomos-conformance-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        LinuxSubject {
            host: LinuxHost::open(&root).unwrap(),
            root,
            unknown: sha(b"content nobody supplied"),
        }
    }

    pub fn host_path(&self, path: &ResourcePath) -> PathBuf {
        self.root.join(path.as_str().trim_start_matches('/'))
    }
}

impl Drop for LinuxSubject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Removes the permission-override capabilities from this thread's
/// effective set. Capabilities are per thread on Linux, and each test runs
/// on its own thread.
pub fn drop_permission_override() {
    use rustix::thread::{CapabilitySet, capabilities, set_capabilities};
    let mut sets = capabilities(None).unwrap();
    sets.effective
        .remove(CapabilitySet::DAC_OVERRIDE | CapabilitySet::DAC_READ_SEARCH);
    set_capabilities(None, sets).unwrap();
}

fn make_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
}

impl Observe for LinuxSubject {
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        self.host.observe(resources)
    }
}

impl Mutate for LinuxSubject {
    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        self.host.apply(request)
    }
}

impl Subject for LinuxSubject {
    fn put(&mut self, path: &ResourcePath, bytes: &[u8]) {
        let at = self.host_path(path);
        make_parent(&at);
        std::fs::write(&at, bytes).unwrap();
    }
    fn remove(&mut self, path: &ResourcePath) {
        let at = self.host_path(path);
        make_parent(&at);
        let _ = std::fs::remove_file(at);
    }
    fn deny(&mut self, path: &ResourcePath) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(self.host_path(path), std::fs::Permissions::from_mode(0o000))
            .unwrap();
    }
    fn truth(&self, path: &ResourcePath) -> Truth {
        let at = self.host_path(path);
        match std::fs::symlink_metadata(&at) {
            Err(_) => Truth::Absent,
            Ok(m) if m.is_file() => match std::fs::read(&at) {
                Ok(bytes) => Truth::File(sha(&bytes)),
                Err(_) => Truth::Other,
            },
            Ok(_) => Truth::Other,
        }
    }
    fn content(&mut self, bytes: &[u8]) -> Digest {
        self.host.add_content(bytes.to_vec())
    }
    fn digest(&self, bytes: &[u8]) -> Digest {
        nomos_substrate_linux::digest_of(bytes)
    }
    fn executions(&self) -> usize {
        self.host.executions()
    }
    fn now(&mut self) -> Instant {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        Instant(t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64)
    }
    fn unsupported(&mut self) -> Vec<Apply> {
        self.put(&p("/etc/linux/plain"), b"x");
        std::fs::create_dir_all(self.host_path(&p("/etc/linux/a-directory"))).unwrap();
        vec![
            // A refresh, which waits for the systemd resource.
            Apply {
                key: key("/etc/linux/plain", 90),
                operation: Operation::Refresh(Requirement::File(FileCondition::present(
                    Content::Any,
                ))),
                settle_by: Instant(u64::MAX),
            },
            // Content that is not in the store.
            replace("/etc/linux/plain", 91, exact(self.unknown)),
            // A missing parent directory.
            replace(
                "/etc/linux/no-such-dir/f",
                92,
                FileCondition::present(Content::Any),
            ),
            // A directory where a file is required.
            replace("/etc/linux/a-directory", 93, FileCondition::Absent),
        ]
    }
}

// ---------------------------------------------------------------------------
// The negative controls

/// Observation that writes: before observing, it removes what it is asked
/// to observe. S4 must fail.
pub struct MutatingTrace<S>(pub S, pub u32);

/// A denied read reported as an absent file. S2 must fail.
pub struct DeniedAsAbsent<S>(pub S);

impl<S: Subject> Observe for MutatingTrace<S> {
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        for r in resources {
            self.1 += 1;
            self.0
                .apply(&replace(r.name(), 1000 + self.1, FileCondition::Absent));
        }
        self.0.observe(resources)
    }
}

impl<S: Subject> Observe for DeniedAsAbsent<S> {
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        self.0
            .observe(resources)
            .into_iter()
            .map(|o| match o.collection() {
                Collection::Failed(CollectionFailure::PermissionDenied) => Observation::new(
                    o.key().clone(),
                    collected(FileEvidence::Absent),
                    o.provenance().clone(),
                )
                .unwrap_or(o),
                _ => o,
            })
            .collect()
    }
}

macro_rules! delegate {
    ($wrapper:ident) => {
        impl<S: Subject> Mutate for $wrapper<S> {
            fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
                self.0.apply(request)
            }
        }
        impl<S: Subject> Subject for $wrapper<S> {
            fn put(&mut self, path: &ResourcePath, bytes: &[u8]) {
                self.0.put(path, bytes)
            }
            fn remove(&mut self, path: &ResourcePath) {
                self.0.remove(path)
            }
            fn deny(&mut self, path: &ResourcePath) {
                self.0.deny(path)
            }
            fn truth(&self, path: &ResourcePath) -> Truth {
                self.0.truth(path)
            }
            fn content(&mut self, bytes: &[u8]) -> Digest {
                self.0.content(bytes)
            }
            fn digest(&self, bytes: &[u8]) -> Digest {
                self.0.digest(bytes)
            }
            fn executions(&self) -> usize {
                self.0.executions()
            }
            fn now(&mut self) -> Instant {
                self.0.now()
            }
            fn unsupported(&mut self) -> Vec<Apply> {
                self.0.unsupported()
            }
        }
    };
}

delegate!(MutatingTrace);
delegate!(DeniedAsAbsent);

/// The clause results, by clause, for a report.
pub fn report(results: &[(&'static str, Result<(), String>)]) -> BTreeMap<&'static str, bool> {
    results.iter().map(|(k, r)| (*k, r.is_ok())).collect()
}
