//! `check-workflow-pins`: every third-party action a workflow uses is pinned
//! to a full commit (verification-strategy.md, Release Admission).
//!
//! A tag such as `v4`, or a branch such as `master`, moves: the code a
//! release job runs, with `contents: write`, would be whatever the action's
//! author last pushed. A full commit SHA does not move. The check reads every
//! `.github/workflows/*.yml` and refuses a `uses:` that names a remote action
//! by anything else. A local action (`./path`) and a container image pinned by
//! digest are allowed.

use std::path::Path;

/// A `uses:` that is not pinned.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Unpinned {
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) uses: String,
}

impl std::fmt::Display for Unpinned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[workflow-action-unpinned] {}:{}: {} is not pinned to a full commit SHA",
            self.file, self.line, self.uses
        )
    }
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether a `uses:` value names its action by an immutable reference.
pub(crate) fn pinned(uses: &str) -> bool {
    if uses.starts_with("./") {
        return true;
    }
    if let Some(image) = uses.strip_prefix("docker://") {
        return image
            .split_once("@sha256:")
            .is_some_and(|(_, digest)| is_hex(digest, 64));
    }
    uses.split_once('@')
        .is_some_and(|(action, reference)| action.contains('/') && is_hex(reference, 40))
}

/// The `uses:` values of a workflow, with their line numbers.
fn uses_of(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let rest = line.trim_start().trim_start_matches("- ").trim_start();
            let value = rest.strip_prefix("uses:")?;
            let value = value.split(" #").next().unwrap_or(value);
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            Some((i + 1, value.to_owned()))
        })
        .collect()
}

/// Every unpinned action in the workflows under `dir`.
pub(crate) fn check(dir: &Path) -> Result<Vec<Unpinned>, String> {
    let mut found = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml"))
        .collect();
    files.sort();
    for path in files {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        for (line, uses) in uses_of(&text) {
            if !pinned(&uses) {
                found.push(Unpinned {
                    file: path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    line,
                    uses,
                });
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "11d5960a326750d5838078e36cf38b85af677262";

    #[test]
    fn only_a_full_commit_sha_is_a_pin() {
        assert!(pinned(&format!("actions/checkout@{SHA}")));
        assert!(pinned(&format!("owner/repo/sub/path@{SHA}")));
        assert!(pinned("./local-action"));
        assert!(pinned(&format!(
            "docker://alpine@sha256:{}",
            "a".repeat(64)
        )));
        for loose in [
            "actions/checkout@v4",
            "dtolnay/rust-toolchain@master",
            "actions/checkout@11d5960",
            &format!("actions/checkout@{}", SHA.to_uppercase()),
            &format!("actions/checkout@{SHA}0"),
            "actions/checkout",
            "docker://alpine:3.20",
            &format!("checkout@{SHA}"),
        ] {
            assert!(!pinned(loose), "{loose}");
        }
    }

    #[test]
    fn a_workflow_is_read_for_its_uses_lines() {
        let text = format!(
            "jobs:\n  a:\n    steps:\n      - uses: actions/checkout@{SHA} # v4\n      - uses: \"actions/upload-artifact@v4\"\n      - name: x\n        uses: ./local\n      - run: echo uses: not-an-action@v1\n"
        );
        let found = uses_of(&text);
        assert_eq!(
            found,
            vec![
                (4, format!("actions/checkout@{SHA}")),
                (5, "actions/upload-artifact@v4".to_owned()),
                (7, "./local".to_owned())
            ]
        );
    }

    /// Negative control: a workflow with a floating action is refused.
    #[test]
    fn a_floating_action_is_reported_with_its_line() {
        let dir = crate::scratch::dir("pins", "floating");
        std::fs::write(
            dir.join("ci.yml"),
            format!("steps:\n  - uses: actions/checkout@{SHA}\n  - uses: softprops/action-gh-release@v2\n"),
        )
        .unwrap();
        let found = check(&dir).unwrap();
        assert_eq!(
            found,
            vec![Unpinned {
                file: "ci.yml".into(),
                line: 3,
                uses: "softprops/action-gh-release@v2".into()
            }]
        );
        assert!(found[0].to_string().contains("ci.yml:3"));
    }

    /// The repository's own workflows pin every action.
    #[test]
    fn the_repository_workflows_pin_every_action() {
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../.github/workflows");
        let found = check(&dir).unwrap();
        assert!(
            found.is_empty(),
            "{}",
            found
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
