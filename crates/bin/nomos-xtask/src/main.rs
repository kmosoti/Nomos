//! # nomos-xtask
//!
//! **Development tooling.** Invoked as `cargo xtask <command>` through the
//! alias in `.cargo/config.toml`. It lives in `crates/bin/` because that layer
//! may depend on anything, and nothing depends on it (ADR 0003).
//!
//! Commands:
//!
//! - `research verify <snapshot-dir>`: verify a research snapshot as a whole
//!   through `snapshot::verify_snapshot`, the same function the tests call.
//!   Manifest, coverage, strict NDJSON, JSON Schema, references, structural
//!   invariants, and negative controls all run; there is no weaker mode.
//!   Prints a report with one block per stage, or the failure's stable code.
//! - `research list <snapshot-dir>`: print the recommendations in review order.
//! - `research verify-all <research-dir>`: verify every `*/snapshot` under it.
//! - `research reproduce`: run the seven counterexample models of the draft
//!   formal documents and print their report as JSON.
//! - `research frozen --base <ref>`: fail if an accepted snapshot changed
//!   relative to the merge base with `<ref>`. Each change prints one `FROZEN`
//!   line with its stable code; an unavailable base fails closed.
//! - `check-layers [--manifest-path <Cargo.toml>]`: the dependency-policy
//!   check of ADR 0000 over the declared and resolved graphs. Prints the
//!   report as JSON and one `FORBIDDEN` line per violation, with its stable
//!   rule code, both packages, both layers, and the dependency kind.
//! - `check-core-purity [--manifest-path <Cargo.toml>]`: the core purity
//!   policy of `crates/core/PURITY.toml` (ADR 0016) over the declared and
//!   resolved graphs and the crate roots. Prints the report as JSON and one
//!   `IMPURE` line per violation, with its stable code.
//! - `check-trust-boundary --base <ref>`: every non-merge commit since the
//!   merge base with `<ref>` declares the oracle it changes (ADR 0015), in a
//!   `Trust-Boundary:` line, and adds no undeclared escape hatch. Prints one
//!   `UNDECLARED` line per violation, with its stable code.
//! - `receipts validate [--dir <dir>]`: every receipt under
//!   `verification/receipts/` is one strict JSON object per line, satisfies
//!   `verification/receipt.schema.json`, names a check registered in
//!   `verification/checks.toml`, and claims `passed` only with evidence of a
//!   zero exit status. Prints one `REJECTED` line per violation.
//! - `receipts record <check-id> --out <file> -- <command...>`: runs the
//!   command, never through a shell, and appends a receipt written from what
//!   happened. Exits non-zero when the command did.
//! - `hermeticity [--scratch <dir>] [--out <file>]`: experiment
//!   `build-hermeticity`. Builds the Canon authoring crate twice in
//!   isolation, runs its generators, compares the IR, and records every
//!   undeclared input the jobs touched. Prints the record as JSON and one
//!   `LEAK` line per failed check; needs `strace`, `unshare`, and `valgrind`,
//!   and fails when one is missing. With `--control build-script` it builds
//!   the negative control `tests/fixtures/canon/authoring-build-script`
//!   instead, runs only the build checks, and names in the record the checks
//!   that ran, those that did not apply, and the one it must fail,
//!   `build-executes-only-the-toolchain`. A control run exits non-zero when
//!   any check fails, the expected one included, and says on its last line
//!   whether the control fired exactly as expected.
//! - `mutants semantic [--corpus <corpus.toml>]`: applies every active
//!   semantic mutant of `tests/semantic-mutants/corpus.toml` to a scratch
//!   copy of the workspace and runs its named test, which must fail. Prints
//!   one line per mutant with its outcome.
//! - `generator-variance [--candidates <dir>] [--out <file>]`: experiment
//!   `generator-variance`. Splices each candidate implementation of the
//!   assessment functions into a scratch copy of the workspace, runs its own
//!   tests, the fixed oracle, and the semantic mutants of the reference
//!   against its tests, and compares every candidate that compiles on
//!   exhaustive bounded and generated inputs. Prints the record as JSON; fails
//!   only when the negative control is not reported as own-tests-accepted and
//!   oracle-rejected, or a step cannot run. A candidate's outcome never fails it.
//! - `debian --release <12|13> [--test <package>/<test>] [--out <file>]`:
//!   the Linux suites on a disposable Debian container with systemd as PID 1
//!   (Phase 1 plan, decision 4). Builds the tests as static binaries, runs
//!   each in the container as root, and prints the record as JSON; fails
//!   when the harness cannot run or any test fails. Needs Docker and the
//!   `x86_64-unknown-linux-musl` target.
//! - `package [--version <v>] [--out <dir>]`: the Cell as a Debian package
//!   (`16-alpha-release`). Builds `nomos-cell` as a static release binary
//!   and assembles `nomos-cell_<version>_amd64.deb` with its service, timer,
//!   and maintainer scripts, at the workspace's version in Debian form
//!   unless one is named. Prints the package's path and SHA-256. Needs the
//!   `x86_64-unknown-linux-musl` target and `dpkg-deb`.
//!
//! The tool checks artifact integrity and workspace policy. It establishes
//! nothing about Nomos semantics.

