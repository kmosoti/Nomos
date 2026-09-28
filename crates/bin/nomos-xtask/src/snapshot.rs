//! Whole-snapshot verification, shared by the CLI and the tests.
//!
//! A snapshot is verifiable only as a whole: the manifest must exist, every
//! file it names must match, every file in the directory must be named, and
//! only then is the graph read. A missing manifest is a failure, not a
//! skipped stage.
//!
//! What this cannot establish: that the manifest itself is the one accepted.
//! Rewriting a file and its checksum together passes here. Freezing accepted
//! snapshots against their merged baseline is a Git comparison and belongs to
//! CI, not to a checksum.

use std::path::Path;

use serde::Serialize;

use crate::{graph, manifest};

const MANIFEST: &str = "MANIFEST.sha256";
const GRAPH: &str = "nomos-research.ndjson";
const SCHEMA: &str = "record.schema.json";

/// Both stages, reported separately.
#[derive(Serialize)]
pub(crate) struct SnapshotReport {
    manifest: ManifestReport,
    graph: graph::Report,
}

impl SnapshotReport {
    /// The manifest stage.
    #[cfg(test)]
    pub(crate) fn manifest(&self) -> &ManifestReport {
        &self.manifest
    }
}

/// The manifest stage: every named file matched, and every file was named.
#[derive(Serialize)]
pub(crate) struct ManifestReport {
    status: &'static str,
    files_verified: usize,
}

impl ManifestReport {
    /// How many files the manifest named and verified.
    #[cfg(test)]
    pub(crate) fn files_verified(&self) -> usize {
        self.files_verified
    }
}

/// Verifies a snapshot directory. Returns the per-file manifest results and the report.
pub(crate) fn verify_snapshot(
    dir: &Path,
    with_schema: bool,
) -> Result<(Vec<(String, bool)>, SnapshotReport), String> {
    let manifest_path = dir.join(MANIFEST);
    if !manifest_path.is_file() {
        return Err(format!(
            "{}: {MANIFEST} is required; a snapshot without a manifest is not verifiable",
            dir.display()
        ));
    }
    let files = manifest::verify(&manifest_path)?;
    if files.is_empty() {
        return Err(format!(
            "{}: {MANIFEST} lists no files",
            manifest_path.display()
        ));
    }

    let mut present: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != MANIFEST)
        .collect();
    present.sort();
    for name in &present {
        if !files.iter().any(|(listed, _)| listed == name) {
            return Err(format!(
                "{}: {name} is not covered by {MANIFEST}",
                dir.display()
            ));
        }
    }
    for required in [GRAPH, SCHEMA] {
        if !files.iter().any(|(listed, _)| listed == required) {
            return Err(format!(
                "{}: {MANIFEST} does not name {required}",
                dir.display()
            ));
        }
    }

    let schema = with_schema.then(|| dir.join(SCHEMA));
    let report = graph::verify(&dir.join(GRAPH), schema.as_deref())?;
    let manifest_report = ManifestReport {
        status: "verified",
        files_verified: files.len(),
    };
    Ok((
        files,
        SnapshotReport {
            manifest: manifest_report,
            graph: report,
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::verify_snapshot;

    fn shipped() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/research/2026-09-28-typed-core/snapshot")
    }

    /// A private copy of the shipped snapshot to corrupt.
    fn scratch_copy(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nomos-xtask-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for entry in std::fs::read_dir(shipped()).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
        }
        dir
    }

    fn append(path: &Path, bytes: &[u8]) {
        let mut content = std::fs::read(path).unwrap();
        content.extend_from_slice(bytes);
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn shipped_snapshot_verifies_as_a_whole() {
        let (files, report) = verify_snapshot(&shipped(), true).unwrap();
        assert_eq!(files.len(), 9);
        assert!(files.iter().all(|(_, ok)| *ok));
        assert_eq!(report.manifest().files_verified(), 9);
    }

    #[test]
    fn a_changed_non_graph_file_fails_the_manifest_stage() {
        let dir = scratch_copy("changed-readme");
        append(&dir.join("README.md"), b"\nedited\n");
        let error = verify_snapshot(&dir, false)
            .err()
            .expect("expected a failure");
        assert!(error.contains("checksum mismatch: README.md"), "{error}");
    }

    #[test]
    fn a_missing_manifest_is_a_failure_not_a_skipped_stage() {
        let dir = scratch_copy("no-manifest");
        std::fs::remove_file(dir.join("MANIFEST.sha256")).unwrap();
        let error = verify_snapshot(&dir, false)
            .err()
            .expect("expected a failure");
        assert!(error.contains("MANIFEST.sha256 is required"), "{error}");
    }

    #[test]
    fn an_empty_manifest_fails() {
        let dir = scratch_copy("empty-manifest");
        std::fs::write(dir.join("MANIFEST.sha256"), "").unwrap();
        let error = verify_snapshot(&dir, false)
            .err()
            .expect("expected a failure");
        assert!(error.contains("lists no files"), "{error}");
    }

    #[test]
    fn a_file_the_manifest_does_not_name_fails_coverage() {
        let dir = scratch_copy("uncovered-file");
        std::fs::write(dir.join("notes.md"), "not in the manifest\n").unwrap();
        let error = verify_snapshot(&dir, false)
            .err()
            .expect("expected a failure");
        assert!(error.contains("notes.md is not covered"), "{error}");
    }

    #[test]
    fn a_manifest_that_omits_the_graph_fails() {
        let dir = scratch_copy("no-graph-entry");
        let manifest = std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap();
        let kept: Vec<&str> = manifest
            .lines()
            .filter(|l| !l.ends_with("nomos-research.ndjson"))
            .collect();
        std::fs::write(dir.join("MANIFEST.sha256"), kept.join("\n") + "\n").unwrap();
        std::fs::remove_file(dir.join("nomos-research.ndjson")).unwrap();
        let error = verify_snapshot(&dir, false)
            .err()
            .expect("expected a failure");
        assert!(
            error.contains("does not name nomos-research.ndjson"),
            "{error}"
        );
    }

    /// Documented limit: a file and its checksum rewritten together pass the
    /// checksum stage. Freezing accepted snapshots is a Git comparison in CI.
    #[test]
    fn a_file_and_its_checksum_rewritten_together_pass_checksums() {
        let dir = scratch_copy("rewritten-pair");
        let readme = dir.join("README.md");
        append(&readme, b"\nedited\n");
        let digest = crate::manifest::sha256_hex(&std::fs::read(&readme).unwrap());
        let manifest = std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap();
        let rewritten: Vec<String> = manifest
            .lines()
            .map(|l| {
                if l.ends_with("  README.md") {
                    format!("{digest}  README.md")
                } else {
                    l.to_owned()
                }
            })
            .collect();
        std::fs::write(dir.join("MANIFEST.sha256"), rewritten.join("\n") + "\n").unwrap();
        assert!(verify_snapshot(&dir, false).is_ok());
    }
}
