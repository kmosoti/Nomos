//! Whole-snapshot verification: the one operation the CLI and the tests share.
//!
//! [`verify_snapshot`] runs every stage, in order, and there is no weaker
//! mode. A snapshot passes only if all of these hold:
//!
//! 0. **Inventory.** The snapshot is a flat directory of regular files. The
//!    directory itself is not a symlink, and it holds no symlink, no
//!    subdirectory, no socket, FIFO, or device, and no name that is not UTF-8.
//!    Every directory entry is read; an unreadable one fails. This runs before
//!    any file is hashed, so the bytes verified are the bytes in the directory.
//! 1. **Manifest.** `MANIFEST.sha256` exists, parses, names no file twice and
//!    no path outside the directory, and every digest matches.
//! 2. **Coverage.** Every file in the directory is named by the manifest, and
//!    the manifest names the graph and its schema.
//! 3. **Graph.** The NDJSON parses strictly (duplicate keys and non-finite
//!    numbers rejected), every record validates against the JSON Schema, every
//!    reference resolves, the structural invariants hold, and each built-in
//!    negative control is rejected for its expected reason.
//!
//! What this cannot establish: that the manifest is the one that was accepted.
//! A file and its checksum rewritten together pass here. Freezing accepted
//! snapshots is a Git comparison against an accepted revision, run in CI on
//! every push and pull request by `cargo xtask research frozen`.

use std::path::Path;

use serde::Serialize;

use crate::error::{Code, VerificationError, Verified, require};
use crate::{graph, manifest};

const MANIFEST: &str = "MANIFEST.sha256";
const GRAPH: &str = "nomos-research.ndjson";
const SCHEMA: &str = "record.schema.json";

/// Every stage's outcome. Produced only when every stage passed.
#[derive(Serialize)]
pub(crate) struct VerificationReport {
    manifest: ManifestStage,
    coverage: CoverageStage,
    graph: graph::Report,
}

/// The manifest stage.
#[derive(Serialize)]
pub(crate) struct ManifestStage {
    status: &'static str,
    files: Vec<String>,
}

/// The coverage stage.
#[derive(Serialize)]
pub(crate) struct CoverageStage {
    status: &'static str,
    files_present: usize,
    required: [&'static str; 2],
}

impl VerificationReport {
    /// The files the manifest named, all of which matched.
    pub(crate) fn files(&self) -> &[String] {
        &self.manifest.files
    }

    /// The graph stage.
    #[cfg(test)]
    pub(crate) fn graph(&self) -> &graph::Report {
        &self.graph
    }
}

/// Verifies a snapshot directory as a whole.
/// The regular files of a snapshot directory, sorted, or the first entry that
/// is not one. Symlinks are never followed.
fn inventory(dir: &Path) -> Verified<Vec<String>> {
    let unreadable = |e: std::io::Error| {
        VerificationError::new(Code::SnapshotUnreadable, format!("{}: {e}", dir.display()))
    };
    let dir_meta = std::fs::symlink_metadata(dir).map_err(unreadable)?;
    require(
        !dir_meta.file_type().is_symlink(),
        Code::SnapshotSymlink,
        || format!("{}: the snapshot directory is a symlink", dir.display()),
    )?;
    require(dir_meta.is_dir(), Code::SnapshotUnreadable, || {
        format!("{}: not a directory", dir.display())
    })?;
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let path = entry.path();
        let name = entry.file_name().into_string().map_err(|raw| {
            VerificationError::new(
                Code::SnapshotNameInvalid,
                format!("{}: {raw:?} is not a UTF-8 file name", dir.display()),
            )
        })?;
        // DirEntry::file_type does not follow symlinks.
        let kind = entry.file_type().map_err(|e| {
            VerificationError::new(Code::SnapshotUnreadable, format!("{}: {e}", path.display()))
        })?;
        if kind.is_symlink() {
            return Err(VerificationError::new(
                Code::SnapshotSymlink,
                format!("{}: symlinks are not snapshot content", path.display()),
            ));
        }
        if kind.is_dir() {
            return Err(VerificationError::new(
                Code::SnapshotSubdirectory,
                format!("{}: a snapshot is a flat directory", path.display()),
            ));
        }
        if !kind.is_file() {
            return Err(VerificationError::new(
                Code::SnapshotSpecialFile,
                format!(
                    "{}: only regular files are snapshot content",
                    path.display()
                ),
            ));
        }
        files.push(name);
    }
    files.sort();
    Ok(files)
}

