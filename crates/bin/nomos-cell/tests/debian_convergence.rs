//! Experiment `debian-convergence`, the Phase 1 exit (plan,
//! `15-debian-convergence`): from every enumerated starting state,
//! `enforce` converges the Debian host, a second `enforce` executes
//! nothing (spec §39, N3), and `trace` afterward reports every Condition
//! Satisfied.
//!
//! The demonstration Canon is a slice of spec §56: a system account, a
//! state directory it owns, a configuration directory and file, a kernel
//! parameter, a package, and a service that runs as the account, reads
//! the configuration when it starts, and is refreshed when it changes.
//! The starting states are enumerated per Condition, never left open: the
//! host with nothing, the host with everything wrong, the converged host,
//! and the converged host with each Condition in turn absent and wrong.
//! Every test changes the host, so every one is ignored on an ordinary
//! run and runs in a disposable container through `cargo xtask debian`.

mod debian;

use std::path::{Path, PathBuf};
use std::process::Command;

use debian::{build, package_truth, publish};
use nomos_canon::artifact::{Profile, encode};
use nomos_canon::model::{Canon, CanonBuilder, RelationKind};
use nomos_core::condition::{
    AccountClass, Activity, Content, DirectoryCondition, Enablement, FileCondition, Metadata,
    PackageCondition, PackageVersion, UnitCondition, UserCondition,
};
use nomos_core::resource::{AccountName, Digest, Mode, ResourcePath};

const ACCOUNT: &str = "nomos-demo";
const STATE_DIR: &str = "/var/lib/nomos-demo";
const CONF_DIR: &str = "/etc/nomos-demo";
const CONF: &str = "/etc/nomos-demo/demo.conf";
const PACKAGE: &str = "nomos-demo-tool";
const SERVICE: &str = "nomos-demo.service";
const LOADED: &str = "/var/lib/nomos-demo/loaded.conf";
const DOMAIN: &str = "demo.nomos.example";
const CONFIG: &[u8] = b"greeting = hello\n";

fn digest(bytes: &[u8]) -> Digest {
    Digest::from_bytes(nomos_canon::sha256::digest(bytes))
}

fn path(p: &str) -> ResourcePath {
    ResourcePath::new(p).unwrap()
}

fn mode(m: &str) -> Mode {
    Mode::from_octal(m).unwrap()
}

/// The demonstration Canon.
pub fn demonstration() -> Canon {
    let meta = |owner: Option<&str>, bits: &str| Metadata {
        owner: owner.map(|o| AccountName::new(o).unwrap()),
        group: None,
        mode: Some(mode(bits)),
    };
    CanonBuilder::new("debian-demonstration")
        .user(
            ACCOUNT,
            UserCondition::Present {
                class: AccountClass::System,
                home: Some(path(STATE_DIR)),
                shell: Some(path("/usr/sbin/nologin")),
            },
        )
        .directory(
            STATE_DIR,
            DirectoryCondition::Present {
                metadata: meta(Some(ACCOUNT), "0750"),
            },
        )
        .directory(
            CONF_DIR,
            DirectoryCondition::Present {
                metadata: meta(Some("root"), "0755"),
            },
        )
        .file(
            CONF,
            FileCondition::Present {
                content: Content::Exactly(digest(CONFIG)),
                metadata: meta(Some("root"), "0644"),
            },
        )
        .sysctl("kernel.domainname", DOMAIN)
        .package(
            PACKAGE,
            PackageCondition::Installed {
                version: Some(PackageVersion::new("1.0-1").unwrap()),
            },
        )
        .unit(
            SERVICE,
            UnitCondition {
                activity: Activity::Active,
                enablement: Enablement::Enabled,
            },
        )
        .relate(
            &format!("user:{ACCOUNT}"),
            RelationKind::Requires,
            &format!("directory:{STATE_DIR}"),
        )
        .relate(
            &format!("directory:{CONF_DIR}"),
            RelationKind::Requires,
            &format!("file:{CONF}"),
        )
        .relate(
            &format!("directory:{STATE_DIR}"),
            RelationKind::Requires,
            &format!("unit:{SERVICE}"),
        )
        .relate(
            &format!("package:{PACKAGE}"),
            RelationKind::Requires,
            &format!("unit:{SERVICE}"),
        )
        .relate(
            &format!("file:{CONF}"),
            RelationKind::OnChange,
            &format!("unit:{SERVICE}"),
        )
        .build()
        .unwrap()
}

