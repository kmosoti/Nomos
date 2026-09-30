//! Kernel parameters, users, and packages on a real Debian host (Phase 1
//! plan, `12-sysctl-and-user` and `13-package`): the running kernel's own
//! `/proc/sys`, the host's user database changed without `--prefix`, and
//! the host's packages through `apt-get`, from a local repository of
//! dummy packages the tests build, as production does.
//!
//! Every test here changes the host, so every one is ignored on an
//! ordinary run and runs in a disposable container: `cargo xtask debian`
//! runs this binary with `--include-ignored`. The ground truth is read
//! from `/proc/sys`, with `getent`, and with `dpkg-query`, never through
//! the adapter.

mod conformance;
mod debian;
mod scratch;
mod support;

use std::path::Path;
use std::process::Command;

use nomos_core::condition::{
    AccountClass, Requirement, SysctlCondition, SysctlValue, UserCondition,
};
use nomos_core::effect::{Apply, EffectKey, Operation, Receipt};
use nomos_core::observation::{Collection, CollectionFailure, Evidence, Instant};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{AccountName, ResourceKey, ResourcePath, SysctlKey};
use nomos_substrate::{Mutate, Observe};
use nomos_substrate_linux::LinuxHost;

/// One test at a time: they share the host's kernel and databases.
static HOST: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    HOST.lock().unwrap_or_else(|e| e.into_inner())
}

fn linux() -> LinuxHost {
    LinuxHost::open(Path::new("/")).unwrap()
}

fn converge(key: &ResourceKey, iteration: u32, requirement: Requirement) -> Apply {
    Apply {
        key: EffectKey::new(
            PlanId::new("debian-host").unwrap(),
            Generation(1),
            iteration,
            key.clone(),
        ),
        operation: Operation::Converge(requirement),
        settle_by: Instant(u64::MAX),
    }
}

fn observe_one(host: &mut LinuxHost, key: &ResourceKey) -> Collection {
    let observed = host.observe(std::slice::from_ref(key));
    assert_eq!(observed.len(), 1);
    observed[0].collection().clone()
}

fn sysctl(name: &str) -> ResourceKey {
    ResourceKey::Sysctl(SysctlKey::new(name).unwrap())
}

fn value(text: &str) -> Requirement {
    Requirement::Sysctl(SysctlCondition {
        value: SysctlValue::normalized(text),
    })
}

/// The running kernel's `kernel.domainname`, which is private to the
/// container's namespace for host and domain names, is read, written, and
/// read back; a value the kernel does not keep fails; a parameter it does
/// not have is unavailable and refused.
#[test]
#[ignore = "changes the running kernel's parameters; run by `cargo xtask debian`"]
fn the_running_kernel_s_parameters_are_read_and_written() {
    let _host = exclusive();
    let file = "/proc/sys/kernel/domainname";
    let original = std::fs::read_to_string(file).unwrap();
    let key = sysctl("kernel.domainname");
    let mut host = linux();

    let seen = observe_one(&mut host, &key);
    assert!(
        matches!(&seen, Collection::Collected(Evidence::Sysctl(e)) if e.value == SysctlValue::normalized(&original)),
        "{seen:?}"
    );
    let receipts = host.apply(&converge(&key, 1, value("nomos-test.example")));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    assert_eq!(
        std::fs::read_to_string(file).unwrap().trim(),
        "nomos-test.example"
    );
    let receipts = host.apply(&converge(&key, 2, value("nomos-test.example")));
    assert_eq!(
        receipts.last(),
        Some(&Receipt::Completed { changed: false })
    );

    // Longer than the kernel's 64 bytes: the kernel keeps the first 64
    // without an error, so only reading the value back shows that it kept
    // another value than the one asked for, and the receipt is Failed.
    let long = "x".repeat(100);
    let receipts = host.apply(&converge(&key, 3, value(&long)));
    assert_eq!(receipts.last(), Some(&Receipt::Failed));
    assert_eq!(std::fs::read_to_string(file).unwrap().trim(), &long[..64]);

    let missing = sysctl("kernel.nomos_nowhere");
    assert_eq!(
        observe_one(&mut host, &missing),
        Collection::Failed(CollectionFailure::Unavailable)
    );
    assert_eq!(
        host.apply(&converge(&missing, 4, value("1"))),
        vec![Receipt::Refused]
    );
    std::fs::write(file, original).unwrap();
}

/// The account as `getent` reports it through the name service: name,
/// ID, home, and shell, or nothing.
fn getent(name: &str) -> Option<(u32, String, String)> {
    let out = Command::new("getent")
        .args(["passwd", name])
        .output()
        .unwrap();
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).unwrap();
    let f: Vec<&str> = text.trim_end().split(':').collect();
    Some((f[2].parse().unwrap(), f[5].to_string(), f[6].to_string()))
}

fn account(name: &str) -> ResourceKey {
    ResourceKey::User(AccountName::new(name).unwrap())
}

fn path(text: &str) -> Option<ResourcePath> {
    Some(ResourcePath::new(text).unwrap())
}

