//! Packages through dpkg's database and `apt-get` ([substrate-contract.md],
//! Packages on Linux; ADR 0013 §4). The status database is read directly;
//! `apt-get` is run by a direct `execve` with a fixed argument vector, a
//! fixed environment, and no shell (AGENTS.md rule 5), first simulated so
//! that an operation that would remove a package the requirement does not
//! name is refused before any effect.
//!
//! [substrate-contract.md]: ../../../../docs/formal/substrate-contract.md

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nomos_core::condition::{PackageCondition, PackageVersion};
use nomos_core::observation::PackageEvidence;
use nomos_core::resource::PackageName;

/// The program every package operation runs.
const APT_GET: &str = "/usr/bin/apt-get";
/// The whole environment it runs with, beside `DEBIAN_FRONTEND`.
const PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// The lock files another package manager holds while it changes the
/// database, beneath the root.
pub const LOCKS: [&str; 2] = ["var/lib/dpkg/lock-frontend", "var/lib/dpkg/lock"];
/// dpkg's status database, beneath the root.
pub const STATUS: &str = "var/lib/dpkg/status";

/// dpkg's name for the architecture this adapter was built for.
pub fn native_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "i386",
        "arm" => "armhf",
        "powerpc64" => "ppc64el",
        "s390x" => "s390x",
        "riscv64" => "riscv64",
        other => other,
    }
}

/// One stanza of the status database, by its fields.
fn fields(stanza: &str) -> Vec<(&str, &str)> {
    stanza
        .lines()
        .filter(|l| !l.starts_with(' ') && !l.starts_with('\t'))
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect()
}

/// The evidence the status database `text` gives `name` (Packages on
/// Linux, Observation).
pub fn evidence(text: &str, name: &PackageName, native: &str) -> PackageEvidence {
    let mut best: Option<(u8, PackageEvidence)> = None;
    for stanza in text.split("\n\n") {
        let f = fields(stanza);
        let get = |k: &str| f.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        if get("Package") != Some(name.as_str()) {
            continue;
        }
        let rank = match get("Architecture") {
            Some(a) if a == native => 2,
            Some("all") => 1,
            _ => continue,
        };
        if best.as_ref().is_some_and(|(r, _)| *r >= rank) {
            continue;
        }
        best = Some((rank, of_status(get("Status"), get("Version"))));
    }
    best.map_or(PackageEvidence::NotInstalled, |(_, e)| e)
}

/// The evidence a `Status` field and a `Version` give: `want flag state`.
fn of_status(status: Option<&str>, version: Option<&str>) -> PackageEvidence {
    let words: Vec<&str> = status.unwrap_or_default().split_whitespace().collect();
    let (flag, state) = match words.as_slice() {
        [_, flag, state] => (*flag, *state),
        _ => return PackageEvidence::Broken,
    };
    if flag != "ok" {
        return PackageEvidence::Broken;
    }
    match state {
        "installed" | "triggers-awaited" | "triggers-pending" => {
            match version.and_then(PackageVersion::new) {
                Some(version) => PackageEvidence::Installed { version },
                None => PackageEvidence::Broken,
            }
        }
        "not-installed" | "config-files" => PackageEvidence::NotInstalled,
        _ => PackageEvidence::Broken,
    }
}

/// Whether the kernel's lock table `text` holds a lock on the file with
/// device `(major, minor)` and inode `ino`.
pub fn locked(text: &str, major: u32, minor: u32, ino: u64) -> bool {
    text.lines().any(|line| {
        line.split_whitespace().any(|token| {
            let mut parts = token.split(':');
            let (Some(ma), Some(mi), Some(i), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return false;
            };
            u32::from_str_radix(ma, 16).ok() == Some(major)
                && u32::from_str_radix(mi, 16).ok() == Some(minor)
                && i.parse::<u64>().ok() == Some(ino)
        })
    })
}

/// One run of `apt-get`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The program's absolute path.
    pub program: PathBuf,
    /// Its arguments, in order.
    pub args: Vec<OsString>,
}

