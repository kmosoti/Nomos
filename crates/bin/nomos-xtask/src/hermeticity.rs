//! Experiment `build-hermeticity` (grounding plan, `06-canon-artifact`;
//! ADR 0004 §3): do two isolated builds of one Canon authoring crate, with
//! the same declared inputs, produce the same IR, and is every undeclared
//! input denied or recorded?
//!
//! The harness copies the declared inputs into two scratch roots whose
//! paths differ, then, for each root, builds `tests/fixtures/canon/authoring`
//! and runs its generators as the build job would:
//!
//! - **Network: denied.** Every build and run is inside `unshare -rn`, a
//!   network namespace with no route; attempts are recorded by `strace`.
//! - **Environment: denied.** `env -i` passes only the declared variables
//!   (`PATH`, `HOME`, `CARGO_HOME`, `TMPDIR`). The harness's own locale,
//!   time zone, and a noise variable differ between the two roots and must
//!   not reach the IR; the leaky generator shows whether they did.
//! - **Files: recorded.** `strace -ff -e trace=%file,%network,getrandom`
//!   records every path the job touches, each classified by where it is.
//! - **Randomness: recorded,** as `getrandom` calls.
//! - **Clock: recorded,** by a second run of each generator under Valgrind
//!   with syscall tracing, which hides the vDSO so that every clock read is
//!   a system call. A `strace` alone cannot see it.
//! - **Working directory, temporary directory, home, source path, target
//!   directory, and time of run: varied.** Each differs between the roots.
//!
//! `telemetry` is the generator under test and must produce identical IR in
//! both roots, equal to the committed golden fixtures, touching no
//! undeclared path, network, or clock. `leaky` is the negative control: it
//! reads each input above, and the harness must see every one. A control
//! that does not fire fails the run.
//!
//! `leaky` runs no code at build time, so it cannot trip
//! `build-executes-only-the-toolchain`. A second control can:
//! [`Subject::BuildScript`] builds `tests/fixtures/canon/authoring-build-script`,
//! a crate with a build script and no generators, through the same job and
//! only the build checks. It must fail `build-executes-only-the-toolchain` and
//! nothing else; the record names the checks that ran, those that did not
//! apply, and the failure the control expects.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The crate built, relative to the workspace root.
const CRATE: &str = "tests/fixtures/canon/authoring";
/// The declared inputs: the authoring crate, the Canon crates it depends
/// on, and the manifests they inherit from.
const DECLARED: &[&str] = &[
    CRATE,
    "crates/core/nomos-core",
    "crates/core/nomos-canon",
    "Cargo.toml",
    "rust-toolchain.toml",
];
/// The build-script control, relative to the workspace root.
const BUILD_SCRIPT_CRATE: &str = "tests/fixtures/canon/authoring-build-script";
/// Its declared inputs: the crate and the toolchain file. It depends on
/// nothing, so nothing else is an input.
const BUILD_SCRIPT_DECLARED: &[&str] = &[BUILD_SCRIPT_CRATE, "rust-toolchain.toml"];
const GENERATORS: &[&str] = &["telemetry", "leaky"];
const PROFILES: &[(&str, &str)] = &[("cbor", "cbor"), ("jcs", "json")];

/// The checks on the build job alone, which every subject runs.
const BUILD_CHECKS: &[&str] = &[
    "build-network-attempts-all-denied",
    "build-executes-only-the-toolchain",
    "build-opens-no-undeclared-file",
];
/// The checks on the generators, which only the authoring crate has.
const GENERATOR_CHECKS: &[&str] = &[
    "telemetry-cbor-identical-across-roots",
    "telemetry-cbor-is-golden",
    "telemetry-jcs-identical-across-roots",
    "telemetry-jcs-is-golden",
    "telemetry-touches-no-undeclared-path",
    "telemetry-makes-no-network-call",
    "telemetry-reads-no-clock",
    "telemetry-reads-no-working-directory",
    "control-output-differs",
    "control-undeclared-file-recorded",
    "control-network-attempt-recorded-and-denied",
    "control-clock-recorded",
    "control-randomness-recorded",
    "control-working-directory-recorded",
    "control-ambient-environment-denied",
];

/// What the harness builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    /// The Canon authoring crate and its generators, under every check: the
    /// experiment itself.
    Authoring,
    /// The build-script control: builds only, under the build checks, and
    /// must fail `build-executes-only-the-toolchain` alone.
    BuildScript,
}

impl Subject {
    /// The subject a `--control` option names; none is the experiment.
    pub fn from_control(control: Option<&str>) -> Result<Subject, String> {
        match control {
            None => Ok(Subject::Authoring),
            Some("build-script") => Ok(Subject::BuildScript),
            Some(other) => Err(format!(
                "unknown control {other:?}; the one control is build-script"
            )),
        }
    }