/// A system account is created, changed, and deleted through the host's
/// own tools, without `--prefix`; an account whose ID another account also
/// holds is refused before any effect.
#[test]
#[ignore = "changes the host's user database; run by `cargo xtask debian`"]
fn an_account_is_managed_on_the_host() {
    let _host = exclusive();
    let name = "nomos-svc";
    let key = account(name);
    let _ = Command::new("userdel").arg(name).status();
    let _ = Command::new("userdel").arg("nomos-alias").status();
    let mut host = linux();

    let create = Requirement::User(UserCondition::Present {
        class: AccountClass::System,
        home: path("/var/lib/nomos-svc"),
        shell: path("/usr/sbin/nologin"),
    });
    let receipts = host.apply(&converge(&key, 1, create));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    let (uid, home, shell) = getent(name).expect("the account was not created");
    assert!(uid < 1000, "a system account got {uid}");
    assert_eq!(
        (home.as_str(), shell.as_str()),
        ("/var/lib/nomos-svc", "/usr/sbin/nologin")
    );
    assert!(
        !Path::new("/var/lib/nomos-svc").exists(),
        "the home directory was created"
    );

    let change = Requirement::User(UserCondition::Present {
        class: AccountClass::System,
        home: None,
        shell: path("/bin/sh"),
    });
    let receipts = host.apply(&converge(&key, 2, change.clone()));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    assert_eq!(getent(name).unwrap().2, "/bin/sh");

    // An alias: a second name for the same ID.
    let status = Command::new("useradd")
        .args([
            "--non-unique",
            "--uid",
            &uid.to_string(),
            "--no-create-home",
        ])
        .arg("nomos-alias")
        .status()
        .unwrap();
    assert!(status.success());
    let receipts = host.apply(&converge(&key, 3, Requirement::User(UserCondition::Absent)));
    assert_eq!(receipts, vec![Receipt::Refused]);
    assert!(getent(name).is_some(), "an aliased account was deleted");
    assert!(
        Command::new("userdel")
            .arg("nomos-alias")
            .status()
            .unwrap()
            .success()
    );

    let receipts = host.apply(&converge(&key, 4, Requirement::User(UserCondition::Absent)));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    assert_eq!(getent(name), None);
}

// ---------------------------------------------------------------------------
// Packages

use conformance::families::{self, Contents, World};
use debian::{REPO, build, package_truth, publish, readable_status, run};
use nomos_app::driver::Cell;
use nomos_app::kernel::{Canon, Input, Managed, Plan, Policy, RunOutcome};
use nomos_core::assessment::Reason;
use nomos_core::condition::{Condition, PackageCondition, PackageVersion};
use nomos_core::observation::{Observation, PackageEvidence};
use nomos_core::resource::{Digest, Family, PackageName};

/// The host's packages, as a world the family suite arranges.
struct PackageSubject {
    host: LinuxHost,
}

impl PackageSubject {
    fn new() -> Self {
        conformance::drop_permission_override();
        PackageSubject { host: linux() }
    }
}

impl Observe for PackageSubject {
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        self.host.observe(resources)
    }
}

impl Mutate for PackageSubject {
    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        self.host.apply(request)
    }
}

impl World for PackageSubject {
    fn families(&self) -> Vec<Family> {
        vec![Family::Package]
    }
    fn arrange(&mut self, key: &ResourceKey, evidence: Option<Evidence>) {
        readable_status();
        let name = key.name();
        assert!(run("dpkg", &["--purge", name]) || package_truth(name).is_none());
        match evidence {
            Some(Evidence::Package(PackageEvidence::Installed { version })) => {
                assert!(run(
                    "dpkg",
                    &["--install", &build(name, version.as_str(), None)]
                ));
            }
            Some(Evidence::Package(PackageEvidence::Broken)) => {
                assert!(run("dpkg", &["--unpack", &build(name, "1.0-1", None)]));
            }
            _ => {}
        }
    }
    fn deny(&mut self, _key: &ResourceKey) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            "/var/lib/dpkg/status",
            std::fs::Permissions::from_mode(0o000),
        )
        .unwrap();
    }
    fn truth(&self, key: &ResourceKey) -> Option<Evidence> {
        package_truth(key.name()).map(Evidence::Package)
    }
    fn provide(&mut self, key: &ResourceKey, _requirement: &Requirement) {
        build(key.name(), "1.0-1", None);
        build(key.name(), "2.0-1", None);
        publish();
    }
    fn executions(&self) -> usize {
        self.host.executions()
    }
    fn now(&mut self) -> Instant {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        Instant(t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64)
    }
    fn contents(&mut self) -> Contents {
        let mut out = [(Digest::from_bytes([0; 32]), 0); 3];
        for (i, bytes) in [&b"one"[..], b"two!", b"three"].into_iter().enumerate() {
            out[i] = (self.host.add_content(bytes.to_vec()), bytes.len() as u64);
        }
        Contents(out)
    }
    fn refusals(&mut self, family: Family) -> Vec<Apply> {
        let c = self.contents();
        let at = |i| families::resource(family, "s7", i);
        let mut requests = vec![families::wrong_family(&at(0), 70)];
        // A refresh, which a package does not have.
        let pkg = at(1);
        self.arrange(&pkg, families::starts(family, &c)[1].clone());
        requests.push(Apply {
            operation: Operation::Refresh(families::requirements(family, &c)[1].clone()),
            ..families::converge(&pkg, 79, families::requirements(family, &c)[1].clone())
        });
        // A package another depends on, asked to be absent: removing it
        // would remove the other too.
        let base = at(2);
        let dependent = "pkgs7-dependent";
        readable_status();
        run("dpkg", &["--purge", dependent]);
        self.arrange(&base, families::starts(family, &c)[1].clone());
        build(dependent, "1.0-1", Some(base.name()));
        self.provide(&base, &families::requirements(family, &c)[0]);
        assert!(run(
            "dpkg",
            &["--install", &format!("{REPO}/{dependent}_1.0-1_all.deb")]
        ));
        requests.push(families::converge(
            &base,
            81,
            Requirement::Package(PackageCondition::Absent),
        ));
        // A version the index does not offer.
        let unoffered = at(3);
        self.arrange(&unoffered, None);
        requests.push(families::converge(
            &unoffered,
            82,
            Requirement::Package(PackageCondition::Installed {
                version: Some(PackageVersion::new("9.0-1").unwrap()),
            }),
        ));
        requests
    }
}