mod counterexamples;
mod debian;
mod error;
mod freeze;
mod generator_variance;
mod graph;
mod hermeticity;
mod layers;
mod manifest;
mod package;
mod purity;
mod receipt;
mod semantic;
mod snapshot;
mod strict_json;
mod trust;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  cargo xtask research verify     <snapshot-dir> [--report <file>]
  cargo xtask research verify-all <research-dir>
  cargo xtask research list       <snapshot-dir>
  cargo xtask research reproduce  [--out <file>]
  cargo xtask research frozen     --base <ref>
  cargo xtask check-layers        [--manifest-path <Cargo.toml>]
  cargo xtask check-core-purity   [--manifest-path <Cargo.toml>]
  cargo xtask check-trust-boundary --base <ref>
  cargo xtask receipts validate   [--dir <receipts-dir>]
  cargo xtask receipts record     <check-id> --out <file.ndjson> [--unchecked <text>] [--properties a,b] -- <command...>
  cargo xtask mutants semantic    [--corpus <corpus.toml>]
  cargo xtask hermeticity         [--scratch <dir>] [--out <file.json>] [--control build-script]
  cargo xtask generator-variance  [--candidates <dir>] [--out <file.json>]
  cargo xtask debian              --release <12|13> [--test <package>/<test>] [--out <file.json>]
  cargo xtask package             [--version <debian-version>] [--out <dir>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = match words.as_slice() {
        ["research", "verify", dir, rest @ ..] => verify(Path::new(dir), rest),
        ["research", "verify-all", dir] => verify_all(Path::new(dir)),
        ["research", "frozen", rest @ ..] => frozen(rest),
        ["check-layers", rest @ ..] => check_layers(rest),
        ["check-core-purity", rest @ ..] => check_core_purity(rest),
        ["check-trust-boundary", rest @ ..] => check_trust_boundary(rest),
        ["receipts", "validate", rest @ ..] => receipts_validate(rest),
        ["receipts", "record", check_id, rest @ ..] => receipts_record(check_id, rest),
        ["mutants", "semantic", rest @ ..] => mutants_semantic(rest),
        ["hermeticity", rest @ ..] => hermeticity(rest),
        ["generator-variance", rest @ ..] => generator_variance(rest),
        ["debian", rest @ ..] => debian(rest),
        ["package", rest @ ..] => package(rest),
        ["research", "list", dir] => list(Path::new(dir)),
        ["research", "reproduce", rest @ ..] => reproduce(rest),
        _ => Err(USAGE.to_string()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// Reads `--flag <value>` pairs and bare `--flag` switches from the tail of a command line.
fn option<'a>(rest: &'a [&'a str], flag: &str) -> Result<Option<&'a str>, String> {
    match rest.iter().position(|w| *w == flag) {
        None => Ok(None),
        Some(i) => rest
            .get(i + 1)
            .copied()
            .map(Some)
            .ok_or_else(|| format!("{flag} needs a value\n{USAGE}")),
    }
}

fn verify(dir: &Path, rest: &[&str]) -> Result<(), String> {
    let report_path = option(rest, "--report")?.map(PathBuf::from);
    let report = snapshot::verify_snapshot(dir)?;
    for name in report.files() {
        eprintln!("{name}: OK");
    }
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    if let Some(path) = report_path {
        std::fs::write(&path, &rendered).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    print!("{rendered}");
    Ok(())
}

fn verify_all(research: &Path) -> Result<(), String> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(research)
        .map_err(|e| format!("{}: {e}", research.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("snapshot"))
        .filter(|snapshot| snapshot.is_dir())
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        return Err(format!("{}: no */snapshot directories", research.display()));
    }
    for dir in dirs {
        let report = snapshot::verify_snapshot(&dir)?;
        println!(
            "{}: verified ({} files)",
            dir.display(),
            report.files().len()
        );
    }
    Ok(())
}

fn frozen(rest: &[&str]) -> Result<(), String> {
    let base =
        option(rest, "--base")?.ok_or_else(|| format!("--base <ref> is required\n{USAGE}"))?;
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let violations = freeze::frozen(&root, base)?;
    if violations.is_empty() {
        println!("research snapshots unchanged since the merge base with {base}");
        return Ok(());
    }
    for violation in &violations {
        eprintln!("FROZEN {violation}");
    }
    Err(format!(
        "{} change(s) to accepted research snapshots; a revision is a new dated snapshot",
        violations.len()
    ))
}

fn debian(rest: &[&str]) -> Result<(), String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let release =
        option(rest, "--release")?.ok_or_else(|| format!("--release is required\n{USAGE}"))?;
    let tests: Vec<(String, String)> = match option(rest, "--test")? {
        Some(t) => {
            let (package, test) = t
                .split_once('/')
                .ok_or_else(|| format!("--test {t}: expected <package>/<test>"))?;
            vec![(package.to_string(), test.to_string())]
        }
        None => debian::DEFAULT_TESTS
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect(),
    };
    let record = debian::run_suites(&root, release, &tests, debian::Selection::default())?;
    let rendered = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())? + "\n";
    if let Some(out) = option(rest, "--out")? {
        std::fs::write(out, &rendered).map_err(|e| format!("{out}: {e}"))?;
    }
    print!("{rendered}");
    if record.passed() {
        Ok(())
    } else {
        Err(format!("the suites failed on {}", record.os_release))
    }
}

fn package(rest: &[&str]) -> Result<(), String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let version = match option(rest, "--version")? {
        Some(v) => v.to_string(),
        None => package::debian_version(&package::workspace_version(&root)?),
    };
    let out = root.join(option(rest, "--out")?.unwrap_or("target/deb"));
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let binary = package::binary(&root)?;
    let deb = package::assemble(&root, &binary, &out, &version)?;
    println!("{} {}", deb.display(), package::sha256(&deb)?);
    Ok(())
}

fn hermeticity(rest: &[&str]) -> Result<(), String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let scratch = option(rest, "--scratch")?
        .map(|s| root.join(s))
        // Outside the workspace: Cargo reads `.cargo/config.toml` in every
        // ancestor of the build, and the workspace's own would be read.
        .unwrap_or_else(|| std::env::temp_dir().join("nomos-hermeticity"));
    let subject = hermeticity::Subject::from_control(option(rest, "--control")?)?;
    let record = hermeticity::run(&root, &scratch, subject)?;
    let rendered = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())? + "\n";
    if let Some(out) = option(rest, "--out")? {
        std::fs::write(out, &rendered).map_err(|e| format!("{out}: {e}"))?;
    }
    print!("{rendered}");
    let failures = record.failures();
    for check in &failures {
        eprintln!("LEAK [{check}]");
    }
    match record.control_fired() {
        Some(true) => {
            eprintln!("control fired: it failed exactly the checks it exists to fail")
        }
        Some(false) => eprintln!(
            "CONTROL DID NOT FIRE AS EXPECTED: the failed checks are not exactly the expected ones"
        ),
        None => {}
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} build-hermeticity check(s) failed",
            failures.len()
        ))
    }
}

