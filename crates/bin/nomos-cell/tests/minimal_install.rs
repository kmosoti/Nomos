//! The Cell on a minimal Debian host (alpha.2 issue #35): a base image and
//! the Cell's package, and nothing else. Every package on the host besides
//! the base is one the Cell's `Depends:` pulled in.
//!
//! The image is built by `cargo xtask debian-minimal`, which installs the
//! package with `apt-get install --no-install-recommends` and starts
//! systemd as PID 1, and copies this test binary in. The test then checks,
//! before it installs anything of its own, that everything the Cell runs
//! is present, and converges the demonstration Canon of `15`, which uses
//! all six resource families, through the installed service.
//!
//! The test builds its own local package repository, which needs only
//! `dpkg-deb` and `apt-get`, both in the base image. It changes the host, so
//! it is ignored on an ordinary run.

mod debian;

use std::path::Path;
use std::process::Command;

use debian::demo::{CONFIG, Holds, LOADED, RESOURCES, artifact, perturb, prepare};

/// Each program the Cell runs or relies on, and the package that owns it on
/// Debian 12 and 13. The package metadata must account for every one.
const RUNTIME: [(&str, &str); 6] = [
    ("/usr/bin/apt-get", "apt"),
    ("/usr/sbin/useradd", "passwd"),
    ("/usr/sbin/usermod", "passwd"),
    ("/usr/sbin/userdel", "passwd"),
    ("/usr/bin/systemctl", "systemd"),
    ("/usr/bin/dbus-daemon", "dbus"),
];

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

#[test]
#[ignore = "inspects and changes the host; run by `cargo xtask debian-minimal`"]
fn the_package_alone_brings_everything_the_cell_runs() {
    // The host is what the package made it: nothing else was installed.
    for (program, package) in RUNTIME {
        assert!(
            Path::new(program).exists(),
            "{program} is absent, and {package} owns it"
        );
        let status = must("dpkg-query", &["-W", "-f=${Status}", package]);
        assert_eq!(status.trim(), "install ok installed", "{package}");
    }
    // The package declares each of them, and nothing was added by hand.
    let depends = must("dpkg-query", &["-W", "-f=${Depends}", "nomos-cell"]);
    for (program, package) in RUNTIME {
        assert!(
            depends
                .split(',')
                .any(|d| d.trim().split(' ').next() == Some(package)),
            "nomos-cell does not declare {package}, which owns {program}: {depends}"
        );
    }
    // D-Bus answers: the system bus is a dependency of the Cell's units.
    assert!(
        Path::new("/run/dbus/system_bus_socket").exists(),
        "no system bus: a Cell without it reports every unit Indeterminate"
    );
    // `is-system-running` exits 1 when the state is `degraded`, which a
    // container's idle units can cause and the Cell does not mind.
    let (_, state) = run("systemctl", &["is-system-running", "--wait"]);
    assert!(
        matches!(state.trim(), "running" | "degraded"),
        "systemd is {state}"
    );

    // All six families, from nothing, through the installed service.
    prepare();
    let scratch = Path::new("/root/demo");
    std::fs::create_dir_all(scratch).unwrap();
    let (canon, bundle) = artifact(scratch);
    std::fs::create_dir_all("/etc/nomos/bundle").unwrap();
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
    must("systemctl", &["start", "nomos-cell.service"]);
    let shown = must(
        "systemctl",
        &["show", "-p", "Result,ExecMainStatus", "nomos-cell.service"],
    );
    assert!(
        shown.contains("Result=success") && shown.contains("ExecMainStatus=0"),
        "the service did not converge the host:\n{shown}\n{}",
        run("journalctl", &["-u", "nomos-cell.service", "--no-pager"]).1
    );
    let out = Command::new("/usr/bin/nomos-cell")
        .args([
            "--state",
            "/var/lib/nomos",
            "trace",
            "--canon",
            "/etc/nomos/canon.cbor",
        ])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{said}");
    assert!(
        said.contains("7 satisfied, 0 variance, 0 indeterminate; 0 actions planned"),
        "{said}"
    );
    assert_eq!(std::fs::read(LOADED).unwrap_or_default(), CONFIG);
}
