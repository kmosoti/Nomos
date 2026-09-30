//! `cargo xtask debian`: the Linux suites on a disposable Debian host
//! (Phase 1 plan, decision 4).
//!
//! The harness builds the named integration tests as static `musl` binaries,
//! builds the image `tests/fixtures/debian/Dockerfile` for the release, runs
//! a fresh container of it with systemd as PID 1, privileged, on the host's
//! network and cgroup namespace, waits for systemd to finish starting, copies
//! each binary in, runs it as root with its ignored tests included, and
//! removes the container whatever happened. A test binary runs there as
//! `cargo test -- --include-ignored` runs it here: the suites that change
//! the service manager are ignored everywhere but in the container.
//!
//! The record names the release, the image, what the container reports of
//! itself (`/etc/os-release`, the kernel, systemd's state), and each binary's
//! exit status, test counts, and output digest. The command fails when the
//! harness cannot run, when systemd does not reach `running` or `degraded`,
//! or when any test fails: a suite that cannot run reports failure, not
//! success.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::receipt::test_counts;

/// The releases the image recipe supports.
pub(crate) const RELEASES: [&str; 2] = ["12", "13"];

/// The integration tests run on Debian when none are named: every suite
/// that drives the Linux adapter.
pub(crate) const DEFAULT_TESTS: [(&str, &str); 5] = [
    ("nomos-cell", "substrate_conformance"),
    ("nomos-cell", "systemd_units"),
    ("nomos-cell", "debian_host"),
    ("nomos-cell", "cell_commands"),
    ("nomos-cell", "debian_convergence"),
];

/// The arguments every test binary runs with on Debian. The suites that
/// change the service manager are ignored on other hosts, where they must
/// not run, and are run here, where they must.
pub(crate) const TEST_ARGS: [&str; 1] = ["--include-ignored"];

/// What one test binary did on the Debian host.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) package: String,
    pub(crate) test: String,
    pub(crate) exit_status: i32,
    pub(crate) passed: u64,
    pub(crate) failed: u64,
    pub(crate) stdout_sha256: String,
}

/// The record of a run.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Record {
    pub(crate) experiment: &'static str,
    pub(crate) release: String,
    pub(crate) image: String,
    pub(crate) os_release: String,
    pub(crate) kernel: String,
    pub(crate) systemd: String,
    pub(crate) runs: Vec<Run>,
}

impl Record {
    /// Whether every binary ran and passed.
    pub(crate) fn passed(&self) -> bool {
        !self.runs.is_empty()
            && self
                .runs
                .iter()
                .all(|r| r.exit_status == 0 && r.failed == 0)
    }
}

/// The executables of the test targets a `cargo --message-format=json`
/// build reports, by target name.
pub(crate) fn executables(messages: &str) -> Vec<(String, PathBuf)> {
    messages
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["profile"]["test"] == true)
        .filter_map(|m| {
            let name = m["target"]["name"].as_str()?.to_string();
            let exe = m["executable"].as_str()?;
            Some((name, PathBuf::from(exe)))
        })
        .collect()
}

/// The value of `PRETTY_NAME` in an os-release file.
pub(crate) fn pretty_name(os_release: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
}

