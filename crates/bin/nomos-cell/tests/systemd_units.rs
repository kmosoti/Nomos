//! The unit family on Linux, and experiment `systemd-refresh` (Phase 1
//! plan, `11-systemd-unit`): does a refresh survive a crash between the
//! change and the job, on a real service manager?
//!
//! Every test here changes the host's service manager, so every one is
//! ignored on an ordinary run and runs where systemd is PID 1 in a
//! disposable container: `cargo xtask debian` runs this binary with
//! `--include-ignored`. The tests arrange units as a foreign writer would,
//! with `systemctl`, and read the ground truth with `systemctl show`,
//! never through the adapter (substrate-contract.md, Units on Linux).

mod conformance;
mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use conformance::families::{self, Contents, World};
use nomos_app::driver::{Cell, JournaledCell};
use nomos_app::kernel::{Canon, Input, Managed, Plan, Policy, RunOutcome};
use nomos_core::assessment::Reason;
use nomos_core::condition::{
    Activity, Condition, Content, Enablement, FileCondition, Requirement, UnitCondition,
};
use nomos_core::effect::{Apply, Receipt};
use nomos_core::observation::{
    ActiveState, Collection, CollectionFailure, Evidence, Instant, Observation, UnitEvidence,
    UnitFileState,
};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{Digest, Family, ResourceKey, ResourcePath, UnitName};
use nomos_store_fs::FileLog;
use nomos_substrate::{Mutate, Observe};
use nomos_substrate_linux::LinuxHost;
use nomos_substrate_linux::units::Systemd;
use nomos_warp::graph::{Edge, EdgeKind};

// ---------------------------------------------------------------------------
// systemd, as a foreign writer

fn systemctl(args: &[&str]) -> (bool, String) {
    let out = Command::new("systemctl").args(args).output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn must(args: &[&str]) -> String {
    let (ok, out) = systemctl(args);
    assert!(ok, "systemctl {args:?} failed");
    out
}

fn unit_path(name: &str) -> PathBuf {
    Path::new("/etc/systemd/system").join(name)
}

/// Writes a unit file whose service runs `service`, with an install
/// section unless `fixed`, and reloads systemd.
fn write_unit(name: &str, service: &str, fixed: bool) {
    let install = if fixed {
        ""
    } else {
        "[Install]\nWantedBy=multi-user.target\n"
    };
    std::fs::write(
        unit_path(name),
        format!("[Unit]\nDescription=Nomos test unit\n[Service]\n{service}{install}"),
    )
    .unwrap();
    must(&["daemon-reload"]);
}

/// Stops, disables, and removes a unit, leaving systemd without it.
fn remove_unit(name: &str) {
    systemctl(&["stop", name]);
    systemctl(&["disable", name]);
    systemctl(&["reset-failed", name]);
    let _ = std::fs::remove_file(unit_path(name));
    must(&["daemon-reload"]);
}

/// The ground truth of a unit, read with `systemctl show`: its active and
/// unit-file state, or `None` when systemd cannot load it. The mapping is
/// written here from systemd's documentation, not taken from the adapter.
fn truth(name: &str) -> Option<UnitEvidence> {
    let out = must(&[
        "show",
        "-p",
        "LoadState",
        "-p",
        "ActiveState",
        "-p",
        "UnitFileState",
        name,
    ]);
    let field = |k: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_default()
            .to_string()
    };
    if !matches!(field("LoadState").as_str(), "loaded" | "masked") {
        return None;
    }
    let active = match field("ActiveState").as_str() {
        "active" => ActiveState::Active,
        "reloading" => ActiveState::Reloading,
        "inactive" => ActiveState::Inactive,
        "failed" => ActiveState::Failed,
        "activating" => ActiveState::Activating,
        "deactivating" => ActiveState::Deactivating,
        other => panic!("{name}: active state {other}"),
    };
    let file_state = match field("UnitFileState").as_str() {
        "enabled" => UnitFileState::Enabled,
        "disabled" => UnitFileState::Disabled,
        "static" => UnitFileState::Static,
        "masked" | "masked-runtime" => UnitFileState::Masked,
        _ => UnitFileState::Other,
    };
    Some(UnitEvidence { active, file_state })
}