    /// The control's name, or none for the experiment.
    fn control(self) -> Option<&'static str> {
        match self {
            Subject::Authoring => None,
            Subject::BuildScript => Some("build-script"),
        }
    }

    fn krate(self) -> &'static str {
        match self {
            Subject::Authoring => CRATE,
            Subject::BuildScript => BUILD_SCRIPT_CRATE,
        }
    }

    fn declared(self) -> &'static [&'static str] {
        match self {
            Subject::Authoring => DECLARED,
            Subject::BuildScript => BUILD_SCRIPT_DECLARED,
        }
    }

    fn generators(self) -> &'static [&'static str] {
        match self {
            Subject::Authoring => GENERATORS,
            Subject::BuildScript => &[],
        }
    }

    /// The checks this subject runs, and those that do not apply to it.
    fn checks(self) -> (Vec<&'static str>, Vec<&'static str>) {
        match self {
            Subject::Authoring => (
                BUILD_CHECKS
                    .iter()
                    .chain(GENERATOR_CHECKS)
                    .copied()
                    .collect(),
                Vec::new(),
            ),
            Subject::BuildScript => (BUILD_CHECKS.to_vec(), GENERATOR_CHECKS.to_vec()),
        }
    }

    /// The checks a control exists to fail; none for the experiment.
    fn expected_failures(self) -> &'static [&'static str] {
        match self {
            Subject::Authoring => &[],
            Subject::BuildScript => &["build-executes-only-the-toolchain"],
        }
    }
}

/// One isolated root and what differs about it.
struct Variant {
    name: &'static str,
    /// Directory name, of a different length in each, so that any path
    /// that reaches the output changes it.
    dir: &'static str,
    /// Ambient variables the job must not see.
    ambient: &'static [(&'static str, &'static str)],
}

const VARIANTS: [Variant; 2] = [
    Variant {
        name: "a",
        dir: "a",
        ambient: &[
            ("TZ", "UTC"),
            ("LC_ALL", "C"),
            ("LANG", "C"),
            ("NOMOS_NOISE", "a"),
        ],
    },
    Variant {
        name: "b",
        dir: "build-root-b",
        ambient: &[
            ("TZ", "Pacific/Chatham"),
            ("LC_ALL", "C.UTF-8"),
            ("LANG", "en_US.UTF-8"),
            ("NOMOS_NOISE", "bbbbbbbb"),
        ],
    },
];

/// What `strace` saw one job do.
#[derive(Debug, Default, Serialize)]
pub struct Access {
    /// Successful path accesses, by class.
    classes: BTreeMap<String, usize>,
    /// Undeclared paths touched, each with the system calls made on it and
    /// whether each succeeded (`openat=ok`, `newfstatat=err`).
    undeclared: BTreeMap<String, BTreeSet<String>>,
    /// Attempts to reach a network: `socket`, `connect`, `bind`, and
    /// `sendto` on an Internet address family, as written by `strace`.
    network: Vec<String>,
    /// Other socket calls: local sockets and pipes.
    local_ipc: usize,
    /// Programs executed, by class, then path.
    executed: BTreeMap<String, BTreeSet<String>>,
    /// `getrandom` calls.
    getrandom: usize,
    /// `getcwd` calls: the working directory read.
    getcwd: usize,
    /// Clock system calls seen under Valgrind; generators only.
    clock: Option<usize>,
}

#[derive(Debug, Serialize)]
struct GeneratorRecord {
    /// SHA-256 of the IR, by profile, then root.
    ir: BTreeMap<String, BTreeMap<String, String>>,
    /// Whether the two roots' IR is identical, by profile.
    identical: BTreeMap<String, bool>,
    /// Whether the IR is the committed golden fixture, by profile; for the
    /// generator under test only.
    golden: BTreeMap<String, bool>,
    /// Whether the two roots' binaries are byte-identical.
    binary_identical: bool,
    /// Access by root and profile.
    runs: BTreeMap<String, Access>,
}

/// The provenance ADR 0004 §3 asks the job to write beside the IR.
#[derive(Debug, Serialize)]
struct Provenance {
    toolchain: String,
    lockfile_sha256: String,
    /// Absent for the build-script control, which does not depend on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    nomos_canon_version: Option<String>,
    declared_files: usize,
    declared_sha256: String,
    ir_sha256: BTreeMap<String, String>,
}

/// What a control run was, and what it was for. Absent from the
/// experiment's own record.
#[derive(Debug, Serialize)]
struct ControlRun {
    /// The control's name, as `--control` takes it.
    name: &'static str,
    /// The crate built, relative to the workspace root.
    #[serde(rename = "crate")]
    krate: &'static str,
    /// The checks that ran; each is in `checks`.
    checks_run: Vec<&'static str>,
    /// The checks that do not apply to this crate, and did not run.
    checks_not_applicable: Vec<&'static str>,
    /// The checks the control exists to fail.
    expected_failures: Vec<&'static str>,
}

/// The experiment's record.
#[derive(Debug, Serialize)]
pub struct Record {
    experiment: &'static str,
    /// Present only for a control run.
    #[serde(skip_serializing_if = "Option::is_none")]
    control: Option<ControlRun>,
    provenance: Provenance,
    build: BTreeMap<String, Access>,
    generators: BTreeMap<String, GeneratorRecord>,
    /// Each check of the decision rule and the negative control, with its
    /// outcome. The run fails if any is false.
    checks: BTreeMap<String, bool>,
}

