//! The trust-boundary check (ADR 0015): oracle changes are declared.
//!
//! A generator writes implementation; the specification and the verifier
//! judge it. AGENTS.md rule 11 says a change to either is a trust-boundary
//! change, made in its own commit. This gate reads `verification/trust-boundary.toml`
//! and walks every non-merge commit between the merge base with `--base` and
//! `HEAD`. For each commit it classifies the touched paths as specification,
//! verifier, implementation, or neutral, reads the `Trust-Boundary: <class>`
//! lines of the message, and scans the added lines of implementation and
//! verifier files for escape hatches: an ignored test, a skipped mutant, an
//! allowed panic lint, an assumption in a proof harness.
//!
//! A commit fails when it touches a protected set it does not declare, mixes
//! a protected set with implementation, adds an escape hatch it does not
//! declare, declares a class it does not touch, or declares a class the
//! policy does not know. The gate does not judge the change. It makes the
//! change visible under its own name, so that a reviewer can.
//!
//! A pattern of the policy can be dated with a `since` commit, the commit that
//! added it. On a release check (`--release`, with a `v*` tag as the base) the
//! pattern is applied only to commits that descend from that commit, because a
//! release range can hold commits written before the rule existed. Every other
//! check applies every pattern to every commit, so a branch started before the
//! rule gets no exemption (verification-strategy.md, Rules Older Than the
//! Range).
//!
//! Merge commits are skipped: their constituent commits are in the range. A
//! squash merge keeps a trailer only if the squashed message does, which is
//! why this repository merges with merge commits. Policy violations carry a
//! stable [`TrustCode`]. Operational failures, such as a base revision that
//! is not available, are errors: the gate fails closed.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// The policy file, relative to the repository root.
pub(crate) const POLICY_PATH: &str = "verification/trust-boundary.toml";

/// The message line that declares a trust-boundary change.
const TRAILER: &str = "Trust-Boundary:";

/// Why a commit fails the policy. The kebab-case form is the stable identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TrustCode {
    /// A commit touches a protected set without declaring it.
    UndeclaredOracleChange,
    /// A commit touches a protected set and implementation together.
    MixedOracleAndImplementation,
    /// A commit adds an escape hatch without declaring it.
    EscapeHatchAdded,
    /// A commit declares a class it does not touch.
    TrailerWithoutChange,
    /// A commit declares a class the policy does not know.
    UnknownTrustBoundaryClass,
}

impl TrustCode {
    /// The stable identifier.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TrustCode::UndeclaredOracleChange => "undeclared-oracle-change",
            TrustCode::MixedOracleAndImplementation => "mixed-oracle-and-implementation",
            TrustCode::EscapeHatchAdded => "escape-hatch-added",
            TrustCode::TrailerWithoutChange => "trailer-without-change",
            TrustCode::UnknownTrustBoundaryClass => "unknown-trust-boundary-class",
        }
    }
}

/// The sets a path can belong to. `Neutral` is everything else.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Class {
    /// `[protected.specification]`
    Specification,
    /// `[protected.verifier]`
    Verifier,
    /// `[implementation]`
    Implementation,
    /// Declared with `Trust-Boundary: escape-hatch`; never a path class.
    EscapeHatch,
    /// Not in any set.
    Neutral,
}

impl Class {
    fn as_str(self) -> &'static str {
        match self {
            Class::Specification => "specification",
            Class::Verifier => "verifier",
            Class::Implementation => "implementation",
            Class::EscapeHatch => "escape-hatch",
            Class::Neutral => "neutral",
        }
    }

    fn declared(word: &str) -> Option<Class> {
        match word {
            "specification" => Some(Class::Specification),
            "verifier" => Some(Class::Verifier),
            "escape-hatch" => Some(Class::EscapeHatch),
            _ => None,
        }
    }
}

/// One failing commit and the reason.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TrustViolation {
    code: TrustCode,
    commit: String,
    subject: String,
    class: Option<Class>,
    path: Option<String>,
    detail: String,
}

impl TrustViolation {
    /// The stable reason.
    #[cfg(test)]
    pub(crate) fn code(&self) -> TrustCode {
        self.code
    }

