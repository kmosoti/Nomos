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
//! - `research reproduce`: run the seven counterexample models of the draft
//!   formal documents and print their report as JSON.
//!
//! The tool checks artifact integrity. It establishes nothing about Nomos.

mod counterexamples;
mod graph;
mod manifest;
mod snapshot;
mod strict_json;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  cargo xtask research verify <snapshot-dir> [--report <file>] [--no-schema]
  cargo xtask research list   <snapshot-dir>
  cargo xtask research reproduce [--out <file>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = match words.as_slice() {
        ["research", "verify", dir, rest @ ..] => verify(Path::new(dir), rest),
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