impl Record {
    /// The checks that failed.
    pub fn failures(&self) -> Vec<&str> {
        self.checks
            .iter()
            .filter(|(_, ok)| !**ok)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// For a control run, whether it failed exactly the checks it exists to
    /// fail; none for the experiment.
    pub fn control_fired(&self) -> Option<bool> {
        let control = self.control.as_ref()?;
        Some(fired_exactly(&self.failures(), &control.expected_failures))
    }
}

/// Whether `failures` is exactly `expected`, which is not empty.
fn fired_exactly(failures: &[&str], expected: &[&str]) -> bool {
    let failed: BTreeSet<&str> = failures.iter().copied().collect();
    let wanted: BTreeSet<&str> = expected.iter().copied().collect();
    !wanted.is_empty() && failed == wanted
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn output(cmd: &mut Command) -> Result<Vec<u8>, String> {
    let out = cmd
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?}: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

fn tool(name: &str) -> Result<PathBuf, String> {
    for dir in ["/usr/bin", "/bin", "/usr/local/bin"] {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(format!(
        "{name} not found: build-hermeticity cannot observe the job without it"
    ))
}

/// Copies the declared inputs, as git lists them (tracked and untracked,
/// not ignored), under `to`. Returns their digests by relative path.
fn copy_declared(
    root: &Path,
    to: &Path,
    declared: &[&str],
) -> Result<BTreeMap<String, String>, String> {
    let listed = output(
        Command::new(tool("git")?)
            .current_dir(root)
            .args(["ls-files", "-co", "--exclude-standard", "--"])
            .args(declared),
    )?;
    let mut digests = BTreeMap::new();
    for rel in String::from_utf8_lossy(&listed).lines() {
        let from = root.join(rel);
        if !from.is_file() {
            continue;
        }
        let bytes = fs::read(&from).map_err(|e| format!("{}: {e}", from.display()))?;
        let dest = to.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&dest, &bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
        digests.insert(rel.to_string(), sha(&bytes));
    }
    Ok(digests)
}

/// Where a path is, relative to the job's declared layout.
struct Layout {
    source: PathBuf,
    target: PathBuf,
    job: PathBuf,
    sysroot: PathBuf,
}

const SYSTEM: &[&str] = &[
    "/etc/ld.so.cache",
    "/etc/ld.so.preload",
    "/lib/",
    "/lib64/",
    "/usr/lib/",
    "/usr/lib64/",
    "/usr/libexec/",
    "/usr/bin/",
    "/bin/",
    "/etc/alternatives/",
];
const KERNEL: &[&str] = &["/proc/", "/sys/", "/dev/"];

impl Layout {
    fn class(&self, path: &str) -> &'static str {
        let p = Path::new(path);
        if path.is_empty() {
            "descriptor"
        } else if !p.is_absolute() {
            "relative"
        } else if p.starts_with(&self.source) {
            "declared-source"
        } else if p.starts_with(&self.target) {
            "build-output"
        } else if p.starts_with(&self.job) {
            "job-directory"
        } else if p.starts_with(&self.sysroot) {
            "toolchain"
        } else if SYSTEM
            .iter()
            .any(|s| path.starts_with(s) || path == s.trim_end_matches('/'))
        {
            "system"
        } else if KERNEL.iter().any(|s| path.starts_with(s)) {
            "kernel-interface"
        } else {
            "undeclared"
        }
    }

    fn mask(&self, text: &str) -> String {
        text.replace(&self.job.display().to_string(), "$JOB")
            .replace(&self.sysroot.display().to_string(), "$SYSROOT")
    }
}

/// `path` with `.` and `..` resolved lexically, so that a path that climbs
/// out of a directory is not classified as inside it.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn first_quoted(line: &str) -> Option<&str> {
    let start = line.find('"')? + 1;
    let end = start + line[start..].find('"')?;
    Some(&line[start..end])
}

const NETWORK: &[&str] = &[
    "socket", "connect", "bind", "listen", "accept", "accept4", "sendto", "sendmsg", "recvfrom",
    "recvmsg",
];

/// Reads every `strace -ff` file with prefix `prefix`.
fn read_strace(prefix: &Path, layout: &Layout, env: &Path) -> Result<Access, String> {
    let dir = prefix.parent().ok_or("strace prefix has no directory")?;
    let stem = prefix
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("strace prefix is not text")?;
    let mut access = Access::default();
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n.starts_with(&format!("{stem}.")))
        })
        .collect();
    files.sort();
    for file in files {
        let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        // The harness's own `env` runs first in the job's process and sets
        // up its locale from the ambient environment before it clears it;
        // its accesses end at the next successful `execve` and are not the
        // job's.
        let mut in_env = false;
        // Relative paths resolve against the process's working directory:
        // the job's, unless the process changed it. A process inherits the
        // job's directory because nothing in the job changes its own before
        // it forks; a path relative to a directory descriptor is inside a
        // directory whose own opening was classified, and is not resolved.
        let mut cwd = layout.job.join("work");
        for line in text.lines() {
            let Some(open) = line.find('(') else { continue };
            let name = &line[..open];
            let ok = line
                .rsplit_once(" = ")
                .is_some_and(|(_, ret)| !ret.starts_with("-1"));
            if name == "execve" && ok {
                in_env = first_quoted(line).is_some_and(|p| Path::new(p) == env);
                if in_env {
                    continue;
                }
                if let Some(program) = first_quoted(line) {
                    access
                        .executed
                        .entry(layout.class(program).to_string())
                        .or_default()
                        .insert(layout.mask(program));
                }
            }
            if in_env {
                continue;
            }
            match name {
                "getrandom" => access.getrandom += 1,
                "getcwd" => access.getcwd += 1,
                n if NETWORK.contains(&n) => {
                    let internet = line.contains("AF_INET") || line.contains("AF_INET6");
                    if internet {
                        access.network.push(layout.mask(line));
                    } else {
                        access.local_ipc += 1;
                    }
                }
                _ => {
                    let Some(quoted) = first_quoted(line) else {
                        continue;
                    };
                    if name == "chdir" && ok {
                        cwd = normalize(&cwd.join(quoted));
                    }
                    let at_cwd = !line[open + 1..].starts_with(|c: char| c.is_ascii_digit());
                    let resolved;
                    let path = if quoted.is_empty() {
                        quoted
                    } else if Path::new(quoted).is_absolute() {
                        resolved = normalize(Path::new(quoted)).display().to_string();
                        resolved.as_str()
                    } else if at_cwd {
                        resolved = normalize(&cwd.join(quoted)).display().to_string();
                        resolved.as_str()
                    } else {
                        if ok {
                            *access
                                .classes
                                .entry("descriptor-relative".to_string())
                                .or_default() += 1;
                        }
                        continue;
                    };
                    let class = layout.class(path);
                    if class == "undeclared" || class == "relative" {
                        access
                            .undeclared
                            .entry(layout.mask(path))
                            .or_default()
                            .insert(format!("{name}={}", if ok { "ok" } else { "err" }));
                    }
                    if ok {
                        *access.classes.entry(class.to_string()).or_default() += 1;
                    }
                }
            }
        }
    }
    Ok(access)
}

