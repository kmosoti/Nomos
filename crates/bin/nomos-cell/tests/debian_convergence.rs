//! Experiment `debian-fixed-point`, the Phase 1 exit (plan,
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

use debian::demo::{CONFIG, LOADED, PACKAGE, artifact, perturb, prepare, starting_states};
use debian::package_truth;
use nomos_core::condition::PackageVersion;

fn cell(args: &[&str]) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = nomos_cell::cli::run(&args, &mut out, &mut err);
    let text = String::from_utf8(out).unwrap() + &String::from_utf8(err).unwrap();
    (status, text)
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

/// Experiment `debian-fixed-point`: the host with nothing, the host with
/// everything wrong, the converged host, and the converged host with each
/// Condition in turn absent and wrong.
#[test]
#[ignore = "changes the host; run by `cargo xtask debian`"]
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
    for (label, changes) in starting_states() {
        for (r, holds) in changes {
            perturb(r, holds);
        }
        let executions = converges(&label, state, canon, bundle);
        runs.push((label, executions));
    }
    for (label, executions) in &runs {
        println!("{label}: converged with {executions} executions");
    }
    assert_eq!(runs[2].1, 0, "the converged host was changed");
    assert!(runs.iter().enumerate().all(|(i, (_, n))| i == 2 || *n > 0));
}