/// Whether systemd's reported state is one the suites can run in.
pub(crate) fn systemd_ready(state: &str) -> bool {
    matches!(state.trim(), "running" | "degraded")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn run(cmd: &mut Command) -> Result<(i32, String, String), String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn ok(cmd: &mut Command) -> Result<String, String> {
    let (status, stdout, stderr) = run(cmd)?;
    if status == 0 {
        Ok(stdout)
    } else {
        Err(format!("{cmd:?} exited {status}: {}", stderr.trim()))
    }
}

/// The container, removed when dropped.
struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

/// Registries that mirror the official Debian image, tried in order: a
/// shared runner's anonymous quota at any one of them can run out, and the
/// image is the same at each.
pub(crate) const MIRRORS: [&str; 3] = [
    "mirror.gcr.io/library/debian",
    "public.ecr.aws/docker/library/debian",
    "docker.io/library/debian",
];

/// Builds the image for `release` from the recipe, passing the caller's
/// proxy settings as build arguments, which Docker does not keep in the
/// image. Each mirror is tried until one builds.
fn build_image(root: &Path, release: &str) -> Result<String, String> {
    let tag = format!("nomos-debian:{release}");
    let mut failures = Vec::new();
    for mirror in MIRRORS {
        let mut cmd = Command::new("docker");
        cmd.args(["build", "--network", "host", "--build-arg"])
            .arg(format!("BASE={mirror}:{release}"));
        for var in ["http_proxy", "https_proxy", "no_proxy"] {
            let value = std::env::var(var)
                .or_else(|_| std::env::var(var.to_uppercase()))
                .unwrap_or_default();
            if !value.is_empty() {
                cmd.arg("--build-arg").arg(format!("{var}={value}"));
            }
        }
        cmd.args(["-t", &tag])
            .arg(root.join("tests/fixtures/debian"));
        match ok(&mut cmd) {
            Ok(_) => {
                let id =
                    ok(Command::new("docker")
                        .args(["image", "inspect", "--format", "{{.Id}}", &tag]))?;
                return Ok(format!("{mirror}:{release} {tag} {}", id.trim()));
            }
            Err(e) => {
                let last = e.lines().last().unwrap_or_default().to_string();
                eprintln!("{mirror}:{release}: {last}");
                failures.push(format!("{mirror}: {last}"));
            }
        }
    }
    Err(format!(
        "no mirror built the image: {}",
        failures.join("; ")
    ))
}

/// How a run selects and builds its tests.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Selection<'a> {
    /// Only the test of this exact name, in every binary.
    pub(crate) exact: Option<&'a str>,
    /// The Cargo target directory, when not the workspace's own.
    pub(crate) target_dir: Option<&'a Path>,
    /// Keep the test binaries' output to the record, rather than print it:
    /// a caller that runs a test to see it fail, as the mutant runner
    /// does, would otherwise print failures that are its expected result.
    pub(crate) quiet: bool,
}

/// Builds each test as a static binary and returns them with their names.
fn build_tests(
    root: &Path,
    tests: &[(String, String)],
    target_dir: Option<&Path>,
) -> Result<Vec<(String, String, PathBuf)>, String> {
    let mut out = Vec::new();
    for (package, test) in tests {
        let mut cargo = Command::new("cargo");
        if let Some(dir) = target_dir {
            cargo.env("CARGO_TARGET_DIR", dir);
        }
        let messages = ok(cargo
            .current_dir(root)
            .args(["test", "--locked", "--no-run", "--message-format=json"])
            .args([
                "--target",
                "x86_64-unknown-linux-musl",
                "-p",
                package,
                "--test",
                test,
            ]))?;
        let exe = executables(&messages)
            .into_iter()
            .find(|(name, _)| name == test)
            .map(|(_, exe)| exe)
            .ok_or_else(|| format!("{package} --test {test}: no executable was built"))?;
        out.push((package.clone(), test.clone(), exe));
    }
    Ok(out)
}

