//! The snapshot freeze: accepted research snapshots never change.
//!
//! Matching checksums show a snapshot is consistent with its manifest. They do
//! not show the manifest is the one that was accepted, because a file and its
//! checksum can be rewritten together. The freeze compares `HEAD` against an
//! accepted revision: inside any `docs/research/*/snapshot/` directory that
//! already existed there, no file may be added, modified, deleted, or change
//! type. A snapshot directory that did not exist there is new and may be added
//! whole.
//!
//! Policy violations carry a stable [`FreezeCode`]. Operational failures, such
//! as a base revision that is not available, are errors: the gate fails closed
//! and never reports an unchecked tree as unchanged.

use std::fmt;
use std::path::Path;
use std::process::Command;

/// Why a change to research evidence is rejected. The string form is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FreezeCode {
    /// A file in an accepted snapshot changed content.
    Modified,
    /// A file was added to an accepted snapshot.
    AddedTo,
    /// A file was removed from an accepted snapshot.
    DeletedFrom,
    /// A file in an accepted snapshot changed type, for example into a symlink.
    TypeChanged,
}

impl FreezeCode {
    /// The stable identifier.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FreezeCode::Modified => "accepted-snapshot-modified",
            FreezeCode::AddedTo => "accepted-snapshot-added-to",
            FreezeCode::DeletedFrom => "accepted-snapshot-deleted-from",
            FreezeCode::TypeChanged => "accepted-snapshot-type-changed",
        }
    }
}

/// One change to an accepted snapshot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FreezeViolation {
    code: FreezeCode,
    path: String,
    snapshot: String,
}

impl FreezeViolation {
    /// The stable reason.
    #[cfg(test)]
    pub(crate) fn code(&self) -> FreezeCode {
        self.code
    }

    /// The changed path.
    #[cfg(test)]
    pub(crate) fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Display for FreezeViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} in accepted snapshot {}",
            self.code.as_str(),
            self.path,
            self.snapshot
        )
    }
}

fn git(root: &Path, args: &[&str]) -> Result<(bool, Vec<u8>), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    Ok((out.status.success(), out.stdout))
}

/// The snapshot directory a path belongs to, if it is inside one.
fn snapshot_dir(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (parts.len() >= 5 && parts[0] == "docs" && parts[1] == "research" && parts[3] == "snapshot")
        .then(|| parts[..4].join("/"))
}