    /// The class the failure names, if any.
    #[cfg(test)]
    pub(crate) fn class(&self) -> Option<Class> {
        self.class
    }

    /// The path the failure names, if any.
    #[cfg(test)]
    pub(crate) fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

impl fmt::Display for TrustViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} {}",
            self.code.as_str(),
            self.commit,
            self.subject
        )?;
        if let Some(class) = self.class {
            write!(f, " class={}", class.as_str())?;
        }
        if let Some(path) = &self.path {
            write!(f, " path={path}")?;
        }
        write!(f, ": {}", self.detail)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    protected: Protected,
    implementation: PathSet,
    #[serde(default)]
    scan: Scan,
    #[serde(default)]
    escape_hatch: Vec<EscapeHatch>,
    /// For a protected-path pattern, the commit that added it. A release
    /// check applies the pattern only to a commit that descends from it
    /// (verification-strategy.md, Rules Older Than the Range).
    #[serde(default)]
    since: BTreeMap<String, String>,
}

/// Paths the escape-hatch scan skips: the policy itself, which lists the
/// patterns, and fixture trees that exist to contain them. They stay
/// verifier paths, so a change to them is still declared.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Scan {
    #[serde(default)]
    exempt: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Protected {
    specification: PathSet,
    verifier: PathSet,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathSet {
    paths: Vec<String>,
}

impl PathSet {
    fn contains(&self, path: &str) -> bool {
        self.contains_unless(path, &BTreeSet::new())
    }

    /// Whether `path` matches a pattern that is not in `off`.
    fn contains_unless(&self, path: &str, off: &BTreeSet<String>) -> bool {
        self.paths
            .iter()
            .any(|p| !off.contains(p) && matches_path(p, path))
    }
}

/// Whether `path` is `pattern`, or, when the pattern ends in `/`, beneath it.
/// A segment of the pattern that is `*` stands for any one segment of the
/// path, so that `crates/*/*/tests/` is the `tests` directory of every crate
/// two levels down, whatever its layer and name.
pub(crate) fn matches_path(pattern: &str, path: &str) -> bool {
    if !pattern.contains('*') {
        return if pattern.ends_with('/') {
            path.starts_with(pattern)
        } else {
            path == pattern
        };
    }
    let beneath = pattern.ends_with('/');
    let want: Vec<&str> = pattern.trim_end_matches('/').split('/').collect();
    let have: Vec<&str> = path.split('/').collect();
    if beneath {
        // A path inside the directory has at least one more segment.
        if have.len() <= want.len() {
            return false;
        }
    } else if have.len() != want.len() {
        return false;
    }
    want.iter().zip(&have).all(|(w, h)| *w == "*" || w == h)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EscapeHatch {
    pattern: String,
    #[serde(default)]
    unless_line_contains: Vec<String>,
    reason: String,
}

impl EscapeHatch {
    fn matches(&self, line: &str) -> bool {
        line.contains(&self.pattern) && !self.unless_line_contains.iter().any(|u| line.contains(u))
    }
}

impl Policy {
    /// The class of `path`, with the patterns in `off` treated as not listed.
    fn classify(&self, path: &str, off: &BTreeSet<String>) -> Class {
        if self.protected.specification.contains_unless(path, off) {
            Class::Specification
        } else if self.protected.verifier.contains_unless(path, off) {
            Class::Verifier
        } else if self.implementation.contains_unless(path, off) {
            Class::Implementation
        } else {
            Class::Neutral
        }
    }
}

/// A full lowercase commit SHA.
fn full_sha(word: &str) -> bool {
    word.len() == 40
        && word
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `ancestor` is `descendant` or one of its ancestors.
fn is_ancestor(root: &Path, ancestor: &str, descendant: &str) -> Result<bool, String> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .status()
        .map_err(|e| format!("git: {e}"))?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git merge-base --is-ancestor {ancestor} {descendant} failed"
        )),
    }
}

/// Every `since` commit is a full SHA that this repository has. A rule whose
/// age cannot be checked is not skipped; the check fails.
fn check_since(root: &Path, policy: &Policy) -> Result<(), String> {
    for (pattern, since) in &policy.since {
        let known = full_sha(since)
            && git(root, &["cat-file", "-e", &format!("{since}^{{commit}}")]).is_ok();
        if !known {
            return Err(format!(
                "[trust-since-unresolved] {POLICY_PATH}: `since` for `{pattern}` names `{since}`, which is not a full commit SHA in this repository, so the age of the rule cannot be checked"
            ));
        }
    }
    Ok(())
}