/// Runs `tests` on a fresh container of `release`.
pub(crate) fn run_suites(
    root: &Path,
    release: &str,
    tests: &[(String, String)],
    selection: Selection<'_>,
) -> Result<Record, String> {
    if !RELEASES.contains(&release) {
        return Err(format!("release {release}: one of {RELEASES:?}"));
    }
    let binaries = build_tests(root, tests, selection.target_dir)?;
    let image = build_image(root, release)?;
    let tag = format!("nomos-debian:{release}");
    let name = format!("nomos-debian-{release}-{}", std::process::id());
    let _ = Command::new("docker").args(["rm", "-f", &name]).output();
    ok(Command::new("docker").args([
        "run",
        "-d",
        "--name",
        &name,
        "--privileged",
        "--cgroupns=host",
        "-v",
        "/sys/fs/cgroup:/sys/fs/cgroup:rw",
        "--tmpfs",
        "/run",
        "--tmpfs",
        "/run/lock",
        "--network",
        "host",
        &tag,
    ]))?;
    let container = Container(name.clone());
    let exec = |args: &[&str]| {
        let mut cmd = Command::new("docker");
        cmd.args(["exec", &container.0]).args(args);
        cmd
    };
    // systemd answers only once D-Bus is up, and `--wait` returns once
    // start-up is complete; until then the command fails or reports a
    // transitional state, so it is asked again, for at most a minute.
    let mut systemd = String::new();
    for _ in 0..60 {
        let (_, state, _) = run(&mut exec(&["systemctl", "is-system-running", "--wait"]))?;
        systemd = state.trim().to_string();
        if systemd_ready(&systemd) || systemd == "maintenance" || systemd == "stopping" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    if !systemd_ready(&systemd) {
        return Err(format!("systemd in the container is {systemd:?}"));
    }
    let os_release = pretty_name(&ok(&mut exec(&["cat", "/etc/os-release"]))?)
        .ok_or("the container has no PRETTY_NAME")?;
    let kernel = ok(&mut exec(&["uname", "-r"]))?.trim().to_string();
    let mut runs = Vec::new();
    for (package, test, exe) in binaries {
        let inside = format!("/usr/local/bin/{test}");
        ok(Command::new("docker")
            .arg("cp")
            .arg(&exe)
            .arg(format!("{}:{inside}", container.0)))?;
        let mut args = vec![inside.as_str()];
        args.extend(TEST_ARGS);
        if let Some(name) = selection.exact {
            args.extend([name, "--exact"]);
        }
        let (status, stdout, stderr) = run(&mut exec(&args))?;
        if !selection.quiet {
            print!("{stdout}");
            eprint!("{stderr}");
        }
        let (passed, failed) = test_counts(&stdout).unwrap_or((0, 0));
        runs.push(Run {
            package,
            test,
            exit_status: status,
            passed,
            failed,
            stdout_sha256: hex(&Sha256::digest(stdout.as_bytes())),
        });
    }
    Ok(Record {
        experiment: "linux-families",
        release: release.to_string(),
        image,
        os_release,
        kernel,
        systemd,
        runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executables_are_read_from_cargo_messages() {
        let messages = concat!(
            r#"{"reason":"compiler-artifact","target":{"name":"nomos_cell"},"profile":{"test":false},"executable":null}"#,
            "\n",
            r#"{"reason":"compiler-artifact","target":{"name":"substrate_conformance"},"profile":{"test":true},"executable":"/t/deps/substrate_conformance-1"}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
            "\nnot json\n"
        );
        assert_eq!(
            executables(messages),
            vec![(
                "substrate_conformance".to_string(),
                PathBuf::from("/t/deps/substrate_conformance-1")
            )]
        );
    }

    #[test]
    fn the_release_and_the_service_manager_are_read_as_reported() {
        assert_eq!(
            pretty_name(
                "NAME=\"Debian GNU/Linux\"\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\n"
            ),
            Some("Debian GNU/Linux 12 (bookworm)".to_string())
        );
        assert_eq!(pretty_name("NAME=x\n"), None);
        assert!(systemd_ready("running\n"));
        assert!(systemd_ready("degraded"));
        assert!(!systemd_ready("starting"));
        assert!(!systemd_ready("offline"));
    }

    /// A run with no binaries, or with a failed test, is not a pass.
    #[test]
    fn a_record_passes_only_when_every_binary_ran_and_passed() {
        let run = |status, failed| Run {
            package: "p".into(),
            test: "t".into(),
            exit_status: status,
            passed: 3,
            failed,
            stdout_sha256: String::new(),
        };
        let record = |runs| Record {
            experiment: "linux-families",
            release: "12".into(),
            image: String::new(),
            os_release: String::new(),
            kernel: String::new(),
            systemd: "running".into(),
            runs,
        };
        assert!(!record(vec![]).passed());
        assert!(record(vec![run(0, 0)]).passed());
        assert!(!record(vec![run(0, 0), run(101, 1)]).passed());
        assert!(!record(vec![run(0, 1)]).passed());
    }
}