/// Returns every change to an accepted snapshot between the merge base of
/// `base` and `HEAD`, and `HEAD`.
///
/// For a pull request `base` is the base branch. For a push to the default
/// branch it is the previous head, so a direct push that rewrites a file and
/// its manifest together is caught. A base that cannot be resolved is an
/// error, not an empty result.
pub(crate) fn frozen(root: &Path, base: &str) -> Result<Vec<FreezeViolation>, String> {
    let (ok, merge_base) = git(root, &["merge-base", base, "HEAD"])?;
    if !ok {
        return Err(format!(
            "git merge-base {base} HEAD failed; the base revision is not available, so the freeze cannot be checked"
        ));
    }
    let merge_base = String::from_utf8_lossy(&merge_base).trim().to_owned();
    let (ok, diff) = git(
        root,
        &[
            "diff",
            "--name-status",
            "--no-renames",
            "-z",
            &merge_base,
            "HEAD",
            "--",
            "docs/research",
        ],
    )?;
    if !ok {
        return Err(format!("git diff {merge_base} HEAD failed"));
    }
    // With -z, each change is a status field and a path field, each ending in NUL.
    let fields: Vec<String> = diff
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect();
    if !fields.len().is_multiple_of(2) {
        return Err("git diff output did not pair statuses with paths".into());
    }
    let mut violations = Vec::new();
    for pair in fields.chunks(2) {
        let (status, path) = (pair[0].as_str(), pair[1].as_str());
        let Some(snapshot) = snapshot_dir(path) else {
            continue;
        };
        let (existed, _) = git(
            root,
            &["cat-file", "-e", &format!("{merge_base}:{snapshot}")],
        )?;
        let code = match (status, existed) {
            ("A", false) => continue,
            ("A", true) => FreezeCode::AddedTo,
            ("D", _) => FreezeCode::DeletedFrom,
            ("T", _) => FreezeCode::TypeChanged,
            _ => FreezeCode::Modified,
        };
        violations.push(FreezeViolation {
            code,
            path: path.to_owned(),
            snapshot,
        });
    }
    violations.sort();
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{FreezeCode, frozen};

    fn git_out(root: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn sh(root: &Path, args: &[&str]) {
        git_out(root, args);
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A repository with one accepted snapshot on `main` and a working branch.
    fn repo(name: &str) -> PathBuf {
        let root = crate::scratch::dir("freeze", name);
        sh(&root, &["init", "-q", "-b", "main"]);
        write(
            &root,
            "docs/research/2026-01-01-a/snapshot/graph.ndjson",
            "{}\n",
        );
        write(
            &root,
            "docs/research/2026-01-01-a/snapshot/MANIFEST.sha256",
            "x  graph.ndjson\n",
        );
        write(&root, "docs/research/2026-01-01-a/README.md", "eval\n");
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-q", "-m", "accept snapshot a"]);
        sh(&root, &["checkout", "-q", "-b", "work"]);
        root
    }

    fn codes(root: &Path, base: &str) -> Vec<FreezeCode> {
        frozen(root, base)
            .unwrap()
            .iter()
            .map(|v| v.code())
            .collect()
    }

    #[test]
    fn an_untouched_snapshot_has_no_violations() {
        let root = repo("clean");
        write(
            &root,
            "docs/research/2026-01-01-a/README.md",
            "eval, revised\n",
        );
        sh(&root, &["commit", "-q", "-am", "revise evaluation"]);
        assert!(codes(&root, "main").is_empty());
    }

    #[test]
    fn a_new_snapshot_directory_is_allowed() {
        let root = repo("new");
        write(
            &root,
            "docs/research/2026-02-02-b/snapshot/graph.ndjson",
            "{}\n",
        );
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-q", "-m", "import snapshot b"]);
        assert!(codes(&root, "main").is_empty());
    }

    #[test]
    fn a_file_rewritten_inside_an_accepted_snapshot_is_modified() {
        let root = repo("rewrite");
        write(
            &root,
            "docs/research/2026-01-01-a/snapshot/graph.ndjson",
            "{\"x\":1}\n",
        );
        sh(&root, &["commit", "-q", "-am", "rewrite evidence"]);
        assert_eq!(codes(&root, "main"), [FreezeCode::Modified]);
    }

    #[test]
    fn an_addition_and_a_deletion_are_each_named() {
        let root = repo("add-delete");
        write(
            &root,
            "docs/research/2026-01-01-a/snapshot/extra.json",
            "{}\n",
        );
        std::fs::remove_file(root.join("docs/research/2026-01-01-a/snapshot/graph.ndjson"))
            .unwrap();
        sh(&root, &["add", "-A"]);
        sh(&root, &["commit", "-q", "-m", "tamper"]);
        assert_eq!(
            codes(&root, "main"),
            [FreezeCode::AddedTo, FreezeCode::DeletedFrom]
        );
    }

    #[test]
    fn a_file_replaced_by_a_symlink_is_a_type_change() {
        let root = repo("type-change");
        let file = root.join("docs/research/2026-01-01-a/snapshot/graph.ndjson");
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink("../README.md", &file).unwrap();
        sh(&root, &["add", "-A"]);
        sh(&root, &["commit", "-q", "-m", "swap in a symlink"]);
        assert_eq!(codes(&root, "main"), [FreezeCode::TypeChanged]);
    }

    /// The checksum stage passes a file and its manifest rewritten together.
    /// A direct push to the default branch is compared with its previous head,
    /// passed as a revision, and this is where that rewrite is caught.
    #[test]
    fn a_direct_push_rewriting_a_file_and_its_manifest_is_caught_against_the_previous_head() {
        let root = repo("direct-push");
        sh(&root, &["checkout", "-q", "main"]);
        let before = git_out(&root, &["rev-parse", "HEAD"]);
        write(
            &root,
            "docs/research/2026-01-01-a/snapshot/graph.ndjson",
            "{\"forged\":true}\n",
        );
        write(
            &root,
            "docs/research/2026-01-01-a/snapshot/MANIFEST.sha256",
            "y  graph.ndjson\n",
        );
        sh(
            &root,
            &[
                "commit",
                "-q",
                "-am",
                "rewrite evidence and its checksum together",
            ],
        );
        let found = frozen(&root, &before).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|v| v.code() == FreezeCode::Modified));
        assert!(found.iter().any(|v| v.path().ends_with("MANIFEST.sha256")));
    }

    #[test]
    fn an_unavailable_base_fails_closed() {
        let root = repo("no-base");
        assert!(frozen(&root, "0000000000000000000000000000000000000000").is_err());
        assert!(frozen(&root, "origin/does-not-exist").is_err());
    }
}