/// A release check takes a release tag as its base.
fn require_release_tag(root: &Path, base: &str) -> Result<(), String> {
    let tag = base.strip_prefix("refs/tags/").unwrap_or(base);
    let is_tag = tag.starts_with('v')
        && git(
            root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/tags/{tag}^{{commit}}"),
            ],
        )
        .is_ok();
    if is_tag {
        Ok(())
    } else {
        Err(format!(
            "[trust-release-base-not-a-tag] --release needs a `v*` release tag as its base, and `{base}` is not one"
        ))
    }
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The `Trust-Boundary:` lines of a message, as written.
fn declarations(message: &str) -> Vec<String> {
    message
        .lines()
        .filter_map(|line| line.trim().strip_prefix(TRAILER))
        .map(|rest| rest.trim().to_owned())
        .collect()
}

/// Added lines of a unified diff, paired with the file they were added to.
fn added_lines(diff: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut file = String::new();
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            file = path.to_owned();
        } else if line.starts_with("+++ ") || line.starts_with("--- ") {
            continue;
        } else if let Some(added) = line.strip_prefix('+') {
            out.push((file.clone(), added.to_owned()));
        }
    }
    out
}

struct Commit {
    full: String,
    sha: String,
    subject: String,
    message: String,
    paths: Vec<String>,
    diff: String,
}

fn commits(root: &Path, base: &str) -> Result<Vec<Commit>, String> {
    let merge_base = git(root, &["merge-base", base, "HEAD"]).map_err(|e| {
        format!("{e}; the base revision is not available, so the trust boundary cannot be checked")
    })?;
    let range = format!("{}..HEAD", merge_base.trim());
    let mut out = Vec::new();
    for sha in git(root, &["rev-list", "--no-merges", "--reverse", &range])?.split_whitespace() {
        let message = git(root, &["show", "-s", "--format=%B", sha])?;
        let paths = git(
            root,
            &[
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "-r",
                "--root",
                sha,
            ],
        )?
        .lines()
        .map(str::to_owned)
        .collect();
        let diff = git(
            root,
            &[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "-p",
                "-U0",
                "--root",
                sha,
            ],
        )?;
        out.push(Commit {
            full: sha.to_owned(),
            sha: sha[..12.min(sha.len())].to_owned(),
            subject: message.lines().next().unwrap_or("").to_owned(),
            message,
            paths,
            diff,
        });
    }
    Ok(out)
}

/// The check's report.
#[derive(Serialize)]
pub(crate) struct Report {
    base: String,
    commits_checked: usize,
    violations: Vec<TrustViolation>,
}

impl Report {
    /// The failing commits, sorted and de-duplicated.
    pub(crate) fn violations(&self) -> &[TrustViolation] {
        &self.violations
    }

    /// How many commits were classified.
    pub(crate) fn commits_checked(&self) -> usize {
        self.commits_checked
    }
}