fn generator_variance(rest: &[&str]) -> Result<(), String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let candidates = option(rest, "--candidates")?
        .map(|c| root.join(c))
        .unwrap_or_else(|| root.join("tests/fixtures/generator-variance"));
    let record = generator_variance::run(&generator_variance::Options {
        candidates,
        // Outside the workspace, for the reason `hermeticity` gives.
        scratch: std::env::temp_dir()
            .join(format!("nomos-generator-variance-{}", std::process::id())),
        // Shared with the semantic-mutant runner: both build scratch copies
        // of the same workspace, so the dependencies build once.
        target_dir: root.join("target/semantic-mutants"),
        root,
    })?;
    let rendered = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())? + "\n";
    if let Some(out) = option(rest, "--out")? {
        std::fs::write(out, &rendered).map_err(|e| format!("{out}: {e}"))?;
    }
    print!("{rendered}");
    let failures = record.failures();
    for failure in &failures {
        eprintln!("NOT MEASURING [{failure}]");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} harness check(s) failed", failures.len()))
    }
}

fn check_layers(rest: &[&str]) -> Result<(), String> {
    let manifest_path = option(rest, "--manifest-path")?
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("Cargo.toml"));
    let report = layers::check(&layers::Options {
        manifest_path,
        locked: true,
        offline: false,
    })?;
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    print!("{rendered}");
    for violation in report.violations() {
        eprintln!("FORBIDDEN {violation}");
    }
    if report.violations().is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} forbidden workspace edge(s); see ADR 0000",
            report.violations().len()
        ))
    }
}

