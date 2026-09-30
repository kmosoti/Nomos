//! The installed Cell (Phase 1 plan, `16-alpha-release`): the Debian
//! package installs, runs the demonstration Canon of `15` through its
//! service, upgrades, and is removed and purged cleanly.
//!
//! The package is installed, upgraded, removed, and purged with the
//! commands the operator guide gives. It is built by `cargo xtask package`, twice, at the
//! workspace's version and at a later one, and `cargo xtask debian` puts
//! both in the container at `/root/debs/base.deb` and
//! `/root/debs/upgrade.deb` before it runs this binary. Every test here
//! changes the host, so every one is ignored on an ordinary run. Unlike
//! the other suites, this one runs the installed binary as its own
//! process, as systemd runs it.

mod debian;

use std::path::Path;
use std::process::Command;

use debian::demo::{CONFIG, Holds, LOADED, RESOURCES, Resource, artifact, perturb, prepare};

fn run(program: &str, args: &[&str]) -> (bool, String) {
    let out = Command::new(program)
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

fn must(program: &str, args: &[&str]) -> String {
    let (ok, out) = run(program, args);
    assert!(ok, "{program} {args:?}:\n{out}");
    out
}

/// The installed binary, as its own process: its status and its output.
fn cell(args: &[&str]) -> (i32, String) {
    let out = Command::new("/usr/bin/nomos-cell")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

/// One run of the installed service, as the timer starts it, and its
/// result as systemd records it.
fn run_service() -> String {
    must("systemctl", &["start", "nomos-cell.service"]);
    must(
        "systemctl",
        &[
            "show",
            "-p",
            "Result,ExecMainStatus,ConditionResult",
            "nomos-cell.service",
        ],
    )
}

fn trace_is_satisfied(label: &str) {
    let (status, out) = cell(&[
        "--state",
        "/var/lib/nomos",
        "trace",
        "--canon",
        "/etc/nomos/canon.cbor",
    ]);
    assert_eq!(status, 0, "{label}: trace after the service ran:\n{out}");
    assert!(
        out.contains("7 satisfied, 0 variance, 0 indeterminate; 0 actions planned"),
        "{label}:\n{out}"
    );
    assert_eq!(
        std::fs::read(LOADED).unwrap_or_default(),
        CONFIG,
        "{label}: the service does not run the configuration"
    );
}

#[test]
#[ignore = "installs a package on the host; run by `cargo xtask debian`"]
fn the_package_installs_converges_upgrades_and_purges() {
    prepare();

    // Install: the binary, the state directory, and the timer enabled.
    must("apt-get", &["install", "--yes", "/root/debs/base.deb"]);
    assert!(Path::new("/usr/bin/nomos-cell").exists());
    assert_eq!(
        must("systemctl", &["is-enabled", "nomos-cell.timer"]).trim(),
        "enabled"
    );
    assert_eq!(
        must("systemctl", &["is-active", "nomos-cell.timer"]).trim(),
        "active"
    );
    let (_, mode) = run("stat", &["-c", "%a %U", "/var/lib/nomos"]);
    assert_eq!(mode.trim(), "700 root");

    // With no Canon, the service is skipped, not failed.
    let shown = run_service();
    assert!(shown.contains("ConditionResult=no"), "{shown}");
    assert!(shown.contains("Result=success"), "{shown}");

    // The demonstration Canon, from nothing, through the service.
    let scratch = Path::new("/root/demo");
    std::fs::create_dir_all(scratch).unwrap();
    let (canon, bundle) = artifact(scratch);
    std::fs::copy(&canon, "/etc/nomos/canon.cbor").unwrap();
    for entry in std::fs::read_dir(&bundle).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(
            entry.path(),
            Path::new("/etc/nomos/bundle").join(entry.file_name()),
        )
        .unwrap();
    }
    for r in RESOURCES.iter().rev() {
        perturb(*r, Holds::Absent);
    }
    let shown = run_service();
    assert!(
        shown.contains("Result=success") && shown.contains("ExecMainStatus=0"),
        "the service did not converge the host:\n{shown}\n{}",
        run("journalctl", &["-u", "nomos-cell.service", "--no-pager"]).1
    );
    trace_is_satisfied("from nothing");

    // A foreign change, and the next run restores the Canon.
    perturb(Resource::Conf, Holds::Wrong);
    let shown = run_service();
    assert!(shown.contains("ExecMainStatus=0"), "{shown}");
    trace_is_satisfied("after a foreign change");

    // Upgrade: the timer stays, and so does the Cell's journal.
    must("apt-get", &["install", "--yes", "/root/debs/upgrade.deb"]);
    let version = must("dpkg-query", &["-W", "-f=${Version}", "nomos-cell"]);
    assert!(version.contains("+upgrade"), "{version}");
    assert_eq!(
        must("systemctl", &["is-enabled", "nomos-cell.timer"]).trim(),
        "enabled"
    );
    let (status, events) = cell(&["--state", "/var/lib/nomos", "events"]);
    assert_eq!(status, 0);
    assert!(
        events.contains("plan-accepted cell generation 2"),
        "{events}"
    );
    let shown = run_service();
    assert!(shown.contains("ExecMainStatus=0"), "{shown}");
    trace_is_satisfied("after the upgrade");

    // Remove: the binary and the timer go; the state and the Canon stay.
    must("apt-get", &["remove", "--yes", "nomos-cell"]);
    assert!(!Path::new("/usr/bin/nomos-cell").exists());
    let (enabled, _) = run("systemctl", &["is-enabled", "nomos-cell.timer"]);
    assert!(!enabled, "the timer is still enabled after removal");
    assert!(Path::new("/var/lib/nomos/journal").exists());
    assert!(Path::new("/etc/nomos/canon.cbor").exists());

    // Purge: the Cell's state goes; the operator's Canon stays.
    must("apt-get", &["purge", "--yes", "nomos-cell"]);
    assert!(!Path::new("/var/lib/nomos").exists());
    assert!(Path::new("/etc/nomos/canon.cbor").exists());
    let (installed, _) = run("dpkg", &["--status", "nomos-cell"]);
    assert!(!installed, "the package is still known after purge");
    // What the Cell enforced is not undone by removing it.
    assert_eq!(std::fs::read(LOADED).unwrap_or_default(), CONFIG);
}