fn sh(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
        .status
        .success()
}

fn cell(args: &[&str]) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = nomos_cell::cli::run(&args, &mut out, &mut err);
    let text = String::from_utf8(out).unwrap() + &String::from_utf8(err).unwrap();
    (status, text)
}

/// A resource of the demonstration, for the starting states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resource {
    User,
    StateDir,
    ConfDir,
    Conf,
    Sysctl,
    Package,
    Unit,
}

const RESOURCES: [Resource; 7] = [
    Resource::User,
    Resource::StateDir,
    Resource::ConfDir,
    Resource::Conf,
    Resource::Sysctl,
    Resource::Package,
    Resource::Unit,
];

/// What a starting state holds of one resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holds {
    /// None of it: no account, no directory, no file, a package not
    /// installed, a service stopped after failing. A kernel parameter has
    /// no absence and holds another value.
    Absent,
    /// It, with something the Canon does not ask for.
    Wrong,
}

/// The service's unit file, which the Canon does not manage: it runs as
/// the account and copies the configuration into its state directory
/// when it starts, with no limit on how often it may start.
const UNIT_FILE: &str = concat!(
    "[Unit]\n",
    "Description=Nomos demonstration service\n",
    // The experiment restarts the service more often than systemd's
    // default of five starts in ten seconds allows; a real service that
    // hits its limit fails its Action until the limit's window passes.
    "StartLimitIntervalSec=0\n",
    "[Service]\n",
    "User=nomos-demo\n",
    "ExecStartPre=+/bin/cp /etc/nomos-demo/demo.conf /var/lib/nomos-demo/loaded.conf\n",
    "ExecStart=/bin/sleep infinity\n",
    "[Install]\n",
    "WantedBy=multi-user.target\n",
);

/// Makes one resource hold `holds`, as a writer other than Nomos would.
fn perturb(resource: Resource, holds: Holds) {
    match (resource, holds) {
        (Resource::User, Holds::Absent) => {
            // An account in use cannot be deleted: the service stops first.
            sh("systemctl", &["stop", SERVICE]);
            sh("userdel", &[ACCOUNT]);
        }
        (Resource::User, Holds::Wrong) => {
            if !sh("usermod", &["--shell", "/bin/sh", ACCOUNT]) {
                assert!(sh(
                    "useradd",
                    &[
                        "--system",
                        "--no-create-home",
                        "--home-dir",
                        STATE_DIR,
                        "--shell",
                        "/bin/sh",
                        ACCOUNT
                    ]
                ));
            }
        }
        (Resource::StateDir, Holds::Absent) => {
            sh("systemctl", &["stop", SERVICE]);
            let _ = std::fs::remove_dir_all(STATE_DIR);
        }
        (Resource::StateDir, Holds::Wrong) => {
            std::fs::create_dir_all(STATE_DIR).unwrap();
            sh("chown", &["root:root", STATE_DIR]);
            sh("chmod", &["0777", STATE_DIR]);
        }
        (Resource::ConfDir, Holds::Absent) => {
            let _ = std::fs::remove_dir_all(CONF_DIR);
        }
        (Resource::ConfDir, Holds::Wrong) => {
            std::fs::create_dir_all(CONF_DIR).unwrap();
            sh("chmod", &["0700", CONF_DIR]);
        }
        (Resource::Conf, Holds::Absent) => {
            let _ = std::fs::remove_file(CONF);
        }
        (Resource::Conf, Holds::Wrong) => {
            std::fs::create_dir_all(CONF_DIR).unwrap();
            std::fs::write(CONF, b"greeting = goodbye\n").unwrap();
        }
        (Resource::Sysctl, _) => {
            std::fs::write("/proc/sys/kernel/domainname", "other.example\n").unwrap();
        }
        (Resource::Package, Holds::Absent) => {
            sh("dpkg", &["--purge", PACKAGE]);
        }
        (Resource::Package, Holds::Wrong) => {
            assert!(sh("dpkg", &["--install", &build(PACKAGE, "2.0-1", None)]));
        }
        (Resource::Unit, Holds::Absent) => {
            sh("systemctl", &["start", SERVICE]);
            sh("systemctl", &["kill", "--signal=KILL", SERVICE]);
        }
        (Resource::Unit, Holds::Wrong) => {
            sh("systemctl", &["stop", SERVICE]);
            sh("systemctl", &["disable", SERVICE]);
        }
    }
}

