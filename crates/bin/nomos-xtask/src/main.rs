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
//!
//! The tool checks artifact integrity and workspace policy. It establishes
//! nothing about Nomos semantics.

mod counterexamples;
mod error;
mod freeze;
mod graph;
mod layers;
mod manifest;
mod snapshot;
mod strict_json;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  cargo xtask research verify     <snapshot-dir> [--report <file>]
  cargo xtask research verify-all <research-dir>
  cargo xtask research list       <snapshot-dir>
  cargo xtask research reproduce  [--out <file>]
  cargo xtask research frozen     --base <ref>
  cargo xtask check-layers        [--manifest-path <Cargo.toml>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = match words.as_slice() {
        ["research", "verify", dir, rest @ ..] => verify(Path::new(dir), rest),
        ["research", "verify-all", dir] => verify_all(Path::new(dir)),
        ["research", "frozen", rest @ ..] => frozen(rest),
        ["check-layers", rest @ ..] => check_layers(rest),
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