/// Waits, for at most ten seconds, until `name` is in `state`.
fn wait_for(name: &str, state: ActiveState) {
    for _ in 0..500 {
        if truth(name).is_some_and(|u| u.active == state) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{name} never became {state:?}: {:?}", truth(name));
}

/// systemd's escaping of a unit name in its object path.
fn object_path(name: &str) -> String {
    let mut out = String::from("/org/freedesktop/systemd1/unit/");
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else {
            out.push_str(&format!("_{b:02x}"));
        }
    }
    out
}

/// Denies every message to the unit's object path on the system bus, with
/// a mandatory policy that binds root too, and reloads the bus.
fn deny_on_bus(name: &str) {
    let policy = format!(
        "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN\" \
         \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n\
         <busconfig><policy context=\"mandatory\">\
         <deny send_destination=\"org.freedesktop.systemd1\" send_path=\"{}\"/>\
         </policy></busconfig>\n",
        object_path(name)
    );
    std::fs::write(
        format!("/etc/dbus-1/system.d/nomos-deny-{name}.conf"),
        policy,
    )
    .unwrap();
    let out = Command::new("busctl")
        .args([
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ReloadConfig",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "the bus did not reload its policy");
}

/// One test at a time: they share the host's service manager and bus.
static MANAGER: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    MANAGER.lock().unwrap_or_else(|e| e.into_inner())
}

const SLEEPER: &str = "ExecStart=/bin/sleep infinity\n";

fn linux() -> LinuxHost {
    LinuxHost::open(Path::new("/"))
        .unwrap()
        .with_systemd(Systemd::system().unwrap())
}

fn now() -> Instant {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    Instant(t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64)
}

fn unit_key(name: &str) -> ResourceKey {
    ResourceKey::Unit(UnitName::new(name).unwrap())
}

fn observe_one(host: &mut LinuxHost, key: &ResourceKey) -> Collection {
    let observed: Vec<Observation> = host.observe(std::slice::from_ref(key));
    assert_eq!(observed.len(), 1);
    observed[0].collection().clone()
}

// ---------------------------------------------------------------------------
// The per-family suite

/// The Linux host serving units, as a world the family suite arranges.
struct UnitSubject {
    host: LinuxHost,
    denied: BTreeSet<String>,
}

impl Observe for UnitSubject {
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        self.host.observe(resources)
    }
}

impl Mutate for UnitSubject {
    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        self.host.apply(request)
    }
}

impl World for UnitSubject {
    fn families(&self) -> Vec<Family> {
        vec![Family::Unit]
    }
    fn arrange(&mut self, key: &ResourceKey, evidence: Option<Evidence>) {
        let name = key.name();
        assert!(!self.denied.contains(name), "{name} is denied on the bus");
        remove_unit(name);
        let Some(Evidence::Unit(u)) = evidence else {
            return;
        };
        write_unit(name, SLEEPER, u.file_state == UnitFileState::Static);
        match u.file_state {
            UnitFileState::Enabled => {
                must(&["enable", name]);
            }
            UnitFileState::Disabled | UnitFileState::Static => {}
            other => panic!("{name}: {other:?} is not a starting point on Linux"),
        }
        match u.active {
            ActiveState::Active => {
                must(&["start", name]);
            }
            ActiveState::Inactive => {}
            ActiveState::Failed => {
                must(&["start", name]);
                must(&["kill", "--signal=KILL", name]);
                wait_for(name, ActiveState::Failed);
            }
            other => panic!("{name}: {other:?} is not a starting point on Linux"),
        }
        assert_eq!(truth(name), Some(u), "{name} was not arranged");
    }
    fn deny(&mut self, key: &ResourceKey) {
        deny_on_bus(key.name());
        self.denied.insert(key.name().to_string());
    }
    fn truth(&self, key: &ResourceKey) -> Option<Evidence> {
        truth(key.name()).map(Evidence::Unit)
    }
    fn provide(&mut self, _key: &ResourceKey, _requirement: &Requirement) {}
    fn executions(&self) -> usize {
        self.host.executions()
    }
    fn now(&mut self) -> Instant {
        now()
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
        let first = families::requirements(family, &c)[0].clone();
        // A requirement of another family; a unit systemd does not know;
        // and a static one asked to be enabled.
        let mut requests = vec![families::wrong_family(&at(0), 70)];
        let unknown = at(1);
        self.arrange(&unknown, None);
        requests.push(families::converge(&unknown, 75, first.clone()));
        let fixed = at(2);
        self.arrange(
            &fixed,
            Some(Evidence::Unit(UnitEvidence {
                active: ActiveState::Active,
                file_state: UnitFileState::Static,
            })),
        );
        requests.push(families::converge(&fixed, 76, first));
        requests
    }
    /// The stable starting points: a unit activating, deactivating, or
    /// reloading leaves that state on its own, so a clause comparing the
    /// ground truth before and after cannot hold it still. The transient
    /// states have a test of their own below.
    fn starts(&mut self, family: Family, c: &Contents) -> Vec<Option<Evidence>> {
        families::starts(family, c)
            .into_iter()
            .filter(|s| {
                !matches!(
                    s,
                    Some(Evidence::Unit(UnitEvidence {
                        active: ActiveState::Activating
                            | ActiveState::Deactivating
                            | ActiveState::Reloading,
                        ..
                    }))
                )
            })
            .collect()
    }
}