#[test]
#[ignore = "changes the host's packages; run by `cargo xtask debian`"]
fn the_linux_adapter_passes_the_suite_for_packages() {
    let _host = exclusive();
    let mut s = PackageSubject::new();
    let results = families::all(&mut s, Family::Package);
    readable_status();
    println!("Package: {:?}", conformance::report(&results));
    let failures: Vec<String> = results
        .into_iter()
        .filter_map(|(k, r)| r.err().map(|e| format!("{k}: {e}")))
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

fn policy() -> Policy {
    Policy::new(4, 120_000_000_000, 120_000_000_000, 3_000_000_000)
}

fn plan(id: &str, generation: u64, canon: Canon) -> Plan {
    Plan {
        id: PlanId::new(id).unwrap(),
        generation: Generation(generation),
        canon,
        bound: 4,
        policy: policy(),
        expires: None,
    }
}

fn package_canon(name: &str, version: &str) -> Canon {
    Canon::new(
        vec![Managed {
            condition: Condition::new(
                ResourceKey::Package(PackageName::new(name).unwrap()),
                Requirement::Package(PackageCondition::Installed {
                    version: Some(PackageVersion::new(version).unwrap()),
                }),
            )
            .unwrap(),
            keys: std::collections::BTreeSet::new(),
            disrupts: std::collections::BTreeSet::new(),
        }],
        vec![],
    )
}

fn clock() -> Instant {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    Instant(t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64)
}

/// The milestone's exit: while another package manager holds dpkg's lock,
/// a package is Indeterminate and nothing is executed, never a false
/// Variance; once it is released, a pin converges, and a second Enforce
/// executes nothing (spec §39, N3).
#[test]
#[ignore = "changes the host's packages; run by `cargo xtask debian`"]
fn a_held_lock_is_indeterminate_and_a_pin_converges_to_a_fixed_point() {
    let _host = exclusive();
    let name = "nomos-pinned";
    readable_status();
    run("dpkg", &["--purge", name]);
    build(name, "1.0-1", None);
    build(name, "2.0-1", None);
    publish();
    let key = ResourceKey::Package(PackageName::new(name).unwrap());
    let canon = package_canon(name, "1.0-1");
    let mut host = linux();

    // Another package manager, as far as dpkg can tell: a POSIX record
    // lock on lock-frontend, which is the lock apt-get takes.
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open("/var/lib/dpkg/lock-frontend")
        .unwrap();
    rustix::fs::fcntl_lock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    let mut cell = Cell::open(support::MemLog::default());
    cell.settle(Input::Tick(clock()), &mut host).unwrap();
    cell.settle(Input::Enforce(plan("locked", 1, canon.clone())), &mut host)
        .unwrap();
    assert_eq!(
        cell.snapshot().outcome(),
        Some(&RunOutcome::Indeterminate(vec![(
            key.clone(),
            Reason::CollectionFailed(CollectionFailure::TimedOut)
        )]))
    );
    assert_eq!(host.executions(), 0);
    drop(lock);

    cell.settle(Input::Tick(clock()), &mut host).unwrap();
    cell.settle(Input::Enforce(plan("pinned", 2, canon.clone())), &mut host)
        .unwrap();
    assert_eq!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    assert_eq!(
        package_truth(name),
        Some(PackageEvidence::Installed {
            version: PackageVersion::new("1.0-1").unwrap()
        })
    );
    let executions = host.executions();
    cell.settle(Input::Tick(clock()), &mut host).unwrap();
    cell.settle(Input::Enforce(plan("again", 3, canon)), &mut host)
        .unwrap();
    assert_eq!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    assert_eq!(
        host.executions(),
        executions,
        "a converged package executed again"
    );
    run("dpkg", &["--purge", name]);
}