/// The host before any test: the unit file, the repository with both
/// versions of the package, and nothing of the Canon's.
fn prepare() {
    std::fs::write(format!("/etc/systemd/system/{SERVICE}"), UNIT_FILE).unwrap();
    assert!(sh("systemctl", &["daemon-reload"]));
    build(PACKAGE, "1.0-1", None);
    build(PACKAGE, "2.0-1", None);
    publish();
}

/// The artifact and its bundle, written once.
fn artifact(dir: &Path) -> (PathBuf, PathBuf) {
    let file = dir.join("demonstration.cbor");
    std::fs::write(&file, encode(&demonstration(), Profile::Cbor)).unwrap();
    let bundle = dir.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    let name: String = digest(CONFIG)
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(bundle.join(name), CONFIG).unwrap();
    (file, bundle)
}

/// One starting state's run: `enforce` converges, the service runs the
/// configuration the Canon names, a second `enforce` executes nothing,
/// and `trace` finds every Condition Satisfied.
fn converges(label: &str, state: &str, canon: &str, bundle: &str) -> usize {
    let (status, out) = cell(&[
        "--state", state, "enforce", "--canon", canon, "--bundle", bundle,
    ]);
    assert_eq!(
        status, 0,
        "{label}: enforce did not converge:
{out}"
    );
    let executions: usize = out
        .lines()
        .find_map(|l| l.strip_prefix("executions: "))
        .and_then(|n| n.parse().ok())
        .unwrap();
    assert_eq!(
        std::fs::read(LOADED).unwrap_or_default(),
        CONFIG,
        "{label}: the service does not run the configuration"
    );
    assert_eq!(
        package_truth(PACKAGE),
        Some(nomos_core::observation::PackageEvidence::Installed {
            version: PackageVersion::new("1.0-1").unwrap()
        }),
        "{label}"
    );
    let (status, out) = cell(&["--state", state, "enforce", "--canon", canon]);
    assert_eq!(
        status, 0,
        "{label}: a second enforce:
{out}"
    );
    assert!(
        out.contains("executions: 0"),
        "{label}: a second enforce executed:
{out}"
    );
    let (status, out) = cell(&["--state", state, "trace", "--canon", canon]);
    assert_eq!(
        status, 0,
        "{label}: trace after convergence:
{out}"
    );
    assert!(
        out.contains("7 satisfied, 0 variance, 0 indeterminate; 0 actions planned"),
        "{label}:
{out}"
    );
    executions
}

/// Experiment `debian-convergence`: the host with nothing, the host with
/// everything wrong, the converged host, and the converged host with each
/// Condition in turn absent and wrong.
#[test]
#[ignore = "changes the host; run by "]
fn the_demonstration_converges_from_every_enumerated_starting_state() {
    prepare();
    let dir = std::env::temp_dir().join(format!("nomos-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (canon, bundle) = artifact(&dir);
    let state = dir.join("state");
    let (canon, bundle, state) = (
        canon.to_str().unwrap(),
        bundle.to_str().unwrap(),
        state.to_str().unwrap(),
    );
    let mut runs = Vec::new();

    // Nothing of the Canon's.
    for r in RESOURCES.iter().rev() {
        perturb(*r, Holds::Absent);
    }
    runs.push((
        "nothing".to_string(),
        converges("nothing", state, canon, bundle),
    ));

    // Everything wrong.
    for r in RESOURCES {
        perturb(r, Holds::Wrong);
    }
    runs.push((
        "everything wrong".to_string(),
        converges("everything wrong", state, canon, bundle),
    ));

    // Converged.
    runs.push((
        "converged".to_string(),
        converges("converged", state, canon, bundle),
    ));

    // Each Condition in turn, absent and wrong.
    for r in RESOURCES {
        for holds in [Holds::Absent, Holds::Wrong] {
            if r == Resource::Sysctl && holds == Holds::Absent {
                continue;
            }
            let label = format!("{r:?} {holds:?}");
            perturb(r, holds);
            runs.push((label.clone(), converges(&label, state, canon, bundle)));
        }
    }
    for (label, executions) in &runs {
        println!("{label}: converged with {executions} executions");
    }
    assert_eq!(runs[2].1, 0, "the converged host was changed");
    assert!(runs.iter().enumerate().all(|(i, (_, n))| i == 2 || *n > 0));
}
