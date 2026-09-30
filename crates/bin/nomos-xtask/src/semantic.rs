//! The semantic-mutant runner: each named wrong behavior is caught by its
//! named test.
//!
//! `cargo mutants` generates mutants by syntax and asks whether any test
//! notices. A semantic mutant runs the other way: it is a hand-written patch
//! that makes the code wrong in one way an invariant forbids, for example a
//! failed Observation reported as a Variance, and the corpus names the one
//! test that must fail when the patch is applied. The runner applies each
//! active patch to a scratch copy of the workspace, runs only the named test,
//! and reports:
//!
//! | Outcome | Meaning |
//! | --- | --- |
//! | `caught` | the named test failed, as it must |
//! | `survived` | the named test passed; the test does not test what it claims |
//! | `unviable` | the patched crate did not compile; the mutant is broken |
//! | `named-test-missing` | the named test ran nothing; the corpus is stale |
//! | `patch-missing` | an active mutant has no patch file |
//! | `patch-does-not-apply` | the patch no longer matches the source |
//!
//! Only `caught` is acceptable for an active mutant. A planned mutant is
//! listed and not run; it may name a patch that does not exist yet. The
//! format of the corpus is in `tests/semantic-mutants/README.md`.
//!
//! A mutant of behavior that exists only on a real host, such as a unit
//! managed through systemd, names `host = "debian"` and the test binary as
//! `target`: its named test runs in a fresh Debian 12 container through
//! `cargo xtask debian`, since it is ignored everywhere else. A harness
//! that cannot run fails the whole run, never the one mutant.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::receipt::test_counts;

/// The corpus, relative to the repository root.
pub(crate) const CORPUS_PATH: &str = "tests/semantic-mutants/corpus.toml";

/// What happened to one mutant. The kebab-case form is the stable identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Outcome {
    /// The named test failed against the mutant.
    Caught,
    /// The named test passed against the mutant.
    Survived,
    /// The mutant did not compile.
    Unviable,
    /// The named test does not exist.
    NamedTestMissing,
    /// The patch file is not there.
    PatchMissing,
    /// The patch does not apply to the current source.
    PatchDoesNotApply,
    /// A planned mutant, listed and not run.
    Planned,
    /// A retired mutant, listed and not run.
    Retired,
}

impl Outcome {
    /// The stable identifier.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Outcome::Caught => "caught",
            Outcome::Survived => "survived",
            Outcome::Unviable => "unviable",
            Outcome::NamedTestMissing => "named-test-missing",
            Outcome::PatchMissing => "patch-missing",
            Outcome::PatchDoesNotApply => "patch-does-not-apply",
            Outcome::Planned => "planned",
            Outcome::Retired => "retired",
        }
    }

    /// Whether an active mutant with this outcome fails the run.
    fn is_failure(self) -> bool {
        !matches!(self, Outcome::Caught | Outcome::Planned | Outcome::Retired)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    #[serde(default)]
    mutant: Vec<Mutant>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Mutant {
    id: String,
    property: String,
    threat: String,
    wrong_behavior: String,
    #[serde(rename = "crate")]
    package: String,
    patch: String,
    test: String,
    status: Status,
    /// Where the named test runs.
    #[serde(default)]
    host: Host,
    /// The integration-test binary that holds the named test, for a mutant
    /// run on Debian.
    #[serde(default)]
    target: Option<String>,
}

/// Where a mutant's named test runs.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
enum Host {
    /// Here, with `cargo test`.
    #[default]
    Local,
    /// In a fresh Debian 12 container, through `cargo xtask debian`.
    Debian,
}

/// The release a Debian mutant's named test runs on.
const MUTANT_RELEASE: &str = "12";

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Status {
    Planned,
    Active,
    Retired,
}

/// One mutant's row of the report.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    id: String,
    property: String,
    threat: String,
    wrong_behavior: String,
    package: String,
    test: String,
    status: Status,
    outcome: Outcome,
    tests_run: Option<u64>,
    detail: String,
}

impl Row {
    /// The mutant's identifier.
    #[cfg(test)]
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// What happened.
    #[cfg(test)]
    pub(crate) fn outcome(&self) -> Outcome {
        self.outcome
    }
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} ({}; test {}): {}",
            self.outcome.as_str().to_ascii_uppercase(),
            self.id,
            self.property,
            self.test,
            self.detail
        )
    }
}

/// The run's report.
#[derive(Serialize)]
pub(crate) struct Report {
    corpus: PathBuf,
    rows: Vec<Row>,
}

impl Report {
    /// Every mutant, in corpus order.
    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The active mutants that were not caught.
    pub(crate) fn failures(&self) -> Vec<&Row> {
        self.rows
            .iter()
            .filter(|r| r.outcome.is_failure())
            .collect()
    }
}

/// Where the runner works.
pub(crate) struct Options {
    /// The workspace root the patches apply to.
    pub(crate) root: PathBuf,
    /// The corpus file; patch paths are relative to its directory.
    pub(crate) corpus: PathBuf,
    /// A target directory shared across mutants, so builds are incremental.
    pub(crate) target_dir: PathBuf,
    /// Pass `--offline` to Cargo, for fixtures with no external dependencies.
    pub(crate) offline: bool,
}

pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let target = to.join(&name);
        let path = entry.path();
        if path.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            copy_tree(&path, &target)?;
        } else {
            std::fs::copy(&path, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    Ok(())
}

/// Applies the mutant to a fresh copy of the workspace and runs its named test.
fn run_one(
    opts: &Options,
    mutant: &Mutant,
    patch: &Path,
) -> Result<(Outcome, Option<u64>, String), String> {
    let scratch = std::env::temp_dir().join(format!(
        "nomos-semantic-{}-{}",
        std::process::id(),
        mutant.id
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    copy_tree(&opts.root, &scratch)?;

    let applied = Command::new("git")
        .args(["apply", "--whitespace=nowarn"])
        .arg(patch)
        .current_dir(&scratch)
        .output()
        .map_err(|e| format!("git apply: {e}"))?;
    if !applied.status.success() {
        let detail = String::from_utf8_lossy(&applied.stderr).trim().to_owned();
        let _ = std::fs::remove_dir_all(&scratch);
        return Ok((Outcome::PatchDoesNotApply, None, detail));
    }

    if mutant.host == Host::Debian {
        let result = run_on_debian(opts, mutant, &scratch);
        let _ = std::fs::remove_dir_all(&scratch);
        return result;
    }

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args(["test", "--quiet", "-p", &mutant.package]);
    if opts.offline {
        cmd.arg("--offline");
    }
    cmd.arg(&mutant.test)
        .args(["--", "--exact"])
        .env("CARGO_TARGET_DIR", &opts.target_dir)
        .current_dir(&scratch);
    let out = cmd.output().map_err(|e| format!("cargo test: {e}"))?;
    let _ = std::fs::remove_dir_all(&scratch);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let Some((passed, failed)) = test_counts(&stdout) else {
        if stderr.contains("could not compile") || stderr.contains("error[E") {
            let first = stderr
                .lines()
                .find(|l| l.starts_with("error"))
                .unwrap_or("compile error")
                .to_owned();
            return Ok((Outcome::Unviable, None, first));
        }
        return Err(format!(
            "cargo test for {} produced no test result: {}",
            mutant.id,
            stderr.trim()
        ));
    };
    let run = passed + failed;
    if run == 0 {
        return Ok((
            Outcome::NamedTestMissing,
            Some(0),
            format!("{} ran no test named {}", mutant.package, mutant.test),
        ));
    }
    if failed > 0 {
        Ok((
            Outcome::Caught,
            Some(run),
            format!("{} failed against the mutant", mutant.test),
        ))
    } else {
        Ok((
            Outcome::Survived,
            Some(run),
            format!(
                "{} passed against the mutant; it does not detect: {}",
                mutant.test, mutant.wrong_behavior
            ),
        ))
    }
}

/// Runs a Debian mutant's named test in a fresh container, from the patched
/// copy of the workspace at `scratch`.
fn run_on_debian(
    opts: &Options,
    mutant: &Mutant,
    scratch: &Path,
) -> Result<(Outcome, Option<u64>, String), String> {
    let target = mutant.target.clone().ok_or_else(|| {
        format!(
            "{}: a Debian mutant names its test binary as `target`",
            mutant.id
        )
    })?;
    let selection = crate::debian::Selection {
        exact: Some(&mutant.test),
        target_dir: Some(&opts.target_dir),
        quiet: true,
    };
    let record = match crate::debian::run_suites(
        scratch,
        MUTANT_RELEASE,
        &[(mutant.package.clone(), target)],
        selection,
    ) {
        Ok(record) => record,
        Err(e) if e.contains("could not compile") || e.contains("error[E") => {
            let first = e
                .lines()
                .find(|l| l.contains("error"))
                .unwrap_or("compile error")
                .to_owned();
            return Ok((Outcome::Unviable, None, first));
        }
        Err(e) => return Err(format!("{}: the Debian harness failed: {e}", mutant.id)),
    };
    let (passed, failed) = record
        .runs
        .iter()
        .fold((0, 0), |(p, f), r| (p + r.passed, f + r.failed));
    let run = passed + failed;
    let on = &record.os_release;
    Ok(if run == 0 {
        (
            Outcome::NamedTestMissing,
            Some(0),
            format!(
                "{} ran no test named {} on {on}",
                mutant.package, mutant.test
            ),
        )
    } else if failed > 0 {
        (
            Outcome::Caught,
            Some(run),
            format!("{} failed against the mutant on {on}", mutant.test),
        )
    } else {
        (
            Outcome::Survived,
            Some(run),
            format!(
                "{} passed against the mutant on {on}; it does not detect: {}",
                mutant.test, mutant.wrong_behavior
            ),
        )
    })
}

/// Runs every active mutant of the corpus.
pub(crate) fn run(opts: &Options) -> Result<Report, String> {
    let text = std::fs::read_to_string(&opts.corpus)
        .map_err(|e| format!("{}: {e}", opts.corpus.display()))?;
    let corpus: Corpus =
        toml::from_str(&text).map_err(|e| format!("{}: {}", opts.corpus.display(), e.message()))?;
    let base = opts
        .corpus
        .parent()
        .ok_or("the corpus has no parent directory")?;
    let mut rows = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for mutant in &corpus.mutant {
        if !seen.insert(mutant.id.clone()) {
            return Err(format!(
                "{}: mutant {} is listed twice",
                opts.corpus.display(),
                mutant.id
            ));
        }
        let patch = base.join(&mutant.patch);
        let (outcome, tests_run, detail) = match mutant.status {
            Status::Planned => (Outcome::Planned, None, "listed, not run".to_owned()),
            Status::Retired => (Outcome::Retired, None, "retired, not run".to_owned()),
            Status::Active if !patch.is_file() => (
                Outcome::PatchMissing,
                None,
                format!("{} is not a file", patch.display()),
            ),
            Status::Active => run_one(opts, mutant, &patch)?,
        };
        rows.push(Row {
            id: mutant.id.clone(),
            property: mutant.property.clone(),
            threat: mutant.threat.clone(),
            wrong_behavior: mutant.wrong_behavior.clone(),
            package: mutant.package.clone(),
            test: mutant.test.clone(),
            status: mutant.status,
            outcome,
            tests_run,
            detail,
        });
    }
    Ok(Report {
        corpus: opts.corpus.clone(),
        rows,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{Options, Outcome, run};

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/semantic-mutants")
    }

    fn target_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/semantic-mutants-fixture")
    }

    #[test]
    fn every_outcome_is_reported_for_the_mutant_that_earns_it() {
        let report = run(&Options {
            root: fixtures().join("workspace"),
            corpus: fixtures().join("corpus.toml"),
            target_dir: target_dir(),
            offline: true,
        })
        .unwrap();
        let outcomes: BTreeMap<&str, Outcome> = report
            .rows()
            .iter()
            .map(|r| (r.id(), r.outcome()))
            .collect();
        assert_eq!(
            outcomes,
            [
                ("SM-FIX-001", Outcome::Caught),
                ("SM-FIX-002", Outcome::Survived),
                ("SM-FIX-003", Outcome::Unviable),
                ("SM-FIX-004", Outcome::NamedTestMissing),
                ("SM-FIX-005", Outcome::PatchMissing),
                ("SM-FIX-006", Outcome::Planned),
            ]
            .into(),
            "{:#?}",
            report.rows()
        );
        let failures: Vec<&str> = report.failures().iter().map(|r| r.id()).collect();
        assert_eq!(
            failures,
            ["SM-FIX-002", "SM-FIX-003", "SM-FIX-004", "SM-FIX-005"]
        );
        let line = report.rows()[0].to_string();
        assert!(
            line.starts_with("CAUGHT SM-FIX-001 (") && line.contains("; test "),
            "the printed row names the outcome, the mutant, and its test: {line}"
        );
    }

    /// The printed identifier of each outcome is its serialized form, the one
    /// the JSON report carries.
    #[test]
    fn each_outcome_prints_its_serialized_identifier() {
        for outcome in [
            Outcome::Caught,
            Outcome::Survived,
            Outcome::Unviable,
            Outcome::NamedTestMissing,
            Outcome::PatchMissing,
            Outcome::PatchDoesNotApply,
            Outcome::Planned,
            Outcome::Retired,
        ] {
            let serialized = serde_json::to_value(outcome).unwrap();
            assert_eq!(Some(outcome.as_str()), serialized.as_str(), "{outcome:?}");
        }
    }

    /// The scratch copy of a workspace leaves out its build output and its
    /// Git directory, and keeps everything else.
    #[test]
    fn copy_tree_leaves_out_target_and_git() {
        let base = std::env::temp_dir().join(format!("nomos-copy-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (from, to) = (base.join("from"), base.join("to"));
        for dir in ["target/debug", ".git/objects", "src/nested"] {
            std::fs::create_dir_all(from.join(dir)).unwrap();
        }
        std::fs::write(from.join("target/debug/big"), b"build output").unwrap();
        std::fs::write(from.join(".git/HEAD"), b"ref").unwrap();
        std::fs::write(from.join("src/nested/lib.rs"), b"kept").unwrap();
        std::fs::create_dir_all(&to).unwrap();
        super::copy_tree(&from, &to).unwrap();
        assert!(!to.join("target").exists());
        assert!(!to.join(".git").exists());
        assert_eq!(
            std::fs::read(to.join("src/nested/lib.rs")).unwrap(),
            b"kept"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_repository_corpus_parses_and_has_no_active_mutant_that_is_not_caught() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let report = run(&Options {
            root: root.clone(),
            corpus: root.join(super::CORPUS_PATH),
            target_dir: root.join("target/semantic-mutants"),
            offline: false,
        })
        .unwrap();
        assert!(report.failures().is_empty(), "{:#?}", report.failures());
    }
}