/// Verifies a snapshot directory as a whole.
pub(crate) fn verify_snapshot(dir: &Path) -> Verified<VerificationReport> {
    // Inventory stage: regular files only, before anything is hashed.
    let regular = inventory(dir)?;

    // Manifest stage.
    require(
        regular.iter().any(|f| f == MANIFEST),
        Code::ManifestMissing,
        || {
            format!(
                "{}: {MANIFEST} is required; a snapshot without a manifest is not verifiable",
                dir.display()
            )
        },
    )?;
    let entries = manifest::verify(&dir.join(MANIFEST))?;
    let files: Vec<String> = entries.into_iter().map(|(name, _)| name).collect();

    // Coverage stage.
    let present: Vec<String> = regular
        .into_iter()
        .filter(|name| name != MANIFEST)
        .collect();
    for name in &present {
        require(files.contains(name), Code::UncoveredFile, || {
            format!("{}: {name} is not covered by {MANIFEST}", dir.display())
        })?;
    }
    for required in [GRAPH, SCHEMA] {
        require(
            files.iter().any(|f| f == required),
            Code::RequiredFileUnlisted,
            || format!("{}: {MANIFEST} does not name {required}", dir.display()),
        )?;
    }

    // Graph stage, schema included.
    let graph = graph::verify(&dir.join(GRAPH), &dir.join(SCHEMA))?;

    Ok(VerificationReport {
        manifest: ManifestStage {
            status: "verified",
            files,
        },
        coverage: CoverageStage {
            status: "verified",
            files_present: present.len(),
            required: [GRAPH, SCHEMA],
        },
        graph,
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::Value;

    use super::verify_snapshot;
    use crate::error::Code;
    use crate::manifest::sha256_hex;

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

    /// The failure code, for a verification expected to fail.
    fn failure(dir: &Path) -> Code {
        match verify_snapshot(dir) {
            Ok(_) => panic!("{} verified; expected a failure", dir.display()),
            Err(error) => error.code(),
        }
    }

    fn append(path: &Path, bytes: &[u8]) {
        let mut content = std::fs::read(path).unwrap();
        content.extend_from_slice(bytes);
        std::fs::write(path, content).unwrap();
    }

    /// Rewrites the manifest so every listed file matches again. Used to push a
    /// corruption past the checksum stage and prove a later stage catches it.
    fn reseal(dir: &Path) {
        let manifest = std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap();
        let resealed: Vec<String> = manifest
            .lines()
            .map(|line| {
                let (_, name) = line.split_once("  ").unwrap();
                let digest = sha256_hex(&std::fs::read(dir.join(name)).unwrap());
                format!("{digest}  {name}")
            })
            .collect();
        std::fs::write(dir.join("MANIFEST.sha256"), resealed.join("\n") + "\n").unwrap();
    }

    fn graph_lines(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("nomos-research.ndjson"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn write_graph(dir: &Path, lines: &[String]) {
        std::fs::write(dir.join("nomos-research.ndjson"), lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn shipped_snapshot_verifies_and_matches_its_own_report() {
        let report = verify_snapshot(&shipped()).unwrap();
        assert_eq!(report.files().len(), 9);
        let shipped_report: Value = serde_json::from_str(
            &std::fs::read_to_string(shipped().join("validation-report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            report.graph().graph_sha256(),
            shipped_report["graph_sha256"].as_str().unwrap()
        );
        assert_eq!(
            Value::Object(report.graph().counts().clone()),
            shipped_report["counts"]
        );
        assert_eq!(
            Value::Object(report.graph().negative_controls().clone()),
            shipped_report["negative_controls"]
        );
    }

    // A. A modified file with an untouched manifest.
    #[test]
    fn a_modified_file_fails_the_checksum() {
        let dir = scratch_copy("modified");
        append(&dir.join("README.md"), b"\nedited\n");
        assert_eq!(failure(&dir), Code::ChecksumMismatch);
    }

    // B. No manifest at all.
    #[test]
    fn a_missing_manifest_fails() {
        let dir = scratch_copy("no-manifest");
        std::fs::remove_file(dir.join("MANIFEST.sha256")).unwrap();
        assert_eq!(failure(&dir), Code::ManifestMissing);
    }

    // C. A file kept, its manifest entry removed.
    #[test]
    fn a_missing_manifest_entry_fails_coverage() {
        let dir = scratch_copy("missing-entry");
        let manifest = std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap();
        let kept: Vec<&str> = manifest
            .lines()
            .filter(|l| !l.ends_with("  RECOMMENDATIONS.md"))
            .collect();
        assert_eq!(kept.len(), 8);
        std::fs::write(dir.join("MANIFEST.sha256"), kept.join("\n") + "\n").unwrap();
        assert_eq!(failure(&dir), Code::UncoveredFile);
    }

    // D. A duplicate object key in a graph record, past a resealed manifest.
    #[test]
    fn a_duplicate_json_key_fails_the_strict_parser() {
        let dir = scratch_copy("duplicate-key");
        let mut lines = graph_lines(&dir);
        lines[1] = lines[1].replacen('{', "{\"id\": \"urn:shadow\", ", 1);
        write_graph(&dir, &lines);
        reseal(&dir);
        assert_eq!(failure(&dir), Code::JsonDuplicateKey);
    }

    // E. An edge that references a node that does not exist.
    #[test]
    fn a_broken_graph_reference_fails_graph_verification() {
        let dir = scratch_copy("dangling-edge");
        let mut lines = graph_lines(&dir);
        let index = lines
            .iter()
            .position(|l| serde_json::from_str::<Value>(l).unwrap()["record_type"] == "edge")
            .expect("the shipped graph has edges");
        let mut edge: Value = serde_json::from_str(&lines[index]).unwrap();
        edge["to"] = Value::String("urn:moiric:nomos:research:2026-09-28:node:nonexistent".into());
        lines[index] = serde_json::to_string(&edge).unwrap();
        write_graph(&dir, &lines);
        reseal(&dir);
        assert_eq!(failure(&dir), Code::DanglingEndpoint);
    }

    #[test]
    fn a_node_citing_a_nonexistent_source_fails_graph_verification() {
        let dir = scratch_copy("unresolved-source");
        let mut lines = graph_lines(&dir);
        let mut node: Value = serde_json::from_str(&lines[1]).unwrap();
        node["source_ids"] =
            serde_json::json!(["urn:moiric:nomos:research:2026-09-28:src:nonexistent"]);
        lines[1] = serde_json::to_string(&node).unwrap();
        write_graph(&dir, &lines);
        reseal(&dir);
        assert_eq!(failure(&dir), Code::UnresolvedSource);
    }

    #[test]
    fn a_schema_violation_fails_the_schema_stage() {
        let dir = scratch_copy("schema");
        let mut lines = graph_lines(&dir);
        let mut node: Value = serde_json::from_str(&lines[1]).unwrap();
        node["as_of"] = Value::String("2026-13-45".into());
        lines[1] = serde_json::to_string(&node).unwrap();
        write_graph(&dir, &lines);
        reseal(&dir);
        assert_eq!(failure(&dir), Code::SchemaViolation);
    }

    #[test]
    fn an_empty_manifest_fails() {
        let dir = scratch_copy("empty-manifest");
        std::fs::write(dir.join("MANIFEST.sha256"), "").unwrap();
        assert_eq!(failure(&dir), Code::ManifestEmpty);
    }

    #[test]
    fn a_malformed_manifest_line_fails() {
        let dir = scratch_copy("malformed-manifest");
        append(&dir.join("MANIFEST.sha256"), b"not-a-digest  README.md\n");
        assert_eq!(failure(&dir), Code::ManifestMalformed);
    }

    #[test]
    fn a_manifest_path_outside_the_snapshot_fails() {
        let dir = scratch_copy("escaping-manifest");
        let digest = sha256_hex(b"");
        append(
            &dir.join("MANIFEST.sha256"),
            format!("{digest}  ../outside\n").as_bytes(),
        );
        assert_eq!(failure(&dir), Code::ManifestMalformed);
    }

    #[test]
    fn a_file_listed_twice_fails() {
        let dir = scratch_copy("duplicate-entry");
        let manifest = std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap();
        let readme = manifest
            .lines()
            .find(|l| l.ends_with("  README.md"))
            .unwrap()
            .to_owned();
        append(
            &dir.join("MANIFEST.sha256"),
            format!("{readme}\n").as_bytes(),
        );
        assert_eq!(failure(&dir), Code::ManifestDuplicateEntry);
    }

    #[test]
    fn a_manifest_naming_a_missing_file_fails() {
        let dir = scratch_copy("named-missing");
        std::fs::remove_file(dir.join("graph-stats.json")).unwrap();
        assert_eq!(failure(&dir), Code::ManifestNamesMissingFile);
    }

    #[test]
    fn an_extra_file_fails_coverage() {
        let dir = scratch_copy("extra-file");
        std::fs::write(dir.join("notes.md"), "not in the manifest\n").unwrap();
        assert_eq!(failure(&dir), Code::UncoveredFile);
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
        assert_eq!(failure(&dir), Code::RequiredFileUnlisted);
    }

    #[test]
    fn a_symlink_to_bytes_outside_the_snapshot_fails_inventory() {
        let dir = scratch_copy("symlink");
        std::os::unix::fs::symlink("../../../../README.md", dir.join("outside.md")).unwrap();
        assert_eq!(failure(&dir), Code::SnapshotSymlink);
    }

    #[test]
    fn a_manifested_file_replaced_by_a_symlink_fails_before_hashing() {
        let dir = scratch_copy("symlinked-entry");
        let readme = dir.join("README.md");
        let target = dir.join("README.real");
        std::fs::rename(&readme, &target).unwrap();
        std::os::unix::fs::symlink("README.real", &readme).unwrap();
        assert_eq!(failure(&dir), Code::SnapshotSymlink);
    }

    #[test]
    fn a_symlinked_manifest_fails_inventory() {
        let dir = scratch_copy("symlinked-manifest");
        let manifest = dir.join("MANIFEST.sha256");
        let elsewhere = std::env::temp_dir().join(format!(
            "nomos-xtask-{}-manifest-elsewhere",
            std::process::id()
        ));
        std::fs::rename(&manifest, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &manifest).unwrap();
        assert_eq!(failure(&dir), Code::SnapshotSymlink);
    }

    #[test]
    fn a_symlinked_snapshot_directory_fails_inventory() {
        let dir = scratch_copy("real-dir");
        let link =
            std::env::temp_dir().join(format!("nomos-xtask-{}-linked-dir", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert_eq!(failure(&link), Code::SnapshotSymlink);
    }

    #[test]
    fn a_nested_directory_fails_inventory_even_when_empty() {
        let dir = scratch_copy("empty-subdir");
        std::fs::create_dir(dir.join("extra")).unwrap();
        assert_eq!(failure(&dir), Code::SnapshotSubdirectory);
        let dir = scratch_copy("full-subdir");
        std::fs::create_dir(dir.join("extra")).unwrap();
        std::fs::write(dir.join("extra").join("hidden.json"), "{}\n").unwrap();
        assert_eq!(failure(&dir), Code::SnapshotSubdirectory);
    }

    #[test]
    fn a_socket_fails_inventory() {
        let dir = scratch_copy("socket");
        let _listener = std::os::unix::net::UnixListener::bind(dir.join("channel.sock")).unwrap();
        assert_eq!(failure(&dir), Code::SnapshotSpecialFile);
    }

    #[test]
    fn a_non_utf8_file_name_fails_inventory() {
        use std::os::unix::ffi::OsStrExt;
        let dir = scratch_copy("bad-name");
        let name = std::ffi::OsStr::from_bytes(b"graph-\xff.json");
        std::fs::write(dir.join(name), "{}\n").unwrap();
        assert_eq!(failure(&dir), Code::SnapshotNameInvalid);
    }

    #[test]
    fn removing_the_nested_directory_makes_the_snapshot_verify_again() {
        let dir = scratch_copy("subdir-restored");
        std::fs::create_dir(dir.join("extra")).unwrap();
        assert_eq!(failure(&dir), Code::SnapshotSubdirectory);
        std::fs::remove_dir(dir.join("extra")).unwrap();
        assert!(verify_snapshot(&dir).is_ok());
    }

    #[test]
    fn restoring_the_corrupted_file_makes_the_snapshot_verify_again() {
        let dir = scratch_copy("restored");
        let readme = dir.join("README.md");
        let original = std::fs::read(&readme).unwrap();
        append(&readme, b"\nedited\n");
        assert_eq!(failure(&dir), Code::ChecksumMismatch);
        std::fs::write(&readme, original).unwrap();
        assert!(verify_snapshot(&dir).is_ok());
    }

    /// Documented limit: a file and its checksum rewritten together pass the
    /// checksum stage. Freezing accepted snapshots is a Git comparison in CI.
    #[test]
    fn a_file_and_its_checksum_rewritten_together_pass_checksums() {
        let dir = scratch_copy("rewritten-pair");
        append(&dir.join("README.md"), b"\nedited\n");
        reseal(&dir);
        assert!(verify_snapshot(&dir).is_ok());
    }
}