fn check_core_purity(rest: &[&str]) -> Result<(), String> {
    let manifest_path = option(rest, "--manifest-path")?
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("Cargo.toml"));
    let report = purity::check(&layers::Options {
        manifest_path,
        locked: true,
        offline: false,
    })?;
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    print!("{rendered}");
    for violation in report.violations() {
        eprintln!("IMPURE {violation}");
    }
    if report.violations().is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} core purity violation(s); see {}",
            report.violations().len(),
            purity::POLICY_PATH
        ))
    }
}

fn check_trust_boundary(rest: &[&str]) -> Result<(), String> {
    let base =
        option(rest, "--base")?.ok_or_else(|| format!("--base <ref> is required\n{USAGE}"))?;
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let report = trust::check(&root, base)?;
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    print!("{rendered}");
    for violation in report.violations() {
        eprintln!("UNDECLARED {violation}");
    }
    if report.violations().is_empty() {
        println!(
            "{} commit(s) since the merge base with {base} declare their trust-boundary changes",
            report.commits_checked()
        );
        Ok(())
    } else {
        Err(format!(
            "{} trust-boundary violation(s); see {}",
            report.violations().len(),
            trust::POLICY_PATH
        ))
    }
}

fn receipts_validate(rest: &[&str]) -> Result<(), String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut paths = receipt::Paths::under(&root);
    if let Some(dir) = option(rest, "--dir")? {
        paths.receipts = PathBuf::from(dir);
    }
    let report = receipt::validate(&paths)?;
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    print!("{rendered}");
    for violation in report.violations() {
        eprintln!("REJECTED {violation}");
    }
    if report.violations().is_empty() {
        println!("{} receipt(s) valid", report.receipts());
        Ok(())
    } else {
        Err(format!(
            "{} receipt violation(s); see {}",
            report.violations().len(),
            receipt::SCHEMA_PATH
        ))
    }
}

fn receipts_record(check_id: &str, rest: &[&str]) -> Result<(), String> {
    let (options, argv) = match rest.iter().position(|w| *w == "--") {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => return Err(format!("the command follows --\n{USAGE}")),
    };
    let out = option(options, "--out")?
        .map(PathBuf::from)
        .ok_or_else(|| format!("--out <file.ndjson> is required\n{USAGE}"))?;
    let unchecked = option(options, "--unchecked")?
        .unwrap_or("not stated")
        .to_owned();
    let properties: Vec<String> = option(options, "--properties")?
        .map(|p| p.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let line = receipt::record(&receipt::Context {
        root,
        check_id: check_id.to_owned(),
        argv: argv.iter().map(|w| (*w).to_owned()).collect(),
        properties,
        unchecked,
        out: Some(out.clone()),
    })?;
    let rendered = serde_json::to_string(&line).map_err(|e| e.to_string())? + "\n";
    let mut existing = std::fs::read_to_string(&out).unwrap_or_default();
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(&rendered);
    std::fs::write(&out, existing).map_err(|e| format!("{}: {e}", out.display()))?;
    print!("{rendered}");
    if line["result"] == "passed" {
        Ok(())
    } else {
        Err(format!("{check_id}: recorded as {}", line["result"]))
    }
}

fn mutants_semantic(rest: &[&str]) -> Result<(), String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let corpus = option(rest, "--corpus")?
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(semantic::CORPUS_PATH));
    let report = semantic::run(&semantic::Options {
        target_dir: root.join("target/semantic-mutants"),
        root,
        corpus,
        offline: false,
    })?;
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    print!("{rendered}");
    for row in report.rows() {
        eprintln!("{row}");
    }
    let failures = report.failures();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} active semantic mutant(s) not caught by their named test",
            failures.len()
        ))
    }
}

fn list(dir: &Path) -> Result<(), String> {
    let graph_path = dir.join("nomos-research.ndjson");
    for line in graph::recommendation_index(&graph_path)? {
        println!("{line}");
    }
    Ok(())
}

fn reproduce(rest: &[&str]) -> Result<(), String> {
    let out = option(rest, "--out")?.map(PathBuf::from);
    let report = counterexamples::report();
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    if let Some(path) = out {
        std::fs::write(&path, &rendered).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    print!("{rendered}");
    Ok(())
}
