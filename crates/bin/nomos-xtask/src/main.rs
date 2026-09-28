//! # nomos-xtask
//!
//! **Development tooling.** Invoked as `cargo xtask <command>` through the
//! alias in `.cargo/config.toml`. It lives in `crates/bin/` because that layer
//! may depend on anything, and nothing depends on it (ADR 0003).
//!
//! Commands:
//!
//! - `research verify <snapshot-dir>`: verify a research snapshot as a whole.
//!   The manifest is required and must cover every file; then the NDJSON graph
//!   is parsed strictly, the referential and structural checks and negative
//!   controls run, and every record is validated against the snapshot's JSON
//!   Schema. Prints a report with one block per stage.
//! - `research list <snapshot-dir>`: print the recommendations in review order.
//! - `research verify-all <research-dir>`: verify every `*/snapshot` under it.
//! - `research reproduce`: run the seven counterexample models of the draft
//!   formal documents and print their report as JSON.
//! - `research frozen --base <ref>`: fail if an accepted snapshot changed
//!   relative to the merge base with `<ref>`.
//! - `check-layers [--manifest-path <Cargo.toml>]`: the dependency-policy
//!   check of ADR 0000 over the declared and resolved graphs.
//!
//! The tool checks artifact integrity and workspace policy. It establishes
//! nothing about Nomos semantics.

mod counterexamples;
mod freeze;
mod graph;
mod layers;
mod manifest;
mod snapshot;
mod strict_json;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  cargo xtask research verify     <snapshot-dir> [--report <file>] [--no-schema]
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
    let with_schema = !rest.contains(&"--no-schema");
    let (files, report) = snapshot::verify_snapshot(dir, with_schema)?;
    for (name, ok) in files {
        eprintln!("{name}: {}", if ok { "OK" } else { "FAILED" });
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
        let (files, _) = snapshot::verify_snapshot(&dir, true)?;
        println!("{}: verified ({} files)", dir.display(), files.len());
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
    Err(format!(
        "accepted research snapshots are frozen; a revision is a new dated snapshot:\n{}",
        violations.join("\n")
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