/// Clock system calls in a Valgrind syscall trace.
fn clock_calls(log: &Path) -> Result<usize, String> {
    let text = fs::read_to_string(log).map_err(|e| format!("{}: {e}", log.display()))?;
    Ok(text
        .lines()
        .filter(|l| {
            [
                "sys_clock_gettime",
                "sys_gettimeofday",
                "sys_time(",
                "sys_time ",
            ]
            .iter()
            .any(|s| l.contains(s))
        })
        .count())
}

struct Job {
    unshare: PathBuf,
    strace: PathBuf,
    env: PathBuf,
    valgrind: PathBuf,
    sysroot: PathBuf,
}

impl Job {
    /// The command the job runs, isolated and observed: `unshare -rn`, then
    /// `strace`, then `env -i` with the declared variables only.
    fn command(&self, variant: &Variant, root: &Path, trace: &Path, argv: &[&str]) -> Command {
        let mut cmd = Command::new(&self.unshare);
        cmd.env_clear();
        for (k, v) in variant.ambient {
            cmd.env(k, v);
        }
        cmd.current_dir(root.join("work"))
            .args(["-rn"])
            .arg(&self.strace)
            .args([
                "-ff",
                "-qq",
                "-e",
                "trace=%file,%network,getrandom,getcwd",
                "-o",
            ])
            .arg(trace)
            .arg(&self.env)
            .arg("-i")
            .args(self.declared_env(root))
            .args(argv);
        cmd
    }

    fn declared_env(&self, root: &Path) -> Vec<String> {
        vec![
            format!("PATH={}/bin:/usr/bin:/bin", self.sysroot.display()),
            format!("HOME={}", root.join("home").display()),
            format!("CARGO_HOME={}", root.join("cargo-home").display()),
            format!("TMPDIR={}", root.join("tmp").display()),
        ]
    }
}

