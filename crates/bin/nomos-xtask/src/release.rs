//! The admission of a release (verification-strategy.md, Release Admission).
//!
//! The checks of this repository live in the repository, so they cannot
//! authenticate it: when the tagged commit is the head of `main`, a check
//! whose base is `main` compares the commit with itself and passes whatever
//! the commit holds. Two commands close that.
//!
//! - `check-release-base --base <tag> [--main <ref>]` takes the trusted base
//!   of a release, which is the previous release tag, and refuses, with a
//!   stable code, a base that cannot be resolved, is not a release tag, is
//!   the tagged commit itself, or is not its ancestor, and a tagged commit
//!   that `main` does not contain.
//! - `check-release-chain --package <deb> --record <file>... --tag <tag>
//!   --commit <sha> --out <file>` ties the release to the artifact it
//!   publishes: every Debian suite named in `RELEASES` ran that exact
//!   package, and passed. It writes the evidence file the release attaches.
//!
//! Both fail closed: a failure to read, resolve, or parse is an error, never
//! a pass.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Why a release is refused. The kebab-case form is the stable identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Refusal {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.detail)
    }
}

fn refuse(code: &'static str, detail: impl Into<String>) -> Refusal {
    Refusal {
        code,
        detail: detail.into(),
    }
}

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git {args:?}: {e}"))
}

/// The commit `rev` names, if it names one.
fn commit_of(root: &Path, rev: &str) -> Result<Option<String>, String> {
    let out = git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )?;
    Ok(out
        .status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()))
}

fn is_ancestor(root: &Path, ancestor: &str, of: &str) -> Result<bool, String> {
    Ok(git(root, &["merge-base", "--is-ancestor", ancestor, of])?
        .status
        .success())
}

/// What `check-release-base` found.
#[derive(Debug, Serialize)]
pub(crate) struct BaseReport {
    pub(crate) head: String,
    pub(crate) base: String,
    pub(crate) base_commit: Option<String>,
    pub(crate) main: String,
    /// How many commits the release adds to its base.
    pub(crate) commits_since_base: Option<usize>,
    pub(crate) refusals: Vec<Refusal>,
}

/// Takes `base`, the previous release tag, as the trusted base of the
/// release at `HEAD`, which `main` must contain.
pub(crate) fn check_base(root: &Path, base: &str, main: &str) -> Result<BaseReport, String> {
    let head = commit_of(root, "HEAD")?.ok_or("HEAD names no commit")?;
    let mut refusals = Vec::new();
    let mut commits_since_base = None;

    let base_commit = commit_of(root, &format!("refs/tags/{base}"))?;
    match &base_commit {
        None if commit_of(root, base)?.is_some() => refusals.push(refuse(
            "release-base-not-a-tag",
            format!("{base} is not a release tag: a release is checked against the previous release, never a branch"),
        )),
        None => refusals.push(refuse(
            "release-base-unresolved",
            format!("{base} cannot be resolved, so the release has no trusted base"),
        )),
        Some(commit) => {
            if !base.starts_with('v') {
                refusals.push(refuse(
                    "release-base-not-a-tag",
                    format!("{base} is a tag, but not a release tag `v<version>`"),
                ));
            }
            if *commit == head {
                refusals.push(refuse(
                    "release-base-is-head",
                    format!("{base} is the tagged commit {head}: the release would be checked against itself"),
                ));
            } else if !is_ancestor(root, commit, &head)? {
                refusals.push(refuse(
                    "release-base-not-ancestor",
                    format!("{base} is not an ancestor of {head}, so its commits say nothing of this release"),
                ));
            } else {
                let range = format!("{commit}..{head}");
                let counted = git(root, &["rev-list", "--count", &range])?;
                commits_since_base = String::from_utf8_lossy(&counted.stdout)
                    .trim()
                    .parse()
                    .ok();
            }
        }
    }

    match commit_of(root, main)? {
        None => refusals.push(refuse(
            "release-not-on-main",
            format!("{main} cannot be resolved, so the release cannot be shown to be on it"),
        )),
        Some(tip) => {
            if !is_ancestor(root, &head, &tip)? {
                refusals.push(refuse(
                    "release-not-on-main",
                    format!("{head} is not reachable from {main}: only a commit main accepted may be released"),
                ));
            }
        }
    }
    refusals.sort();
    Ok(BaseReport {
        head,
        base: base.to_owned(),
        base_commit,
        main: main.to_owned(),
        commits_since_base,
        refusals,
    })
}