#[test]
#[ignore = "changes the service manager; run by `cargo xtask debian`"]
fn the_linux_adapter_passes_the_suite_for_units() {
    let _manager = exclusive();
    let mut s = UnitSubject {
        host: linux(),
        denied: BTreeSet::new(),
    };
    let results = families::all(&mut s, Family::Unit);
    println!("Unit: {:?}", conformance::report(&results));
    let failures: Vec<String> = results
        .into_iter()
        .filter_map(|(k, r)| r.err().map(|e| format!("{k}: {e}")))
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A unit held in each transient state for two seconds is observed in it,
/// and a convergence waits for the job and ends Satisfied.
#[test]
#[ignore = "changes the service manager; run by `cargo xtask debian`"]
fn a_unit_in_transition_is_observed_and_converged() {
    let _manager = exclusive();
    let name = "nomos-slow.service";
    let key = unit_key(name);
    let mut host = linux();
    let converge = |activity, i| {
        families::converge(
            &key,
            i,
            Requirement::Unit(UnitCondition {
                activity,
                enablement: Enablement::Any,
            }),
        )
    };
    let slow = "ExecStartPre=/bin/sleep 2\nExecStart=/bin/sleep infinity\n\
                ExecReload=/bin/sleep 2\nExecStop=/bin/sleep 2\n";
    remove_unit(name);
    write_unit(name, slow, false);

    must(&["start", "--no-block", name]);
    wait_for(name, ActiveState::Activating);
    let seen = observe_one(&mut host, &key);
    assert!(
        matches!(seen, Collection::Collected(Evidence::Unit(u)) if u.active == ActiveState::Activating),
        "{seen:?}"
    );
    let receipts = host.apply(&converge(Activity::Active, 1));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    assert_eq!(truth(name).unwrap().active, ActiveState::Active);

    must(&["reload", "--no-block", name]);
    wait_for(name, ActiveState::Reloading);
    let seen = observe_one(&mut host, &key);
    assert!(
        matches!(seen, Collection::Collected(Evidence::Unit(u)) if u.active == ActiveState::Reloading),
        "{seen:?}"
    );
    let receipts = host.apply(&converge(Activity::Active, 2));
    assert!(matches!(receipts.last(), Some(Receipt::Completed { .. })));
    // A stop while the reload runs would kill the reload and fail the
    // unit; the stop is arranged from a running unit.
    wait_for(name, ActiveState::Active);

    must(&["stop", "--no-block", name]);
    wait_for(name, ActiveState::Deactivating);
    let seen = observe_one(&mut host, &key);
    assert!(
        matches!(seen, Collection::Collected(Evidence::Unit(u)) if u.active == ActiveState::Deactivating),
        "{seen:?}"
    );
    let receipts = host.apply(&converge(Activity::Inactive, 3));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    assert_eq!(truth(name).unwrap().active, ActiveState::Inactive);
    remove_unit(name);
}

/// A job still pending at the settle-by instant is cancelled, and no
/// receipt settles the execution: its outcome is unknown (N10).
#[test]
#[ignore = "changes the service manager; run by `cargo xtask debian`"]
fn a_job_pending_at_the_deadline_is_cancelled_and_unsettled() {
    let _manager = exclusive();
    let name = "nomos-stuck.service";
    let key = unit_key(name);
    remove_unit(name);
    write_unit(
        name,
        "ExecStartPre=/bin/sleep 30\nExecStart=/bin/sleep infinity\n",
        false,
    );
    let mut host = linux();
    let requirement = Requirement::Unit(UnitCondition {
        activity: Activity::Active,
        enablement: Enablement::Any,
    });
    let request = Apply {
        settle_by: Instant(now().0 + 1_000_000_000),
        ..families::converge(&key, 1, requirement)
    };
    let receipts = host.apply(&request);
    assert_eq!(receipts, vec![Receipt::Accepted, Receipt::Started]);
    assert!(!receipts.iter().any(Receipt::settles));
    remove_unit(name);
}

// ---------------------------------------------------------------------------
// The milestone's exit, through the production driver

/// Timeouts long enough for a real service manager: receipts and
/// verification within a minute, effects settled three seconds after
/// dispatch.
fn policy() -> Policy {
    Policy::new(4, 60_000_000_000, 60_000_000_000, 3_000_000_000)
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

fn managed(condition: Condition) -> Managed {
    Managed {
        condition,
        keys: BTreeSet::new(),
        disrupts: BTreeSet::new(),
    }
}

fn unit(name: &str, activity: Activity, enablement: Enablement) -> Managed {
    managed(Condition::unit(
        UnitName::new(name).unwrap(),
        UnitCondition {
            activity,
            enablement,
        },
    ))
}

/// A job that fails is reported Failed by the adapter, and leaves the
/// Action Failed, not Converged.
#[test]
#[ignore = "changes the service manager; run by `cargo xtask debian`"]
fn a_job_that_fails_leaves_the_action_failed() {
    let _manager = exclusive();
    let name = "nomos-fails.service";
    remove_unit(name);
    write_unit(
        name,
        "ExecStartPre=/bin/false\nExecStart=/bin/sleep infinity\n",
        false,
    );
    let mut host = linux();
    // The adapter's own receipt says so; the kernel's verification, which
    // would also refuse the Action, is not what this checks.
    let requirement = Requirement::Unit(UnitCondition {
        activity: Activity::Active,
        enablement: Enablement::Any,
    });
    let receipts = host.apply(&families::converge(&unit_key(name), 1, requirement));
    assert_eq!(
        receipts,
        vec![Receipt::Accepted, Receipt::Started, Receipt::Failed],
        "a failed start job was reported as done"
    );
    systemctl(&["reset-failed", name]);

    let canon = Canon::new(vec![unit(name, Activity::Active, Enablement::Any)], vec![]);
    let mut cell = Cell::open(support::MemLog::default());
    cell.settle(Input::Tick(now()), &mut host).unwrap();
    cell.settle(Input::Enforce(plan("fails", 1, canon)), &mut host)
        .unwrap();
    assert_eq!(
        cell.snapshot().outcome(),
        Some(&RunOutcome::Failed {
            failed: vec![unit_key(name)],
            unknown: vec![],
        })
    );
    assert_eq!(truth(name).unwrap().active, ActiveState::Failed);
    remove_unit(name);
}

/// A unit systemd cannot load is Indeterminate with its reason, and
/// nothing is executed (N13).
#[test]
#[ignore = "changes the service manager; run by `cargo xtask debian`"]
fn a_unit_systemd_cannot_load_is_indeterminate() {
    let _manager = exclusive();
    let name = "nomos-nowhere.service";
    remove_unit(name);
    let canon = Canon::new(
        vec![unit(name, Activity::Active, Enablement::Enabled)],
        vec![],
    );
    let mut host = linux();
    let mut cell = Cell::open(support::MemLog::default());
    cell.settle(Input::Tick(now()), &mut host).unwrap();
    cell.settle(Input::Enforce(plan("nowhere", 1, canon)), &mut host)
        .unwrap();
    assert_eq!(
        cell.snapshot().outcome(),
        Some(&RunOutcome::Indeterminate(vec![(
            unit_key(name),
            Reason::CollectionFailed(CollectionFailure::Unavailable)
        )]))
    );
    assert_eq!(host.executions(), 0);
}

const CONFIG: &str = "/etc/nomos-refresh/app.conf";
const LOADED: &str = "/run/nomos-refresh.loaded";
const SERVICE: &str = "nomos-refresh.service";

/// The refresh scenario of `05-transition-kernel` on a real host: a
/// configuration file, and a service that reads it when it starts and
/// must be restarted when it changes.
fn refresh_canon(new: Digest) -> Canon {
    let config = ResourcePath::new(CONFIG).unwrap();
    Canon::new(
        vec![
            managed(Condition::file(
                config.clone(),
                FileCondition::present(Content::Exactly(new)),
            )),
            unit(SERVICE, Activity::Active, Enablement::Enabled),
        ],
        vec![Edge::new(
            ResourceKey::File(config),
            unit_key(SERVICE),
            EdgeKind::OnChange,
        )],
    )
}

/// The service runs the old configuration, which is on disk.
fn arrange_refresh(old: &[u8]) {
    remove_unit(SERVICE);
    std::fs::create_dir_all("/etc/nomos-refresh").unwrap();
    std::fs::write(CONFIG, old).unwrap();
    write_unit(
        SERVICE,
        &format!("ExecStartPre=/bin/cp {CONFIG} {LOADED}\n{SLEEPER}"),
        false,
    );
    must(&["enable", "--now", SERVICE]);
    assert_eq!(std::fs::read(LOADED).unwrap(), old);
}

fn open(path: &Path) -> JournaledCell<FileLog<Input>> {
    let (log, _) = FileLog::open(path).unwrap();
    JournaledCell::open(log)
}

/// Experiment `systemd-refresh`, the milestone's exit: for every input of
/// the scenario, the Cell is killed right after it, reopened from its
/// journal alone, and must hold the snapshot it had, its Obligations
/// included, and then end with the service restarted on the new
/// configuration.
#[test]
#[ignore = "changes the service manager; run by `cargo xtask debian`"]
fn a_refresh_survives_a_crash_between_the_change_and_the_restart() {
    let _manager = exclusive();
    let old = b"setting = old\n";
    let new = b"setting = new\n";
    let journals = std::env::temp_dir().join(format!("nomos-refresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&journals);
    std::fs::create_dir_all(&journals).unwrap();

    // The uninterrupted run, which gives the number of inputs.
    arrange_refresh(old);
    let mut host = linux();
    let canon = refresh_canon(host.add_content(new.to_vec()));
    let mut cell = open(&journals.join("reference"));
    cell.settle(Input::Tick(now()), &mut host).unwrap();
    cell.settle(Input::Enforce(plan("p1", 1, canon.clone())), &mut host)
        .unwrap();
    assert_eq!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    assert_eq!(std::fs::read(LOADED).unwrap(), new);
    let inputs = nomos_store::EventLog::events(cell.journal()).len();
    drop(cell);

    let mut owed = 0;
    for kill in 1..=inputs {
        arrange_refresh(old);
        let mut host = linux();
        host.add_content(new.to_vec());
        let path = journals.join(format!("kill-{kill}"));
        let mut cell = open(&path);
        let mut queue = std::collections::VecDeque::from([
            Input::Tick(now()),
            Input::Enforce(plan("p1", 1, canon.clone())),
        ]);
        for _ in 0..kill {
            let Some(input) = queue.pop_front() else {
                break;
            };
            queue.extend(cell.handle(input, &mut host).unwrap());
        }
        // The process dies: its queue and snapshot are gone; the host keeps
        // what was done to it, and the journal what was appended.
        let snapshot = cell.snapshot().clone();
        if !snapshot.obligations().is_empty() {
            owed += 1;
        }
        drop(cell);
        let mut cell = open(&path);
        assert_eq!(
            cell.snapshot(),
            &snapshot,
            "after input {kill}: the recovered snapshot differs"
        );
        cell.settle(Input::Recovered, &mut host).unwrap();
        // Past every settle-by instant, on the host's clock and the
        // kernel's, so that what was in flight is Settled by time.
        std::thread::sleep(Duration::from_millis(3_100));
        cell.settle(Input::Tick(now()), &mut host).unwrap();
        cell.settle(Input::Enforce(plan("p2", 2, canon.clone())), &mut host)
            .unwrap();
        assert_eq!(
            cell.snapshot().outcome(),
            Some(&RunOutcome::Converged),
            "killed after input {kill}"
        );
        assert_eq!(
            std::fs::read(LOADED).unwrap(),
            new,
            "killed after input {kill}: the service runs the old configuration"
        );
        assert!(cell.snapshot().obligations().is_empty());
    }
    assert!(owed > 0, "no kill point owed a refresh");
    println!("killed after each of {inputs} inputs; {owed} of them with a refresh owed");
    remove_unit(SERVICE);
    let _ = std::fs::remove_dir_all(&journals);
}