impl Invocation {
    fn new(args: &[&str]) -> Self {
        Invocation {
            program: Path::new(APT_GET).to_path_buf(),
            args: args.iter().map(OsString::from).collect(),
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .env_clear()
            .env("PATH", PATH)
            .env("LC_ALL", "C")
            .env("DEBIAN_FRONTEND", "noninteractive")
            .stdin(Stdio::null());
        cmd
    }

    /// Runs it: `true` when it exits with status 0.
    pub fn run(&self) -> bool {
        self.command()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Runs it and returns what it printed, or `None` when it failed.
    pub fn output(&self) -> Option<String> {
        let out = self.command().stderr(Stdio::null()).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// What the adapter does for a requirement (Packages on Linux, Mutation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Already as required.
    Nothing,
    /// Simulate the first, and if the simulation removes nothing else, run
    /// the second.
    Run {
        /// The simulation.
        simulate: Invocation,
        /// The operation.
        run: Invocation,
    },
}

/// The command for `requirement` given the package's evidence `before`.
pub fn plan(name: &PackageName, requirement: &PackageCondition, before: &PackageEvidence) -> Plan {
    const MUTATE: [&str; 6] = [
        "-o",
        "DPkg::Lock::Timeout=60",
        "-o",
        "Dpkg::Options::=--force-confdef",
        "-o",
        "Dpkg::Options::=--force-confold",
    ];
    let (verb, target, extra): (&str, String, &[&str]) = match (requirement, before) {
        (PackageCondition::Absent, PackageEvidence::NotInstalled) => return Plan::Nothing,
        (PackageCondition::Absent, _) => ("remove", name.as_str().to_string(), &[]),
        (PackageCondition::Installed { version: None }, PackageEvidence::Installed { .. }) => {
            return Plan::Nothing;
        }
        (PackageCondition::Installed { version: None }, _) => (
            "install",
            name.as_str().to_string(),
            &["--no-install-recommends"],
        ),
        (
            PackageCondition::Installed { version: Some(v) },
            PackageEvidence::Installed { version },
        ) if v == version => {
            return Plan::Nothing;
        }
        (PackageCondition::Installed { version: Some(v) }, _) => (
            "install",
            format!("{}={}", name.as_str(), v.as_str()),
            &["--no-install-recommends", "--allow-downgrades"],
        ),
    };
    let mut simulate = vec![verb, "--simulate", "--yes"];
    simulate.extend(extra);
    simulate.push(&target);
    let mut run = vec![verb, "--yes"];
    run.extend(extra);
    run.extend(MUTATE);
    run.push(&target);
    Plan::Run {
        simulate: Invocation::new(&simulate),
        run: Invocation::new(&run),
    }
}

/// The packages a simulation's output would remove, by name.
pub fn removals(simulation: &str) -> Vec<String> {
    simulation
        .lines()
        .filter_map(|l| l.strip_prefix("Remv "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|n| n.split(':').next().unwrap_or(n).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> PackageName {
        PackageName::new(s).unwrap()
    }

    fn v(s: &str) -> PackageVersion {
        PackageVersion::new(s).unwrap()
    }

    fn installed(s: &str) -> PackageEvidence {
        PackageEvidence::Installed { version: v(s) }
    }

    /// Stanzas in dpkg's status format, one per state the table of
    /// Packages on Linux names.
    const STATUS: &str = "\
Package: hello
Status: install ok installed
Priority: optional
Architecture: amd64
Version: 2.10-3
Description: example
 a continuation line: with a colon

Package: libc6
Status: install ok installed
Architecture: i386
Version: 2.36-9

Package: libc6
Status: install ok installed
Architecture: amd64
Version: 2.36-9+deb12u4

Package: tzdata
Status: install ok triggers-pending
Architecture: all
Version: 2024a-0+deb12u1

Package: gone
Status: deinstall ok config-files
Architecture: amd64
Version: 1.0-1

Package: half
Status: install ok unpacked
Architecture: amd64
Version: 1.0-1

Package: failing
Status: install reinstreq half-installed
Architecture: amd64
Version: 1.0-1

Package: foreign
Status: install ok installed
Architecture: arm64
Version: 1.0-1
";

    #[test]
    fn the_status_database_reads_as_the_table_says() {
        let e = |name: &str| evidence(STATUS, &n(name), "amd64");
        assert_eq!(e("hello"), installed("2.10-3"));
        assert_eq!(
            e("libc6"),
            installed("2.36-9+deb12u4"),
            "the native entry wins"
        );
        assert_eq!(e("tzdata"), installed("2024a-0+deb12u1"));
        assert_eq!(e("gone"), PackageEvidence::NotInstalled);
        assert_eq!(e("half"), PackageEvidence::Broken);
        assert_eq!(e("failing"), PackageEvidence::Broken);
        assert_eq!(
            e("foreign"),
            PackageEvidence::NotInstalled,
            "another architecture"
        );
        assert_eq!(e("absent"), PackageEvidence::NotInstalled);
        assert_eq!(
            evidence("", &n("hello"), "amd64"),
            PackageEvidence::NotInstalled
        );
    }

    #[test]
    fn a_lock_is_found_by_device_and_inode() {
        let table = "\
1: POSIX  ADVISORY  WRITE 4242 00:2f:1311 0 EOF
2: FLOCK  ADVISORY  WRITE 17 fd:01:77 0 EOF
2: -> FLOCK  ADVISORY  WRITE 18 fd:01:77 0 EOF
";
        assert!(locked(table, 0, 0x2f, 1311));
        assert!(locked(table, 0xfd, 1, 77));
        assert!(!locked(table, 0, 0x2f, 1312));
        assert!(!locked(table, 0, 0x30, 1311));
        assert!(!locked("", 0, 0x2f, 1311));
    }

    fn argv(i: &Invocation) -> Vec<String> {
        let mut out = vec![i.program.display().to_string()];
        out.extend(i.args.iter().map(|a| a.to_string_lossy().into_owned()));
        out
    }

    /// The command table of Packages on Linux, written from the spec.
    #[test]
    fn each_requirement_runs_the_command_the_table_names() {
        let lock = [
            "-o",
            "DPkg::Lock::Timeout=60",
            "-o",
            "Dpkg::Options::=--force-confdef",
            "-o",
            "Dpkg::Options::=--force-confold",
        ];
        let run = |r: PackageCondition, before: PackageEvidence| match plan(&n("web"), &r, &before)
        {
            Plan::Run { simulate, run } => (argv(&simulate), argv(&run)),
            Plan::Nothing => panic!("{r:?} from {before:?} ran nothing"),
        };
        let (sim, cmd) = run(
            PackageCondition::Installed { version: None },
            PackageEvidence::NotInstalled,
        );
        assert_eq!(
            sim,
            [
                "/usr/bin/apt-get",
                "install",
                "--simulate",
                "--yes",
                "--no-install-recommends",
                "web"
            ]
        );
        let mut want = vec![
            "/usr/bin/apt-get",
            "install",
            "--yes",
            "--no-install-recommends",
        ];
        want.extend(lock);
        want.push("web");
        assert_eq!(cmd, want);

        let (_, cmd) = run(
            PackageCondition::Installed {
                version: Some(v("1.0-1")),
            },
            installed("2.0-1"),
        );
        let mut want = vec![
            "/usr/bin/apt-get",
            "install",
            "--yes",
            "--no-install-recommends",
            "--allow-downgrades",
        ];
        want.extend(lock);
        want.push("web=1.0-1");
        assert_eq!(cmd, want);

        let (sim, cmd) = run(PackageCondition::Absent, PackageEvidence::Broken);
        assert_eq!(
            sim,
            ["/usr/bin/apt-get", "remove", "--simulate", "--yes", "web"]
        );
        let mut want = vec!["/usr/bin/apt-get", "remove", "--yes"];
        want.extend(lock);
        want.push("web");
        assert_eq!(cmd, want);

        for (r, before) in [
            (PackageCondition::Absent, PackageEvidence::NotInstalled),
            (
                PackageCondition::Installed { version: None },
                installed("1.0-1"),
            ),
            (
                PackageCondition::Installed {
                    version: Some(v("1.0-1")),
                },
                installed("1.0-1"),
            ),
        ] {
            assert_eq!(plan(&n("web"), &r, &before), Plan::Nothing, "{r:?}");
        }
        assert!(matches!(
            plan(
                &n("web"),
                &PackageCondition::Installed { version: None },
                &PackageEvidence::Broken
            ),
            Plan::Run { .. }
        ));
    }

    #[test]
    fn a_simulation_s_removals_are_read_by_name() {
        let out = "\
NOTE: This is only a simulation!
Reading package lists...
Remv web-extras [1.0-1]
Remv libweb1:amd64 [1.0-1] [web ]
Inst web (2.0-1 local [all])
Conf web (2.0-1 local [all])
";
        assert_eq!(removals(out), ["web-extras", "libweb1"]);
        assert!(removals("Inst web (1.0-1 local [all])\n").is_empty());
    }
}
