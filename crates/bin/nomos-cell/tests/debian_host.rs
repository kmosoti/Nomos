//! Kernel parameters and users on a real Debian host (Phase 1 plan,
//! `12-sysctl-and-user`): the running kernel's own `/proc/sys`, and the
//! host's user database changed without `--prefix`, as production does.
//!
//! Every test here changes the host, so every one is ignored on an
//! ordinary run and runs in a disposable container: `cargo xtask debian`
//! runs this binary with `--include-ignored`. The ground truth is read
//! from `/proc/sys` and with `getent`, never through the adapter.

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
