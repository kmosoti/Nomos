//! The snapshot freeze: accepted research snapshots never change.
//!
//! Matching checksums show a snapshot is consistent with its manifest. They do
//! not show the manifest is the one that was accepted, because a file and its
//! checksum can be rewritten together. The freeze compares the tree against
//! the merge base with an accepted branch: inside any `docs/research/*/snapshot/`
//! directory that already existed there, no file may be added, modified, or
//! deleted. A snapshot directory that did not exist at the base is new and may
//! be added whole.

use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<(bool, String), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

/// Returns the frozen-snapshot violations between the merge base with `base` and `HEAD`.
pub(crate) fn frozen(root: &Path, base: &str) -> Result<Vec<String>, String> {
    let (ok, merge_base) = git(root, &["merge-base", base, "HEAD"])?;
    if !ok {
        return Err(format!(
            "git merge-base {base} HEAD failed; fetch the base branch first"
        ));
    }
    let merge_base = merge_base.trim().to_owned();
    let (ok, diff) = git(
        root,
        &[
            "diff",
            "--name-status",
            "--no-renames",
            &merge_base,
            "HEAD",
            "--",
            "docs/research",
        ],
    )?;
    if !ok {
        return Err("git diff failed".into());
    }
    let mut violations = Vec::new();
    for line in diff.lines() {
        let Some((status, path)) = line.split_once('\t') else {
            continue;
        };
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 4 || parts[0] != "docs" || parts[1] != "research" || parts[3] != "snapshot"
        {
            continue;
        }
        let snapshot_dir = parts[..4].join("/");
        let (existed, _) = git(
            root,
            &["cat-file", "-e", &format!("{merge_base}:{snapshot_dir}")],
        )?;
        match (status, existed) {
            ("A", false) => {}
            ("A", true) => violations.push(format!(
                "{path}: added to the frozen snapshot {snapshot_dir}"
            )),
            ("M", _) => violations.push(format!(
                "{path}: modified in the frozen snapshot {snapshot_dir}"
            )),
            ("D", _) => violations.push(format!(
                "{path}: deleted from the frozen snapshot {snapshot_dir}"
            )),
            (other, _) => violations.push(format!(
                "{path}: {other} in the frozen snapshot {snapshot_dir}"
            )),
        }
    }
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::frozen;

    fn sh(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A repository with one accepted snapshot on `main` and a working branch.
    fn repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("nomos-freeze-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        sh(&root, &["init", "-q", "-b", "main"]);
        write(
            &root,
            "docs/research/2026-01-01-a/snapshot/graph.ndjson",
            "{}\n",
        );
        write(&root, "docs/research/2026-01-01-a/README.md", "eval\n");
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-q", "-m", "accept snapshot a"]);
        sh(&root, &["checkout", "-q", "-b", "work"]);
        root
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
        assert!(frozen(&root, "main").unwrap().is_empty());
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
        assert!(frozen(&root, "main").unwrap().is_empty());
    }

    #[test]
    fn a_file_rewritten_inside_an_accepted_snapshot_is_a_violation() {
        let root = repo("rewrite");
        write(
            &root,
            "docs/research/2026-01-01-a/snapshot/graph.ndjson",
            "{\"x\":1}\n",
        );
        sh(&root, &["commit", "-q", "-am", "rewrite evidence"]);
        let v = frozen(&root, "main").unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("modified in the frozen snapshot"));
    }

    #[test]
    fn additions_and_deletions_inside_an_accepted_snapshot_are_violations() {
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
        let v = frozen(&root, "main").unwrap();
        assert_eq!(v.len(), 2, "{v:?}");
    }
}