// ---------------------------------------------------------------------------
// The chain from the package to the published asset

/// What a Debian record says of the package it ran.
#[derive(Debug, Deserialize)]
struct RecordedPackage {
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct RecordedRun {
    test: String,
    exit_status: i32,
    failed: u64,
}

/// The parts of a `cargo xtask debian` record the chain reads.
#[derive(Debug, Deserialize)]
struct Recorded {
    release: String,
    os_release: String,
    package: Option<RecordedPackage>,
    runs: Vec<RecordedRun>,
}

/// One suite's part of the evidence.
#[derive(Debug, Serialize)]
pub(crate) struct Suite {
    pub(crate) release: String,
    pub(crate) os_release: String,
    pub(crate) record: String,
    pub(crate) package_sha256: Option<String>,
    pub(crate) install_test_passed: bool,
}

/// The package the release publishes.
#[derive(Debug, Serialize)]
pub(crate) struct Published {
    pub(crate) file: String,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
}

/// The evidence a release attaches, and what `check-release-chain` found.
#[derive(Debug, Serialize)]
pub(crate) struct ChainReport {
    pub(crate) tag: String,
    pub(crate) commit: String,
    pub(crate) package: Published,
    pub(crate) suites: Vec<Suite>,
    pub(crate) refusals: Vec<Refusal>,
}

/// The tests that install the package on Debian: on the standard host, and
/// on the minimal host.
const INSTALL_TESTS: [&str; 2] = ["package_install", "minimal_install"];

/// Checks that the package at `package` is the one every suite of
/// `releases` ran and passed, from the records at `records`.
pub(crate) fn check_chain(
    package: &Path,
    records: &[PathBuf],
    tag: &str,
    commit: &str,
    releases: &[&str],
) -> Result<ChainReport, String> {
    let sha256 = crate::package::sha256(package)?;
    let bytes = std::fs::metadata(package)
        .map_err(|e| format!("{}: {e}", package.display()))?
        .len();
    let mut refusals = Vec::new();
    let mut suites = Vec::new();
    for path in records {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let recorded: Recorded = serde_json::from_str(&text)
            .map_err(|e| format!("{}: not a Debian record: {e}", path.display()))?;
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let install_test_passed = recorded.runs.iter().any(|r| {
            INSTALL_TESTS.contains(&r.test.as_str()) && r.exit_status == 0 && r.failed == 0
        });
        match &recorded.package {
            None => refusals.push(refuse(
                "release-suite-did-not-run-package",
                format!(
                    "{name} (Debian {}) records no package: that suite built its own, and says nothing of this file",
                    recorded.release
                ),
            )),
            Some(p) if p.sha256 != sha256 => refusals.push(refuse(
                "release-package-digest-mismatch",
                format!(
                    "{name} (Debian {}) ran sha256 {}, and the release publishes {sha256}",
                    recorded.release, p.sha256
                ),
            )),
            Some(_) => {}
        }
        let failing_run = recorded
            .runs
            .iter()
            .any(|r| r.exit_status != 0 || r.failed != 0);
        if failing_run || !install_test_passed {
            refusals.push(refuse(
                "release-suite-failed",
                format!(
                    "{name} (Debian {}) has {}",
                    recorded.release,
                    if failing_run {
                        "a failing run".to_owned()
                    } else {
                        format!("no passing {} run", INSTALL_TESTS.join(" or "))
                    }
                ),
            ));
        }
        suites.push(Suite {
            release: recorded.release,
            os_release: recorded.os_release,
            record: name,
            package_sha256: recorded.package.map(|p| p.sha256),
            install_test_passed,
        });
    }
    for release in releases {
        if !suites.iter().any(|s| s.release == *release) {
            refusals.push(refuse(
                "release-suite-missing",
                format!("no record of a Debian {release} suite for this package"),
            ));
        }
    }
    refusals.sort();
    refusals.dedup();
    suites.sort_by(|a, b| a.release.cmp(&b.release));
    Ok(ChainReport {
        tag: tag.to_owned(),
        commit: commit.to_owned(),
        package: Published {
            file: package
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            sha256,
            bytes,
        },
        suites,
        refusals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn run(root: &Path, args: &[&str]) -> String {
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
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn commit(root: &Path, file: &str, text: &str, message: &str) {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-q", "-m", message]);
    }

    /// `main` with a release `v0.9.0` on its first commit, and a second
    /// commit after it that changes the verifier without declaring it.
    fn history(label: &str) -> PathBuf {
        let root = crate::scratch::dir("release", label);
        run(&root, &["init", "-q", "-b", "main"]);
        let policy =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../verification/trust-boundary.toml");
        fs::create_dir_all(root.join("verification")).unwrap();
        // The repository's policy, without its `since` table: the commits it
        // names belong to this repository's history, not to this one.
        let mut keep = true;
        let text: String = fs::read_to_string(&policy)
            .unwrap()
            .lines()
            .filter(|line| {
                if line.trim() == "[since]" {
                    keep = false;
                } else if line.trim().is_empty() || line.starts_with('[') {
                    keep = true;
                }
                keep
            })
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(root.join("verification/trust-boundary.toml"), text).unwrap();
        commit(&root, "docs/a.md", "a\n", "A release");
        run(&root, &["tag", "v0.9.0"]);
        commit(
            &root,
            ".github/workflows/ci.yml",
            "name: weakened\n",
            "Change a workflow and say nothing",
        );
        root
    }

    fn codes(report: &BaseReport) -> Vec<&'static str> {
        report.refusals.iter().map(|r| r.code).collect()
    }

    /// The release of the previous tag, on main's lineage: accepted, with the
    /// commits it adds counted.
    #[test]
    fn a_release_after_the_previous_release_has_a_trusted_base() {
        let root = history("ok");
        run(&root, &["tag", "v1.0.0"]);
        let report = check_base(&root, "v0.9.0", "main").unwrap();
        assert_eq!(codes(&report), Vec::<&str>::new(), "{report:#?}");
        assert_eq!(report.commits_since_base, Some(1));
    }

    /// Negative control: the base is the branch the tag is on. Alpha 1's
    /// trust check ran with `origin/main` and inspected no commit.
    #[test]
    fn a_branch_is_not_a_trusted_base() {
        let root = history("branch");
        run(&root, &["tag", "v1.0.0"]);
        let report = check_base(&root, "main", "main").unwrap();
        assert_eq!(
            codes(&report),
            vec!["release-base-not-a-tag"],
            "{report:#?}"
        );
    }

    /// Negative control: the release is checked against itself.
    #[test]
    fn a_release_is_not_checked_against_itself() {
        let root = history("self");
        run(&root, &["tag", "v1.0.0"]);
        let report = check_base(&root, "v1.0.0", "main").unwrap();
        assert_eq!(codes(&report), vec!["release-base-is-head"], "{report:#?}");
    }

    /// Negative control: a base that cannot be resolved fails closed.
    #[test]
    fn an_unresolvable_base_is_refused() {
        let root = history("unresolved");
        let report = check_base(&root, "v9.9.9", "main").unwrap();
        assert_eq!(
            codes(&report),
            vec!["release-base-unresolved"],
            "{report:#?}"
        );
        let report = check_base(&root, "v0.9.0", "origin/main").unwrap();
        assert_eq!(codes(&report), vec!["release-not-on-main"], "{report:#?}");
    }

    /// Negative control: a tag on a revision main never accepted.
    #[test]
    fn a_release_main_does_not_contain_is_refused() {
        let root = history("off-main");
        run(&root, &["checkout", "-q", "-b", "side", "v0.9.0"]);
        commit(
            &root,
            "docs/side.md",
            "side\n",
            "A commit main never accepted",
        );
        run(&root, &["tag", "v1.0.0"]);
        let report = check_base(&root, "v0.9.0", "main").unwrap();
        assert_eq!(codes(&report), vec!["release-not-on-main"], "{report:#?}");
    }

    /// Negative control: a base from another line of history.
    #[test]
    fn a_base_that_is_not_an_ancestor_is_refused() {
        let root = history("not-ancestor");
        run(&root, &["checkout", "-q", "-b", "other", "v0.9.0"]);
        commit(&root, "docs/other.md", "other\n", "Another line");
        run(&root, &["tag", "v0.9.1"]);
        run(&root, &["checkout", "-q", "main"]);
        run(&root, &["tag", "v1.0.0"]);
        let report = check_base(&root, "v0.9.1", "main").unwrap();
        assert_eq!(
            codes(&report),
            vec!["release-base-not-ancestor"],
            "{report:#?}"
        );
    }

    /// The whole of #31: an undeclared verifier change is rejected on the
    /// admission path, and tagging the commit that carries it cannot turn
    /// that rejection into a passing release gate.
    #[test]
    fn tagging_a_rejected_commit_does_not_pass_the_release_gate() {
        let root = history("rejected");
        // Admission, as CI runs it for a push to main: against the commit
        // before the push. It is rejected.
        let admitted = crate::trust::check(&root, "HEAD~1", false).unwrap();
        assert!(
            !admitted.violations().is_empty(),
            "the undeclared verifier change must be rejected on its way in"
        );
        // The release of that same commit, as alpha 1's gate ran it: against
        // main, which is the commit itself. It inspects nothing, and passes.
        run(&root, &["tag", "v1.0.0"]);
        let self_compared = crate::trust::check(&root, "main", false).unwrap();
        assert_eq!(self_compared.commits_checked(), 0);
        assert!(self_compared.violations().is_empty());
        // The release gate of alpha 2 refuses that base outright...
        let refused = check_base(&root, "main", "main").unwrap();
        assert_eq!(codes(&refused), vec!["release-base-not-a-tag"]);
        // ...and with the previous release as the base it inspects the
        // rejected commit, and rejects it again.
        let report = check_base(&root, "v0.9.0", "main").unwrap();
        assert!(report.refusals.is_empty(), "{report:#?}");
        let released = crate::trust::check(&root, "v0.9.0", true).unwrap();
        assert_eq!(released.commits_checked(), 1);
        assert!(!released.violations().is_empty());
    }

    /// The rulesets the release process asks the owner to import require
    /// jobs the workflow defines, so the two cannot drift apart unseen.
    #[test]
    fn the_rulesets_require_the_jobs_ci_defines() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
        let ruleset: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join(".github/rulesets/main.json")).unwrap(),
        )
        .unwrap();
        let rules = ruleset["rules"].as_array().unwrap();
        let checks = rules
            .iter()
            .find(|r| r["type"] == "required_status_checks")
            .expect("main requires status checks")["parameters"]["required_status_checks"]
            .as_array()
            .unwrap();
        assert!(checks.len() >= 5);
        for check in checks {
            let context = check["context"].as_str().unwrap();
            let job = context.split(" (").next().unwrap();
            assert!(
                ci.contains(&format!("\n  {job}:\n")),
                "the ruleset requires {context}, and ci.yml defines no job {job}"
            );
        }
        assert!(
            rules.iter().any(|r| r["type"] == "non_fast_forward"),
            "main refuses force pushes"
        );
        let tags: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join(".github/rulesets/release-tags.json")).unwrap(),
        )
        .unwrap();
        let kinds: Vec<&str> = tags["rules"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["type"].as_str())
            .collect();
        for kind in ["creation", "update", "deletion", "non_fast_forward"] {
            assert!(kinds.contains(&kind), "release tags: {kind}");
        }
    }

    /// The security documents describe the release line the workspace is on:
    /// the policy does not say there are no releases, the model is written,
    /// and the index does not call it unwritten.
    #[test]
    fn the_security_documents_are_not_stale() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let version = crate::package::workspace_version(&root).unwrap();
        let line: Vec<&str> = version.split('.').take(2).collect();
        let line = line.join(".");
        let policy = fs::read_to_string(root.join("SECURITY.md")).unwrap();
        assert!(
            !policy.contains("no releases yet"),
            "SECURITY.md still says there are no releases"
        );
        assert!(
            policy.contains(&format!("`{line}`")),
            "SECURITY.md does not name the {line} line the workspace is on ({version})"
        );
        let model = fs::read_to_string(root.join("docs/security-model.md")).unwrap();
        assert!(
            !model.contains("Not written yet"),
            "the security model is a placeholder"
        );
        for heading in [
            "## Assets",
            "## Principals",
            "## Trust Boundaries",
            "## Non-Goals",
        ] {
            assert!(
                model.contains(heading),
                "the security model has no {heading}"
            );
        }
        let index = fs::read_to_string(root.join("docs/README.md")).unwrap();
        assert!(
            !index.contains("security-model.md](security-model.md) | Security model and hardening (not yet written)"),
            "the docs index calls the security model unwritten"
        );
        // Every document the model cites for a claim exists.
        for link in [
            "formal/cell-commands.md",
            "adr/0017-local-store.md",
            "release-process.md",
            "plans/results/alpha2-state-ownership.md",
        ] {
            assert!(root.join("docs").join(link).exists(), "{link}");
        }
    }

    // -----------------------------------------------------------------
    // The chain

    fn record(
        dir: &Path,
        name: &str,
        release: &str,
        digest: Option<&str>,
        passed: bool,
    ) -> PathBuf {
        let package = digest.map_or(String::from("null"), |d| {
            format!(r#"{{"file":"nomos-cell.deb","sha256":"{d}","upgrade_sha256":"u"}}"#)
        });
        let (status, failed) = if passed { (0, 0) } else { (101, 1) };
        let text = format!(
            r#"{{"experiment":"linux-families","release":"{release}","image":"i","os_release":"Debian {release}","kernel":"k","systemd":"running","package":{package},"runs":[{{"package":"nomos-cell","test":"package_install","exit_status":{status},"passed":1,"failed":{failed},"stdout_sha256":"s"}}]}}"#
        );
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        path
    }

    fn deb(dir: &Path, bytes: &[u8]) -> (PathBuf, String) {
        let path = dir.join("nomos-cell_1.0.0_amd64.deb");
        fs::write(&path, bytes).unwrap();
        let digest = crate::package::sha256(&path).unwrap();
        (path, digest)
    }

    fn chain(records: &[PathBuf], package: &Path) -> Vec<&'static str> {
        let report = check_chain(package, records, "v1.0.0", "abc", &["12", "13"]).unwrap();
        report.refusals.iter().map(|r| r.code).collect()
    }

    #[test]
    fn a_package_both_suites_ran_is_the_one_published() {
        let dir = crate::scratch::dir("release", "chain-ok");
        let (package, digest) = deb(&dir, b"the package");
        let records = [
            record(&dir, "d12.json", "12", Some(&digest), true),
            record(&dir, "d13.json", "13", Some(&digest), true),
        ];
        assert_eq!(chain(&records, &package), Vec::<&str>::new());
        let report = check_chain(&package, &records, "v1.0.0", "abc", &["12", "13"]).unwrap();
        assert_eq!(report.package.sha256, digest);
        assert_eq!(report.suites.len(), 2);
    }

    /// Negative control: the package was rebuilt between validation and
    /// publication, so the published bytes are not the validated bytes.
    #[test]
    fn a_package_replaced_after_validation_is_refused() {
        let dir = crate::scratch::dir("release", "chain-replaced");
        let (_, validated) = deb(&dir, b"the package that passed");
        let records = [
            record(&dir, "d12.json", "12", Some(&validated), true),
            record(&dir, "d13.json", "13", Some(&validated), true),
        ];
        let (published, _) = deb(&dir, b"a rebuilt package");
        assert_eq!(
            chain(&records, &published),
            vec![
                "release-package-digest-mismatch",
                "release-package-digest-mismatch"
            ]
        );
    }

    /// Negative control: a suite that built its own package.
    #[test]
    fn a_suite_that_did_not_run_the_package_is_refused() {
        let dir = crate::scratch::dir("release", "chain-own");
        let (package, digest) = deb(&dir, b"the package");
        let records = [
            record(&dir, "d12.json", "12", Some(&digest), true),
            record(&dir, "d13.json", "13", None, true),
        ];
        assert_eq!(
            chain(&records, &package),
            vec!["release-suite-did-not-run-package"]
        );
    }

    #[test]
    fn a_missing_or_failed_suite_is_refused() {
        let dir = crate::scratch::dir("release", "chain-missing");
        let (package, digest) = deb(&dir, b"the package");
        let only12 = [record(&dir, "d12.json", "12", Some(&digest), true)];
        assert_eq!(chain(&only12, &package), vec!["release-suite-missing"]);
        let failed = [
            record(&dir, "d12.json", "12", Some(&digest), true),
            record(&dir, "d13.json", "13", Some(&digest), false),
        ];
        assert_eq!(chain(&failed, &package), vec!["release-suite-failed"]);
    }

    #[test]
    fn a_record_that_is_not_one_is_an_error_not_a_pass() {
        let dir = crate::scratch::dir("release", "chain-garbage");
        let (package, _) = deb(&dir, b"the package");
        let garbage = dir.join("garbage.json");
        fs::write(&garbage, "{}").unwrap();
        assert!(check_chain(&package, &[garbage], "v1", "abc", &["12"]).is_err());
        assert!(check_chain(&package, &[dir.join("absent.json")], "v1", "abc", &["12"]).is_err());
    }
}