/// Runs the check over the commits between the merge base with `base` and `HEAD`.
///
/// With `release`, a pattern that the policy dates with `since` is applied to
/// a commit only when that commit descends from the `since` commit; any other
/// check applies every pattern to every commit.
pub(crate) fn check(root: &Path, base: &str, release: bool) -> Result<Report, String> {
    let policy_path = root.join(POLICY_PATH);
    let text = std::fs::read_to_string(&policy_path)
        .map_err(|e| format!("{}: {e}", policy_path.display()))?;
    let policy: Policy =
        toml::from_str(&text).map_err(|e| format!("{}: {}", policy_path.display(), e.message()))?;
    check_since(root, &policy)?;
    if release {
        require_release_tag(root, base)?;
    }
    let commits = commits(root, base)?;
    let mut violations: BTreeSet<TrustViolation> = BTreeSet::new();

    for commit in &commits {
        let mut off: BTreeSet<String> = BTreeSet::new();
        if release {
            for (pattern, since) in &policy.since {
                if !is_ancestor(root, since, &commit.full)? {
                    off.insert(pattern.clone());
                }
            }
        }
        let mut violation = |code, class: Option<Class>, path: Option<&str>, detail: String| {
            violations.insert(TrustViolation {
                code,
                commit: commit.sha.clone(),
                subject: commit.subject.clone(),
                class,
                path: path.map(str::to_owned),
                detail,
            });
        };

        let mut declared: BTreeSet<Class> = BTreeSet::new();
        for word in declarations(&commit.message) {
            match Class::declared(&word) {
                Some(class) => {
                    declared.insert(class);
                }
                None => violation(
                    TrustCode::UnknownTrustBoundaryClass,
                    None,
                    None,
                    format!(
                        "{TRAILER} {word} names no class; use specification, verifier, or escape-hatch"
                    ),
                ),
            }
        }

        let classified: Vec<(&str, Class)> = commit
            .paths
            .iter()
            .map(|p| (p.as_str(), policy.classify(p, &off)))
            .collect();
        let touched: BTreeSet<Class> = classified.iter().map(|(_, c)| *c).collect();
        let first_of = |class: Class| {
            classified
                .iter()
                .find(|(_, c)| *c == class)
                .map(|(p, _)| *p)
        };

        for class in [Class::Specification, Class::Verifier] {
            if touched.contains(&class) && !declared.contains(&class) {
                violation(
                    TrustCode::UndeclaredOracleChange,
                    Some(class),
                    first_of(class),
                    format!(
                        "changes the {} without a `{TRAILER} {}` line in the commit message",
                        class.as_str(),
                        class.as_str()
                    ),
                );
            }
            if declared.contains(&class) && !touched.contains(&class) {
                violation(
                    TrustCode::TrailerWithoutChange,
                    Some(class),
                    None,
                    format!(
                        "declares `{TRAILER} {}` but changes no {} path",
                        class.as_str(),
                        class.as_str()
                    ),
                );
            }
        }
        let protected =
            touched.contains(&Class::Specification) || touched.contains(&Class::Verifier);
        if protected && touched.contains(&Class::Implementation) {
            violation(
                TrustCode::MixedOracleAndImplementation,
                Some(Class::Implementation),
                first_of(Class::Implementation),
                "changes an oracle and implementation in one commit; a trust-boundary change is made in its own commit (AGENTS.md rule 11)".into(),
            );
        }

        let mut hatches = 0usize;
        let exempt = PathSet {
            paths: policy.scan.exempt.clone(),
        };
        for (file, line) in added_lines(&commit.diff) {
            let class = policy.classify(&file, &off);
            if !matches!(class, Class::Implementation | Class::Verifier) || exempt.contains(&file) {
                continue;
            }
            for hatch in policy.escape_hatch.iter().filter(|h| h.matches(&line)) {
                hatches += 1;
                if !declared.contains(&Class::EscapeHatch) {
                    violation(
                        TrustCode::EscapeHatchAdded,
                        Some(class),
                        Some(&file),
                        format!(
                            "adds `{}` ({}) without a `{TRAILER} escape-hatch` line",
                            hatch.pattern, hatch.reason
                        ),
                    );
                }
            }
        }
        if declared.contains(&Class::EscapeHatch) && hatches == 0 {
            violation(
                TrustCode::TrailerWithoutChange,
                Some(Class::EscapeHatch),
                None,
                format!("declares `{TRAILER} escape-hatch` but adds none"),
            );
        }
    }

    Ok(Report {
        base: base.to_owned(),
        commits_checked: commits.len(),
        violations: violations.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{Class, TrustCode, TrustViolation, added_lines, check, declarations};

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/agent-proof")
    }

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
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn copy_tree(from: &Path, to: &Path) {
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                std::fs::create_dir_all(&target).unwrap();
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// A repository with the fixture base committed on `main`, and a work
    /// branch carrying one commit per case, in order, each made of the case's
    /// `tree/` overlay and its `message`.
    fn repo(label: &str, cases: &[&str]) -> PathBuf {
        let root = crate::scratch::dir("trust", label);
        git_out(&root, &["init", "-q", "-b", "main"]);
        copy_tree(&fixtures().join("base"), &root);
        git_out(&root, &["add", "."]);
        git_out(&root, &["commit", "-q", "-m", "base"]);
        git_out(&root, &["checkout", "-q", "-b", "work"]);
        for case in cases {
            let dir = fixtures().join("cases").join(case);
            copy_tree(&dir.join("tree"), &root);
            let message = dir.join("message").canonicalize().unwrap();
            git_out(&root, &["add", "."]);
            git_out(&root, &["commit", "-q", "-F", message.to_str().unwrap()]);
        }
        root
    }

    fn run(case: &str) -> Vec<TrustViolation> {
        let root = repo(case, &[case]);
        let report = check(&root, "main", false).unwrap();
        assert_eq!(report.commits_checked(), 1);
        report.violations().to_vec()
    }

    fn codes(found: &[TrustViolation]) -> BTreeSet<TrustCode> {
        found.iter().map(TrustViolation::code).collect()
    }

    fn assert_case(case: &str, expected: &[TrustCode], class: Option<Class>, path: Option<&str>) {
        let found = run(case);
        assert_eq!(
            codes(&found),
            expected.iter().copied().collect(),
            "{case}: {found:#?}"
        );
        assert!(
            found.iter().any(|v| v.class() == class && v.path() == path),
            "{case}: no violation with class {class:?} path {path:?}: {found:#?}"
        );
    }

    fn assert_passes(case: &str) {
        let found = run(case);
        assert!(found.is_empty(), "{case}: {found:#?}");
    }

    #[test]
    fn an_implementation_only_commit_needs_no_declaration() {
        assert_passes("implementation-only");
    }

    #[test]
    fn a_declared_specification_change_passes() {
        assert_passes("declared-spec-change");
    }

    #[test]
    fn a_declared_verifier_change_passes() {
        assert_passes("declared-verifier-change");
    }

    #[test]
    fn a_declared_escape_hatch_passes_for_review() {
        assert_passes("declared-escape-hatch");
    }

    #[test]
    fn a_lint_allowed_only_under_test_is_not_an_escape_hatch() {
        assert_passes("test-scoped-allow");
    }

    #[test]
    fn a_pattern_added_to_the_policy_itself_is_not_an_escape_hatch() {
        assert_passes("pattern-in-policy");
    }

    #[test]
    fn an_undeclared_specification_change_is_rejected() {
        assert_case(
            "undeclared-spec-change",
            &[TrustCode::UndeclaredOracleChange],
            Some(Class::Specification),
            Some("docs/PROJECT-SPEC.md"),
        );
    }

    #[test]
    fn an_undeclared_verifier_change_is_rejected() {
        assert_case(
            "undeclared-verifier-change",
            &[TrustCode::UndeclaredOracleChange],
            Some(Class::Verifier),
            Some("crates/bin/nomos-xtask/src/main.rs"),
        );
    }

    #[test]
    fn loosening_a_gate_test_is_an_undeclared_verifier_change() {
        assert_case(
            "weakened-gate",
            &[TrustCode::UndeclaredOracleChange],
            Some(Class::Verifier),
            Some("crates/bin/nomos-xtask/src/main.rs"),
        );
    }

    #[test]
    fn removing_a_ci_step_is_an_undeclared_verifier_change() {
        assert_case(
            "removed-ci-step",
            &[TrustCode::UndeclaredOracleChange],
            Some(Class::Verifier),
            Some(".github/workflows/ci.yml"),
        );
    }

    #[test]
    fn an_undeclared_change_to_the_integration_tests_of_a_crate_is_rejected() {
        assert_case(
            "undeclared-crate-test-change",
            &[TrustCode::UndeclaredOracleChange],
            Some(Class::Verifier),
            Some("crates/bin/nomos-cell/tests/state.rs"),
        );
    }

    #[test]
    fn a_declared_change_to_the_integration_tests_of_a_crate_passes() {
        assert_passes("declared-crate-test-change");
    }

    #[test]
    fn a_crate_test_changed_with_its_implementation_is_rejected_even_when_declared() {
        assert_case(
            "crate-test-with-implementation",
            &[TrustCode::MixedOracleAndImplementation],
            Some(Class::Implementation),
            Some("crates/bin/nomos-cell/src/cli.rs"),
        );
    }

    /// The repository's own policy, not a fixture's: every crate's integration
    /// tests are verifier paths, and its sources are not.
    #[test]
    fn the_repository_policy_protects_the_integration_tests_of_every_crate() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let text = std::fs::read_to_string(root.join(super::POLICY_PATH)).unwrap();
        let policy: super::Policy = toml::from_str(&text).unwrap();
        for path in [
            "crates/bin/nomos-cell/tests/cell_commands.rs",
            "crates/bin/nomos-cell/tests/debian/mod.rs",
            "crates/core/nomos-core/tests/anything.rs",
            "crates/ports/nomos-store/tests/anything.rs",
            "crates/app/nomos-app/tests/anything.rs",
            "crates/adapters/nomos-store-fs/tests/anything.rs",
            "tests/semantic-mutants/corpus.toml",
        ] {
            assert_eq!(
                policy.classify(path, &BTreeSet::new()),
                Class::Verifier,
                "{path}"
            );
        }
        for path in [
            "crates/bin/nomos-cell/src/cli.rs",
            "crates/core/nomos-core/src/assessment.rs",
            "crates/adapters/nomos-store-fs/src/state.rs",
            "crates/bin/nomos-cell/tests",
            "crates/bin/nomos-cell/Cargo.toml",
        ] {
            assert_eq!(
                policy.classify(path, &BTreeSet::new()),
                Class::Implementation,
                "{path}"
            );
        }
    }

    #[test]
    fn a_star_segment_stands_for_one_segment() {
        use super::matches_path;
        assert!(matches_path(
            "crates/*/*/tests/",
            "crates/bin/nomos-cell/tests/a/b.rs"
        ));
        assert!(!matches_path("crates/*/*/tests/", "crates/bin/tests/a.rs"));
        assert!(!matches_path(
            "crates/*/*/tests/",
            "crates/bin/nomos-cell/tests"
        ));
        assert!(!matches_path(
            "crates/*/*/tests/",
            "crates/bin/nomos-cell/src/tests/a.rs"
        ));
        assert!(matches_path("a/*/c", "a/b/c"));
        assert!(!matches_path("a/*/c", "a/b/c/d"));
        assert!(matches_path("docs/adr/", "docs/adr/0015.md"));
    }

    #[test]
    fn a_specification_change_mixed_with_implementation_is_rejected_even_when_declared() {
        assert_case(
            "mixed-spec-and-implementation",
            &[TrustCode::MixedOracleAndImplementation],
            Some(Class::Implementation),
            Some("crates/core/nomos-core/src/lib.rs"),
        );
    }

    #[test]
    fn an_ignored_test_is_an_undeclared_escape_hatch() {
        assert_case(
            "ignored-test",
            &[TrustCode::EscapeHatchAdded],
            Some(Class::Implementation),
            Some("crates/core/nomos-core/src/lib.rs"),
        );
    }

    #[test]
    fn a_test_skipped_in_ci_is_an_undeclared_escape_hatch() {
        assert_case(
            "skipped-test-in-ci",
            &[TrustCode::EscapeHatchAdded],
            Some(Class::Verifier),
            Some(".github/workflows/ci.yml"),
        );
    }

    /// A receipt records the command a check ran; a skipped test named in it
    /// configures nothing and is not an escape hatch.
    #[test]
    fn a_skip_recorded_in_a_receipt_is_not_an_escape_hatch() {
        assert_passes("skip-recorded-in-receipt");
    }

    #[test]
    fn a_declaration_without_a_matching_change_is_rejected() {
        assert_case(
            "trailer-without-change",
            &[TrustCode::TrailerWithoutChange],
            Some(Class::Verifier),
            None,
        );
    }

    #[test]
    fn an_unknown_class_is_rejected() {
        assert_case(
            "unknown-class",
            &[TrustCode::UnknownTrustBoundaryClass],
            None,
            None,
        );
    }

    #[test]
    fn every_commit_in_the_range_is_checked_and_named() {
        let root = repo(
            "range",
            &[
                "implementation-only",
                "undeclared-spec-change",
                "ignored-test",
            ],
        );
        let report = check(&root, "main", false).unwrap();
        assert_eq!(report.commits_checked(), 3);
        assert_eq!(
            codes(report.violations()),
            [
                TrustCode::UndeclaredOracleChange,
                TrustCode::EscapeHatchAdded
            ]
            .into()
        );
        let subjects: BTreeSet<&str> = report
            .violations()
            .iter()
            .map(|v| v.subject.as_str())
            .collect();
        assert_eq!(subjects.len(), 2, "{:#?}", report.violations());
    }

    #[test]
    fn a_merge_commit_is_skipped_but_its_commits_are_not() {
        let root = repo("merge", &["undeclared-spec-change"]);
        git_out(&root, &["checkout", "-q", "main"]);
        git_out(&root, &["checkout", "-q", "-b", "other"]);
        std::fs::write(root.join("README.md"), "other\n").unwrap();
        git_out(&root, &["add", "."]);
        git_out(&root, &["commit", "-q", "-m", "other work"]);
        git_out(
            &root,
            &["merge", "-q", "--no-ff", "-m", "merge work", "work"],
        );
        let report = check(&root, "main", false).unwrap();
        assert_eq!(report.commits_checked(), 2);
        assert_eq!(
            codes(report.violations()),
            [TrustCode::UndeclaredOracleChange].into()
        );
    }

    #[test]
    fn an_unavailable_base_fails_closed() {
        let root = repo("nobase", &["implementation-only"]);
        let err = check(&root, "no-such-ref", false)
            .err()
            .expect("an unavailable base is an error");
        assert!(err.contains("not available"), "{err}");
    }

    /// An integration test of a crate that the fixture policy classifies as
    /// implementation unless the verifier pattern for it is in force.
    const CRATE_TEST: &str = "crates/bin/nomos-cell/tests";

    fn commit_file(root: &Path, path: &str, text: &str, message: &str) {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text).unwrap();
        git_out(root, &["add", "."]);
        git_out(root, &["commit", "-q", "-m", message]);
    }

    /// A history in which the rule for integration tests lands mid-range, with
    /// the release tag `v0.0.0` before all of it:
    ///
    /// 1. `old`, an integration test changed without a declaration, before the rule;
    /// 2. `rule`, the commit the policy will name as the rule's age;
    /// 3. the policy dates the pattern with that commit (declared);
    /// 4. `new`, the same undeclared change after the rule.
    fn rule_landing_mid_range(label: &str) -> PathBuf {
        let root = crate::scratch::dir("trust", label);
        git_out(&root, &["init", "-q", "-b", "main"]);
        copy_tree(&fixtures().join("base"), &root);
        git_out(&root, &["add", "."]);
        git_out(&root, &["commit", "-q", "-m", "base"]);
        git_out(&root, &["tag", "v0.0.0"]);
        git_out(&root, &["checkout", "-q", "-b", "work"]);
        commit_file(
            &root,
            &format!("{CRATE_TEST}/old.rs"),
            "// old\n",
            "Change an integration test before the rule",
        );
        commit_file(&root, "NOTES", "the rule lands\n", "The rule lands");
        let rule = git_out(&root, &["rev-parse", "HEAD"]);
        let policy = root.join(super::POLICY_PATH);
        let mut text = std::fs::read_to_string(&policy).unwrap();
        text.push_str(&format!("\n[since]\n\"crates/*/*/tests/\" = \"{rule}\"\n"));
        std::fs::write(&policy, text).unwrap();
        git_out(&root, &["add", "."]);
        git_out(
            &root,
            &[
                "commit",
                "-q",
                "-m",
                "Date the rule\n\nTrust-Boundary: verifier",
            ],
        );
        commit_file(
            &root,
            &format!("{CRATE_TEST}/new.rs"),
            "// new\n",
            "Change an integration test after the rule",
        );
        root
    }

    /// The paths of the violations, sorted: their order follows commit hashes.
    fn paths_of(found: &[TrustViolation]) -> Vec<&str> {
        let mut paths: Vec<&str> = found.iter().filter_map(TrustViolation::path).collect();
        paths.sort_unstable();
        paths
    }

    /// The release range holds a commit written before the rule. It is not
    /// judged by the rule, and the commit after the rule still is.
    #[test]
    fn a_release_does_not_hold_a_commit_older_than_a_rule_to_it() {
        let root = rule_landing_mid_range("release-epoch");
        let report = check(&root, "v0.0.0", true).unwrap();
        assert_eq!(report.commits_checked(), 4);
        assert_eq!(
            codes(report.violations()),
            [TrustCode::UndeclaredOracleChange].into(),
            "{:#?}",
            report.violations()
        );
        assert_eq!(
            paths_of(report.violations()),
            [format!("{CRATE_TEST}/new.rs")]
        );
    }

    /// Negative control: the same history on any other check holds both
    /// commits to the rule, so a branch started before the rule gets no
    /// exemption in review.
    #[test]
    fn a_pull_request_ignores_the_age_of_a_rule() {
        let root = rule_landing_mid_range("pr-strict");
        let report = check(&root, "main", false).unwrap();
        assert_eq!(report.commits_checked(), 4);
        assert_eq!(
            paths_of(report.violations()),
            [
                format!("{CRATE_TEST}/new.rs"),
                format!("{CRATE_TEST}/old.rs")
            ]
        );
    }

    /// Negative control: a release check takes a release tag as its base.
    #[test]
    fn a_release_check_whose_base_is_not_a_tag_fails() {
        let root = rule_landing_mid_range("release-not-tag");
        let err = check(&root, "main", true)
            .err()
            .expect("a branch is not a release");
        assert!(err.contains("[trust-release-base-not-a-tag]"), "{err}");
    }

    /// Negative control: a rule whose age cannot be checked fails the check,
    /// on every check, never skipped. A short SHA does not count.
    #[test]
    fn a_since_commit_that_cannot_be_resolved_fails_closed() {
        for bad in ["0".repeat(40), "9624211".to_owned()] {
            let root = rule_landing_mid_range(&format!("since-{}", bad.len()));
            let policy = root.join(super::POLICY_PATH);
            let text = std::fs::read_to_string(&policy).unwrap();
            let start = text.find("[since]").unwrap();
            let dated = format!(
                "{}[since]\n\"crates/*/*/tests/\" = \"{bad}\"\n",
                &text[..start]
            );
            std::fs::write(&policy, dated).unwrap();
            for (base, release) in [("v0.0.0", true), ("main", false)] {
                let err = check(&root, base, release)
                    .err()
                    .expect("an unresolved since is an error");
                assert!(err.contains("[trust-since-unresolved]"), "{err}");
            }
        }
    }

    /// The repository's policy dates the rule for integration tests.
    #[test]
    fn the_repository_policy_dates_the_integration_test_rule() {
        let text = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../")
                .join(super::POLICY_PATH),
        )
        .unwrap();
        let policy: super::Policy = toml::from_str(&text).unwrap();
        let dated = policy
            .since
            .get("crates/*/*/tests/")
            .expect("the rule is dated");
        assert!(super::full_sha(dated), "{dated}");
        assert!(
            policy
                .protected
                .verifier
                .paths
                .iter()
                .any(|p| p == "crates/*/*/tests/"),
            "the dated pattern is a verifier pattern"
        );
    }

    #[test]
    fn declarations_are_read_anywhere_in_the_message() {
        let message =
            "Subject\n\nBody.\nTrust-Boundary: verifier\n\nTrust-Boundary:   specification  \n";
        assert_eq!(declarations(message), ["verifier", "specification"]);
    }

    #[test]
    fn added_lines_are_paired_with_their_file() {
        let diff = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1,2 @@\n-old\n+new\n+more\ndiff --git a/y.rs b/y.rs\n--- a/y.rs\n+++ b/y.rs\n@@ -0,0 +1 @@\n+++y\n";
        assert_eq!(
            added_lines(diff),
            [("x.rs", "new"), ("x.rs", "more"), ("y.rs", "++y")]
                .map(|(f, l)| (f.to_owned(), l.to_owned()))
        );
    }

    #[test]
    fn every_case_directory_has_a_test() {
        let cases: BTreeSet<String> = std::fs::read_dir(fixtures().join("cases"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let source = include_str!("trust.rs");
        for case in &cases {
            assert!(
                source.contains(&format!("\"{case}\"")),
                "fixture case {case} has no test"
            );
        }
        assert_eq!(cases.len(), 19);
    }
}