/// Runs the experiment, or the control `subject` names, with scratch space
/// under `scratch`, which it empties first.
pub fn run(root: &Path, scratch: &Path, subject: Subject) -> Result<Record, String> {
    let rustc = output(
        Command::new("rustc")
            .current_dir(root)
            .args(["--print", "sysroot"]),
    )?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&rustc).trim());
    let toolchain = String::from_utf8_lossy(&output(
        Command::new(sysroot.join("bin/rustc")).arg("--version"),
    )?)
    .trim()
    .to_string();
    let job = Job {
        unshare: tool("unshare")?,
        strace: tool("strace")?,
        env: tool("env")?,
        valgrind: tool("valgrind")?,
        sysroot: sysroot.clone(),
    };
    if scratch.exists() {
        fs::remove_dir_all(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    }

    let mut declared = None;
    let mut build = BTreeMap::new();
    let mut ir: BTreeMap<(&str, &str, &str), Vec<u8>> = BTreeMap::new();
    let mut binaries: BTreeMap<(&str, &str), Vec<u8>> = BTreeMap::new();
    let mut runs: BTreeMap<(&str, String), Access> = BTreeMap::new();
    for (i, variant) in VARIANTS.iter().enumerate() {
        if i > 0 {
            // A different time of run.
            std::thread::sleep(Duration::from_millis(1100));
        }
        let base = scratch.join(variant.dir);
        let source = base.join("src");
        let digests = copy_declared(root, &source, subject.declared())?;
        match &declared {
            None => declared = Some(digests),
            Some(first) if *first == digests => {}
            Some(_) => return Err("the two copies of the declared inputs differ".into()),
        }
        for d in ["work", "home", "cargo-home", "tmp", "trace"] {
            fs::create_dir_all(base.join(d)).map_err(|e| format!("{d}: {e}"))?;
        }
        let target = base.join("target");
        let layout = Layout {
            source: source.clone(),
            target: target.clone(),
            job: base.clone(),
            sysroot: sysroot.clone(),
        };
        let manifest = source.join(subject.krate()).join("Cargo.toml");
        let cargo = sysroot.join("bin/cargo");
        let trace = base.join("trace/build");
        let argv = [
            cargo.to_str().ok_or("cargo path is not text")?,
            "build",
            "--release",
            "--offline",
            "--locked",
            "--manifest-path",
            manifest.to_str().ok_or("manifest path is not text")?,
            "--target-dir",
            target.to_str().ok_or("target path is not text")?,
        ];
        output(&mut job.command(variant, &base, &trace, &argv))?;
        build.insert(
            variant.name.to_string(),
            read_strace(&trace, &layout, &job.env)?,
        );

        for generator in subject.generators() {
            let bin = target.join("release").join(generator);
            let bytes = fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
            binaries.insert((generator, variant.name), bytes);
            let bin_text = bin.to_str().ok_or("binary path is not text")?;
            for (profile, _) in PROFILES {
                let trace = base.join(format!("trace/{generator}-{profile}"));
                let out = output(&mut job.command(variant, &base, &trace, &[bin_text, profile]))?;
                ir.insert((generator, profile, variant.name), out);
                let mut access = read_strace(&trace, &layout, &job.env)?;
                let log = base.join(format!("trace/{generator}-{profile}.valgrind"));
                let mut vg = Command::new(&job.unshare);
                vg.env_clear()
                    .current_dir(base.join("work"))
                    .arg("-rn")
                    .arg(&job.env)
                    .arg("-i")
                    .args(job.declared_env(&base))
                    .arg(&job.valgrind)
                    .args(["--tool=none", "--trace-syscalls=yes"])
                    .arg(format!("--log-file={}", log.display()))
                    .args([bin_text, profile]);
                output(vg.stdout(Stdio::null()))?;
                access.clock = Some(clock_calls(&log)?);
                runs.insert((generator, format!("{}-{profile}", variant.name)), access);
            }
        }
    }

    let declared = declared.ok_or("no variant ran")?;
    let mut listing = String::new();
    for (path, digest) in &declared {
        listing.push_str(&format!("{path}\0{digest}\n"));
    }
    let lock = declared
        .get(&format!("{}/Cargo.lock", subject.krate()))
        .cloned()
        .ok_or("the built crate's Cargo.lock is not a declared input")?;
    let canon_manifest = fs::read_to_string(root.join("crates/core/nomos-canon/Cargo.toml"))
        .map_err(|e| e.to_string())?;
    let workspace_manifest =
        fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let version = if canon_manifest.contains("version.workspace = true") {
        workspace_manifest
            .lines()
            .find_map(|l| l.strip_prefix("version = "))
            .map(|v| v.trim_matches('"').to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let mut checks = build_checks(&build);

    let (checks_run, checks_not_applicable) = subject.checks();
    let declared_files = declared.len();
    let declared_sha256 = sha(listing.as_bytes());
    let provenance = |nomos_canon_version, ir_sha256| Provenance {
        toolchain,
        lockfile_sha256: lock,
        nomos_canon_version,
        declared_files,
        declared_sha256,
        ir_sha256,
    };
    if let Some(name) = subject.control() {
        check_names(&checks, &checks_run)?;
        return Ok(Record {
            experiment: "build-hermeticity",
            control: Some(ControlRun {
                name,
                krate: subject.krate(),
                checks_run,
                checks_not_applicable,
                expected_failures: subject.expected_failures().to_vec(),
            }),
            provenance: provenance(None, BTreeMap::new()),
            build,
            generators: BTreeMap::new(),
            checks,
        });
    }

    let mut generators = BTreeMap::new();
    let golden = root.join("tests/fixtures/canon/golden");
    for generator in GENERATORS {
        let mut rec = GeneratorRecord {
            ir: BTreeMap::new(),
            identical: BTreeMap::new(),
            golden: BTreeMap::new(),
            binary_identical: binaries.get(&(*generator, "a")) == binaries.get(&(*generator, "b")),
            runs: BTreeMap::new(),
        };
        for (profile, ext) in PROFILES {
            let a = &ir[&(*generator, *profile, "a")];
            let b = &ir[&(*generator, *profile, "b")];
            rec.ir.insert(
                profile.to_string(),
                BTreeMap::from([("a".to_string(), sha(a)), ("b".to_string(), sha(b))]),
            );
            rec.identical.insert(profile.to_string(), a == b);
            if *generator == "telemetry" {
                let committed = fs::read(golden.join(format!("telemetry.v3.{ext}")))
                    .map_err(|e| format!("golden telemetry.v3.{ext}: {e}"))?;
                rec.golden.insert(profile.to_string(), *a == committed);
            }
        }
        for ((g, key), access) in std::mem::take(&mut runs) {
            if g == *generator {
                rec.runs.insert(key, access);
            } else {
                runs.insert((g, key), access);
            }
        }
        generators.insert(generator.to_string(), rec);
    }

    // The decision rule, for the generator under test.
    let t = &generators["telemetry"];
    for (profile, _) in PROFILES {
        checks.insert(
            format!("telemetry-{profile}-identical-across-roots"),
            t.identical[*profile],
        );
        checks.insert(format!("telemetry-{profile}-is-golden"), t.golden[*profile]);
    }
    checks.insert(
        "telemetry-touches-no-undeclared-path".into(),
        t.runs.values().all(|a| a.undeclared.is_empty()),
    );
    checks.insert(
        "telemetry-makes-no-network-call".into(),
        t.runs.values().all(|a| a.network.is_empty()),
    );
    checks.insert(
        "telemetry-reads-no-clock".into(),
        t.runs.values().all(|a| a.clock == Some(0)),
    );
    checks.insert(
        "telemetry-reads-no-working-directory".into(),
        t.runs.values().all(|a| a.getcwd == 0),
    );
    // The negative control: each undeclared input it reads is seen.
    let l = &generators["leaky"];
    let t_random = t.runs.values().map(|a| a.getrandom).max().unwrap_or(0);
    checks.insert(
        "control-output-differs".into(),
        PROFILES.iter().all(|(p, _)| !l.identical[*p]),
    );
    checks.insert(
        "control-undeclared-file-recorded".into(),
        l.runs.values().all(|a| {
            a.undeclared
                .get("/etc/hostname")
                .is_some_and(|calls| calls.contains("openat=ok"))
        }),
    );
    checks.insert(
        "control-network-attempt-recorded-and-denied".into(),
        l.runs.values().all(|a| {
            a.network
                .iter()
                .any(|n| n.starts_with("connect(") && n.contains("= -1"))
        }),
    );
    checks.insert(
        "control-clock-recorded".into(),
        l.runs.values().all(|a| a.clock.is_some_and(|c| c > 0)),
    );
    checks.insert(
        "control-randomness-recorded".into(),
        l.runs.values().all(|a| a.getrandom > t_random),
    );
    checks.insert(
        "control-working-directory-recorded".into(),
        l.runs.values().all(|a| a.getcwd > 0),
    );
    // The ambient locale and time zone differ between the roots; `env -i`
    // must keep both from the generator, which writes what it saw.
    let ambient_denied = PROFILES.iter().all(|(p, _)| {
        ["a", "b"].iter().all(|v| {
            let text = String::from_utf8_lossy(&ir[&("leaky", *p, *v)]).to_string();
            text.contains("locale:none") && text.contains("tz:none")
        })
    });
    checks.insert("control-ambient-environment-denied".into(), ambient_denied);

    check_names(&checks, &checks_run)?;
    let ir_sha256 = PROFILES
        .iter()
        .map(|(p, _)| (p.to_string(), sha(&ir[&("telemetry", *p, "a")])))
        .collect();
    Ok(Record {
        experiment: "build-hermeticity",
        control: None,
        provenance: provenance(Some(version), ir_sha256),
        build,
        generators,
        checks,
    })
}

/// The checks on the build job, by root; every subject runs these.
fn build_checks(build: &BTreeMap<String, Access>) -> BTreeMap<String, bool> {
    let mut checks = BTreeMap::new();
    checks.insert(
        "build-network-attempts-all-denied".into(),
        build.values().all(|a| {
            a.network
                .iter()
                .all(|l| l.starts_with("socket(") || l.contains("= -1"))
        }),
    );
    // Only the pinned toolchain and the system linker are executed at build
    // time: no build script, which Cargo runs as its own process out of the
    // target directory. A procedural macro is not executed: `rustc` loads it
    // as a shared library, which this check does not see; that the authoring
    // crate has none rests on its lockfile, not on this check.
    checks.insert(
        "build-executes-only-the-toolchain".into(),
        build
            .values()
            .all(|a| a.executed.keys().all(|c| c == "toolchain" || c == "system")),
    );
    checks.insert(
        "build-opens-no-undeclared-file".into(),
        build.values().all(|a| {
            a.undeclared
                .values()
                .all(|calls| !calls.iter().any(|c| c == "openat=ok" || c == "execve=ok"))
        }),
    );
    checks
}

/// The checks computed are exactly the checks the subject declares it runs,
/// so that the record's `checks_run` and `checks_not_applicable` cannot
/// drift from the code that computes them.
fn check_names(checks: &BTreeMap<String, bool>, run: &[&str]) -> Result<(), String> {
    let computed: BTreeSet<&str> = checks.keys().map(String::as_str).collect();
    let declared: BTreeSet<&str> = run.iter().copied().collect();
    if computed == declared {
        Ok(())
    } else {
        Err(format!(
            "harness error: the checks computed, {computed:?}, are not the checks declared, {declared:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(job: &Path) -> Layout {
        Layout {
            source: job.join("src"),
            target: job.join("target"),
            job: job.to_path_buf(),
            sysroot: PathBuf::from("/toolchain"),
        }
    }

    /// A synthetic `strace -ff` trace, one file per process, read back.
    fn trace(name: &str, files: &[(&str, &str)]) -> Access {
        let dir = std::env::temp_dir().join(format!(
            "nomos-hermeticity-parse-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (pid, text) in files {
            fs::write(dir.join(format!("t.{pid}")), text).unwrap();
        }
        let job = PathBuf::from("/j");
        let access = read_strace(&dir.join("t"), &layout(&job), Path::new("/usr/bin/env")).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        access
    }

    /// The harness's own `env` is not the job: its accesses before the job's
    /// `execve` are skipped, and everything after is counted.
    #[test]
    fn the_harness_env_is_not_charged_to_the_job() {
        let a = trace(
            "env",
            &[(
                "1",
                "execve(\"/usr/bin/env\", [\"env\"], 0x0 /* 3 vars */) = 0\n\
             openat(AT_FDCWD, \"/usr/share/locale/locale.alias\", O_RDONLY) = 3\n\
             getrandom(\"\\x01\", 8, GRND_NONBLOCK) = 8\n\
             execve(\"/j/target/release/g\", [\"g\"], 0x0 /* 0 vars */) = 0\n\
             openat(AT_FDCWD, \"/etc/hostname\", O_RDONLY) = 3\n",
            )],
        );
        assert_eq!(a.getrandom, 0);
        assert_eq!(a.undeclared.len(), 1);
        assert!(a.undeclared["/etc/hostname"].contains("openat=ok"));
        assert!(a.executed["build-output"].contains("$JOB/target/release/g"));
    }

    /// A relative path resolves against the process's working directory,
    /// after a `chdir`; a path relative to a descriptor is not charged.
    #[test]
    fn relative_paths_resolve_against_the_working_directory() {
        let a = trace(
            "relative",
            &[(
                "2",
                "chdir(\"/j/src/crate\") = 0\n\
             openat(AT_FDCWD, \"src/lib.rs\", O_RDONLY) = 3\n\
             unlinkat(5, \"lib.rmeta\", 0) = 0\n\
             openat(AT_FDCWD, \"../../../../elsewhere\", O_RDONLY) = -1 ENOENT (No such file or directory)\n",
            )],
        );
        assert_eq!(a.classes.get("declared-source"), Some(&2));
        assert_eq!(a.classes.get("descriptor-relative"), Some(&1));
        assert_eq!(a.undeclared.len(), 1, "{:?}", a.undeclared);
    }

    /// Internet sockets are network attempts; local ones are not.
    #[test]
    fn only_internet_sockets_are_network() {
        let a = trace(
            "network",
            &[(
                "3",
                "socket(AF_UNIX, SOCK_STREAM, 0) = 3\n\
             recvfrom(4, \"\", 8, 0, NULL, NULL) = 0\n\
             socket(AF_INET, SOCK_STREAM, IPPROTO_IP) = 5\n\
             connect(5, {sa_family=AF_INET, sin_port=htons(80)}, 16) = -1 ENETUNREACH (Network is unreachable)\n",
            )],
        );
        assert_eq!(a.local_ipc, 2);
        assert_eq!(a.network.len(), 2);
        assert!(a.network[1].contains("= -1"));
    }

    #[test]
    fn paths_are_classified_by_where_they_are() {
        let l = layout(Path::new("/j"));
        assert_eq!(l.class("/j/src/Cargo.toml"), "declared-source");
        assert_eq!(l.class("/j/target/release/g"), "build-output");
        assert_eq!(l.class("/j/home/x"), "job-directory");
        assert_eq!(l.class("/toolchain/bin/rustc"), "toolchain");
        assert_eq!(l.class("/usr/lib/x86_64-linux-gnu/libc.so.6"), "system");
        assert_eq!(l.class("/proc/self/maps"), "kernel-interface");
        assert_eq!(l.class("/home/user/.cargo/config.toml"), "undeclared");
        assert_eq!(l.class("/jx/src"), "undeclared");
        assert_eq!(
            l.class(
                &normalize(Path::new("/j/src/../../etc/passwd"))
                    .display()
                    .to_string()
            ),
            "undeclared"
        );
    }

    /// A build that runs only the toolchain passes the build checks; the
    /// same build running a build script out of the target directory fails
    /// `build-executes-only-the-toolchain` and nothing else. The synthetic
    /// traces stand in for `strace`, which the harness's own run needs and
    /// these tests do not.
    #[test]
    fn a_build_script_fails_only_the_toolchain_check() {
        let toolchain = "execve(\"/toolchain/bin/cargo\", [\"cargo\"], 0x0 /* 4 vars */) = 0\n\
             execve(\"/toolchain/bin/rustc\", [\"rustc\"], 0x0 /* 4 vars */) = 0\n\
             execve(\"/usr/bin/cc\", [\"cc\"], 0x0 /* 4 vars */) = 0\n";
        let clean = BTreeMap::from([("a".to_string(), trace("clean", &[("1", toolchain)]))]);
        assert!(build_checks(&clean).values().all(|ok| *ok));

        let script = "execve(\"/j/target/release/build/c-0123/build-script-build\", [\"build-script-build\"], 0x0 /* 9 vars */) = 0\n\
             openat(AT_FDCWD, \"/etc/ld.so.cache\", O_RDONLY|O_CLOEXEC) = 3\n";
        let dirty = BTreeMap::from([(
            "a".to_string(),
            trace("script", &[("1", toolchain), ("2", script)]),
        )]);
        assert!(
            dirty["a"].executed["build-output"]
                .contains("$JOB/target/release/build/c-0123/build-script-build")
        );
        let checks = build_checks(&dirty);
        let failed: Vec<&str> = checks
            .iter()
            .filter(|(_, ok)| !**ok)
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(failed, ["build-executes-only-the-toolchain"]);
        assert!(fired_exactly(
            &failed,
            Subject::BuildScript.expected_failures()
        ));
    }

    #[test]
    fn the_control_option_names_a_subject_or_fails() {
        assert_eq!(Subject::from_control(None), Ok(Subject::Authoring));
        assert_eq!(
            Subject::from_control(Some("build-script")),
            Ok(Subject::BuildScript)
        );
        assert!(Subject::from_control(Some("build_script")).is_err());
        assert!(Subject::from_control(Some("")).is_err());
    }

    /// The experiment runs every check and names no control; the control
    /// runs the build checks, lists the rest as not applicable, and expects
    /// to fail one check that it runs.
    #[test]
    fn each_subject_runs_the_checks_it_declares() {
        let (all, none) = Subject::Authoring.checks();
        assert!(none.is_empty());
        assert_eq!(all.len(), BUILD_CHECKS.len() + GENERATOR_CHECKS.len());
        assert_eq!(Subject::Authoring.control(), None);
        assert!(Subject::Authoring.expected_failures().is_empty());
        assert_eq!(Subject::Authoring.krate(), CRATE);
        assert_eq!(Subject::Authoring.generators(), GENERATORS);

        let (run, skipped) = Subject::BuildScript.checks();
        assert_eq!(run, BUILD_CHECKS);
        assert_eq!(skipped, GENERATOR_CHECKS);
        let mut union: Vec<&str> = run.iter().chain(&skipped).copied().collect();
        union.sort_unstable();
        let mut every = all.clone();
        every.sort_unstable();
        assert_eq!(union, every);
        assert!(Subject::BuildScript.generators().is_empty());
        for expected in Subject::BuildScript.expected_failures() {
            assert!(run.contains(expected), "{expected} is expected but not run");
        }
    }

    /// The control crate is where the harness looks for it, has a build
    /// script, and has the lockfile `--locked` needs.
    #[test]
    fn the_build_script_control_crate_is_in_place() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let krate = root.join(Subject::BuildScript.krate());
        assert!(krate.join("build.rs").is_file());
        assert!(krate.join("Cargo.lock").is_file());
        let manifest = fs::read_to_string(krate.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("[workspace]"));
        assert!(
            Subject::BuildScript
                .declared()
                .contains(&BUILD_SCRIPT_CRATE)
        );
    }

    #[test]
    fn a_control_fires_only_on_exactly_its_expected_failures() {
        let expected = ["build-executes-only-the-toolchain"];
        assert!(fired_exactly(&expected, &expected));
        assert!(!fired_exactly(&[], &expected));
        assert!(!fired_exactly(
            &[
                "build-executes-only-the-toolchain",
                "build-opens-no-undeclared-file"
            ],
            &expected
        ));
        assert!(!fired_exactly(&[], &[]));
    }

    #[test]
    fn computed_checks_must_match_the_declared_checks() {
        let checks: BTreeMap<String, bool> =
            BUILD_CHECKS.iter().map(|c| (c.to_string(), true)).collect();
        assert!(check_names(&checks, BUILD_CHECKS).is_ok());
        assert!(check_names(&checks, &BUILD_CHECKS[..2]).is_err());
        let (all, _) = Subject::Authoring.checks();
        assert!(check_names(&checks, &all).is_err());
    }

    #[test]
    fn clock_calls_are_counted_from_a_valgrind_trace() {
        let dir = std::env::temp_dir().join(format!("nomos-hermeticity-vg-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("vg");
        fs::write(
            &log,
            "SYSCALL[1,1](228) sys_clock_gettime( 0, 0x1 )[sync] --> Success(0x0)\n\
             SYSCALL[1,1](257) sys_openat ( 1, 0x2(/etc/x), 0 ) --> Success(0x3)\n\
             SYSCALL[1,1](96) sys_gettimeofday ( 0x1, 0x0 ) --> Success(0x0)\n",
        )
        .unwrap();
        assert_eq!(clock_calls(&log).unwrap(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }
}
