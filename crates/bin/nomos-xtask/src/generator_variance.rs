//! Experiment `generator-variance`: independent generations of one kernel
//! function, judged by their own tests and by a fixed oracle.
//!
//! The design is `docs/research/2026-09-28-typed-core/results/generator-variance.md`.
//! Each candidate is one file holding `assess_evidence`, `assess_collection`,
//! and `assess` with the reference's signatures, any private helpers, and a
//! `#[cfg(test)] mod tests` of its own. For every candidate the harness:
//!
//! 1. formats it with `rustfmt`, the formatter `cargo fmt` runs, and records
//!    whether normalization changed it;
//! 2. splices its functions into a scratch copy of the workspace in place of
//!    the reference's three, keeping everything else of
//!    `crates/core/nomos-core/src/assessment.rs`, the reference's own tests
//!    included;
//! 3. runs the candidate's tests, renamed `mod candidate_tests`, against the
//!    candidate (own-test acceptance);
//! 4. runs every `nomos-core` test, without the candidate's module, against
//!    the candidate (oracle acceptance);
//! 5. runs the candidate's tests against the reference and against the
//!    reference with each semantic mutant applied (whether the candidate's
//!    tests catch the mutant);
//! 6. compares every candidate that compiles, with the reference, on
//!    exhaustive bounded inputs and on generated inputs from a fixed seed.
//!
//! | Outcome | Meaning |
//! | --- | --- |
//! | `accepted` | the tests ran and none failed |
//! | `rejected` | at least one test failed |
//! | `does-not-compile` | the spliced crate did not compile |
//! | `ran-nothing` | the filter matched no test |
//!
//! One scratch copy of the workspace serves every candidate; the harness
//! rewrites `assessment.rs` in it before each run and removes the copy when
//! the run ends, whether it succeeded or not. Every build shares one target
//! directory with the semantic-mutant runner.
//!
//! The result is exploratory. A candidate's outcome never fails the command.
//! The command fails when the negative control is not reported as
//! own-tests-accepted and oracle-rejected, because then the harness is not
//! measuring, and when a step cannot run at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::receipt::test_counts;
use crate::semantic::copy_tree;

/// The reference implementation, relative to the workspace root.
pub(crate) const REFERENCE_PATH: &str = "crates/core/nomos-core/src/assessment.rs";

/// The package the reference belongs to.
const PACKAGE: &str = "nomos-core";

/// The name the candidate's own test module takes once spliced.
const CANDIDATE_MODULE: &str = "candidate_tests";

/// The semantic mutants of the reference that the candidates' tests face.
pub(crate) const MUTANTS: [&str; 3] = ["SM-ASSESS-001", "SM-ASSESS-002", "SM-ASSESS-003"];

/// The candidate directory that holds the negative control.
pub(crate) const CONTROL: &str = "cand-0";

/// The generated comparison test, relative to the package.
const COMPARISON_TEST: &str = "generator_variance";

/// The functions every candidate must define.
const FUNCTIONS: [&str; 3] = [
    "pub fn assess_evidence(",
    "pub fn assess_collection(",
    "pub fn assess(",
];

/// A candidate file cut in two.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// The three functions and any private helpers.
    pub(crate) implementation: String,
    /// The candidate's test module, renamed to [`CANDIDATE_MODULE`].
    pub(crate) tests: String,
}

/// Splits a candidate into its implementation and its test module, and
/// renames the module so it can sit beside the reference's `mod tests`.
pub(crate) fn split_candidate(text: &str) -> Result<Candidate, String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .windows(2)
        .position(|w| w[0].trim() == "#[cfg(test)]" && w[1].trim() == "mod tests {")
        .ok_or("no `#[cfg(test)] mod tests {` block")?;
    let close = lines[start + 1..]
        .iter()
        .position(|l| *l == "}")
        .map(|i| start + 1 + i)
        .ok_or("the `mod tests` block is not closed at column 0")?;
    if lines[close + 1..].iter().any(|l| !l.trim().is_empty()) {
        return Err("text follows the `mod tests` block".into());
    }
    let implementation = lines[..start].join("\n").trim_end().to_owned() + "\n";
    for function in FUNCTIONS {
        if !defines(&implementation, function) {
            return Err(format!("the implementation does not define `{function}`"));
        }
    }
    let mut tests = Vec::with_capacity(close - start + 1);
    tests.push("#[cfg(test)]".to_owned());
    tests.push(format!("mod {CANDIDATE_MODULE} {{"));
    tests.extend(lines[start + 2..=close].iter().map(|l| (*l).to_owned()));
    Ok(Candidate {
        implementation,
        tests: tests.join("\n") + "\n",
    })
}

/// Whether `text` has a line that begins with `signature`.
fn defines(text: &str, signature: &str) -> bool {
    text.lines().any(|l| l.starts_with(signature))
}

/// The byte range of the reference's three functions, doc comments
/// included: from the docs of `assess_evidence` to the closing brace of
/// `assess` and its newline.
pub(crate) fn reference_span(text: &str) -> Result<(usize, usize), String> {
    let first = line_start(text, FUNCTIONS[0])
        .ok_or("the reference does not define `assess_evidence` at column 0")?;
    let mut start = first;
    while let Some(previous) = text[..start.saturating_sub(1)].rfind('\n').map(|i| i + 1) {
        if text[previous..start].starts_with("///") {
            start = previous;
        } else {
            break;
        }
        if previous == 0 {
            break;
        }
    }
    let last = line_start(text, FUNCTIONS[2])
        .ok_or("the reference does not define `assess` at column 0")?;
    let close = text[last..]
        .find("\n}\n")
        .ok_or("`assess` is not closed at column 0")?;
    let end = last + close + 3;
    if !text[start..end].contains(FUNCTIONS[1]) {
        return Err("the reference's three functions are not contiguous".into());
    }
    Ok((start, end))
}

/// The byte offset of the first line that begins with `prefix`.
fn line_start(text: &str, prefix: &str) -> Option<usize> {
    if text.starts_with(prefix) {
        return Some(0);
    }
    text.find(&format!("\n{prefix}")).map(|i| i + 1)
}

/// The reference module with its three functions replaced by
/// `implementation` and, when given, `tests` appended at the end.
pub(crate) fn splice(
    reference: &str,
    implementation: &str,
    tests: Option<&str>,
) -> Result<String, String> {
    let (start, end) = reference_span(reference)?;
    let mut out = String::with_capacity(reference.len() + implementation.len());
    out.push_str(&reference[..start]);
    out.push_str(implementation.trim_end());
    out.push('\n');
    out.push_str(&reference[end..]);
    if let Some(tests) = tests {
        append_tests(&mut out, tests);
    }
    Ok(out)
}

fn append_tests(module: &mut String, tests: &str) {
    if !module.ends_with('\n') {
        module.push('\n');
    }
    module.push('\n');
    module.push_str(tests);
}

/// The item the first hunk of a unified diff sits in, from the context text
/// Git prints after the hunk header: the name after `fn ` or `impl `.
pub(crate) fn touched_item(patch: &str) -> Option<String> {
    let header = patch.lines().find(|l| l.starts_with("@@"))?;
    let context = header.splitn(3, "@@").nth(2)?.trim();
    let rest = context
        .split_once("fn ")
        .or_else(|| context.split_once("impl "))?
        .1;
    let name: String = rest.chars().take_while(|c| is_ident(*c)).collect();
    (!name.is_empty()).then_some(name)
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether `text` names `ident` as a whole word.
pub(crate) fn mentions(text: &str, ident: &str) -> bool {
    text.match_indices(ident).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + ident.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// Names of the tests `cargo test` reported as failed.
pub(crate) fn failed_tests(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|l| l.strip_prefix("test ")?.strip_suffix(" ... FAILED"))
        .map(str::to_owned)
        .collect()
}

/// The names of the functions `implementation` defines, public or not.
pub(crate) fn defined_functions(implementation: &str) -> Vec<String> {
    implementation
        .lines()
        .filter_map(|l| {
            l.strip_prefix("pub fn ")
                .or_else(|| l.strip_prefix("fn "))
                .map(|rest| rest.chars().take_while(|c| is_ident(*c)).collect())
        })
        .collect()
}

/// Lines of `implementation` that are neither blank nor comments.
pub(crate) fn code_lines(implementation: &str) -> usize {
    implementation
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .count()
}

/// A Rust module name for a candidate directory name.
pub(crate) fn module_name(candidate: &str) -> String {
    let mut name: String = candidate
        .chars()
        .map(|c| if is_ident(c) { c } else { '_' })
        .collect();
    if name.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

/// The comparison test: every implementation in its own module, compared on
/// exhaustive bounded inputs and on generated inputs. `impls` pairs a module
/// name with the implementation text; the first is the baseline.
pub(crate) fn comparison_source(impls: &[(String, String)]) -> String {
    let mut out = String::from(COMPARISON_HEADER);
    for (name, implementation) in impls {
        out.push_str(&format!(
            "\n#[allow(dead_code, unused_imports)]\nmod {name} {{\n{COMPARISON_PRELUDE}\n{implementation}}}\n"
        ));
    }
    out.push_str("\nconst IMPLS: &[Impl] = &[\n");
    for (name, _) in impls {
        out.push_str(&format!(
            "    Impl {{ name: {name:?}, evidence: {name}::assess_evidence, collection: {name}::assess_collection, assess: {name}::assess }},\n"
        ));
    }
    out.push_str("];\n");
    out.push_str(COMPARISON_BODY);
    out
}

const COMPARISON_HEADER: &str = r#"//! Generated by `cargo xtask generator-variance`. Not part of the repository.

extern crate alloc;

use std::collections::BTreeMap;

use nomos_core::assessment::Assessment;
use nomos_core::condition::{Condition, Content, FileCondition};
use nomos_core::observation::{
    Collection, CollectionFailure, CollectorId, FileEvidence, Instant, Observation, Provenance,
    Window,
};
use nomos_core::resource::{Digest, ResourcePath};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, RngAlgorithm, TestError, TestRng, TestRunner};

struct Impl {
    name: &'static str,
    evidence: fn(&FileCondition, &FileEvidence) -> Assessment,
    collection: fn(&FileCondition, &Collection) -> Assessment,
    assess: fn(&Condition, &[Observation]) -> Assessment,
}
"#;

const COMPARISON_PRELUDE: &str = "    use alloc::vec::Vec;
    use nomos_core::assessment::{Assessment, Reason, Variance};
    use nomos_core::condition::{Condition, Content, FileCondition};
    use nomos_core::observation::{Collection, CollectionFailure, FileEvidence, Observation};
    use nomos_core::resource::Digest;
";

const COMPARISON_BODY: &str = r#"
/// The seed of the generated comparison: "gv", then zeros. Fixed forever.
const SEED: [u8; 32] = [
    0x67, 0x76, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0,
];
const CASES: u32 = 4096;
const MAX_LEN: usize = 3;

fn digest(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}

fn requirements() -> Vec<FileCondition> {
    vec![
        FileCondition::Absent,
        FileCondition::Present { content: Content::Any },
        FileCondition::Present { content: Content::Exactly(digest(1)) },
        FileCondition::Present { content: Content::Exactly(digest(2)) },
    ]
}

fn evidences() -> Vec<FileEvidence> {
    vec![
        FileEvidence::Absent,
        FileEvidence::Present { digest: digest(1), size: 0 },
        FileEvidence::Present { digest: digest(1), size: 1 },
        FileEvidence::Present { digest: digest(2), size: 0 },
        FileEvidence::Present { digest: digest(3), size: 0 },
    ]
}

const FAILURES: [CollectionFailure; 4] = [
    CollectionFailure::PermissionDenied,
    CollectionFailure::TimedOut,
    CollectionFailure::Unsupported,
    CollectionFailure::Io,
];

fn collections() -> Vec<Collection> {
    let mut out: Vec<Collection> = evidences().into_iter().map(Collection::Collected).collect();
    out.extend(FAILURES.map(Collection::Failed));
    out
}

fn provenance(collector: &str, end: u64) -> Provenance {
    Provenance::new(
        CollectorId::new(collector).unwrap(),
        Window::new(Instant(0), Instant(end)).unwrap(),
    )
}

fn path(n: u8) -> ResourcePath {
    ResourcePath::new(&format!("/r/{n}")).unwrap()
}

/// Disagreements between two implementations over one domain.
#[derive(Default)]
struct Pair {
    disagreements: u64,
    first: Option<String>,
    minimized: Option<String>,
}

struct Domain {
    function: &'static str,
    domain: &'static str,
    cases: u64,
    pairs: BTreeMap<(usize, usize), Pair>,
}

impl Domain {
    fn new(function: &'static str, domain: &'static str) -> Self {
        let mut pairs = BTreeMap::new();
        for a in 0..IMPLS.len() {
            for b in a + 1..IMPLS.len() {
                pairs.insert((a, b), Pair::default());
            }
        }
        Domain { function, domain, cases: 0, pairs }
    }

    fn record(&mut self, input: &dyn Fn() -> String, outputs: &[Assessment]) {
        self.cases += 1;
        for ((a, b), pair) in self.pairs.iter_mut() {
            if outputs[*a] != outputs[*b] {
                pair.disagreements += 1;
                if pair.first.is_none() {
                    pair.first = Some(format!(
                        "{} => {}: {:?}; {}: {:?}",
                        input(),
                        IMPLS[*a].name,
                        outputs[*a],
                        IMPLS[*b].name,
                        outputs[*b]
                    ));
                }
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        let pairs: Vec<serde_json::Value> = self
            .pairs
            .iter()
            .map(|((a, b), p)| {
                serde_json::json!({
                    "a": IMPLS[*a].name,
                    "b": IMPLS[*b].name,
                    "disagreements": p.disagreements,
                    "first": p.first,
                    "minimized": p.minimized,
                })
            })
            .collect();
        serde_json::json!({
            "function": self.function,
            "domain": self.domain,
            "cases": self.cases,
            "pairs": pairs,
        })
    }
}

fn exhaustive_evidence() -> Domain {
    let mut d = Domain::new("assess_evidence", "exhaustive: 4 requirements x 5 evidences");
    for r in requirements() {
        for e in evidences() {
            let outputs: Vec<Assessment> = IMPLS.iter().map(|i| (i.evidence)(&r, &e)).collect();
            d.record(&|| format!("{r:?} vs {e:?}"), &outputs);
        }
    }
    d
}

fn exhaustive_collection() -> Domain {
    let mut d = Domain::new("assess_collection", "exhaustive: 4 requirements x 9 collections");
    for r in requirements() {
        for c in collections() {
            let outputs: Vec<Assessment> = IMPLS.iter().map(|i| (i.collection)(&r, &c)).collect();
            d.record(&|| format!("{r:?} vs {c:?}"), &outputs);
        }
    }
    d
}

/// Every Observation list of length at most `MAX_LEN` over 2 paths, 9
/// collections, and 2 provenances (36 atoms), against 2 paths x 4
/// requirements. Lists are enumerated shortest first, so the first
/// disagreement recorded is one of minimal length.
fn exhaustive_assess() -> Domain {
    let mut d = Domain::new(
        "assess",
        "exhaustive: lists of length 0..=3 over 36 Observations, 8 Conditions",
    );
    let mut atoms = Vec::new();
    for p in 0..2 {
        for c in collections() {
            for v in [provenance("a", 0), provenance("b", 10)] {
                atoms.push(Observation::file(path(p), c, v));
            }
        }
    }
    let mut conditions = Vec::new();
    for p in 0..2 {
        for r in requirements() {
            conditions.push(Condition::file(path(p), r));
        }
    }
    let mut lists: Vec<Vec<usize>> = vec![Vec::new()];
    let mut frontier: Vec<Vec<usize>> = vec![Vec::new()];
    for _ in 0..MAX_LEN {
        let mut next = Vec::new();
        for list in &frontier {
            for i in 0..atoms.len() {
                let mut longer = list.clone();
                longer.push(i);
                next.push(longer);
            }
        }
        lists.extend(next.iter().cloned());
        frontier = next;
    }
    for list in &lists {
        let observations: Vec<Observation> = list.iter().map(|i| atoms[*i].clone()).collect();
        for condition in &conditions {
            let outputs: Vec<Assessment> =
                IMPLS.iter().map(|i| (i.assess)(condition, &observations)).collect();
            d.record(&|| format!("{condition:?} vs {observations:?}"), &outputs);
        }
    }
    d
}

fn runner() -> TestRunner {
    let config = Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    };
    TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &SEED))
}

fn digest_strategy() -> impl Strategy<Value = Digest> {
    (1u8..4).prop_map(digest)
}

fn input() -> impl Strategy<Value = (Condition, Vec<Observation>)> {
    let requirement = prop_oneof![
        Just(FileCondition::Absent),
        Just(FileCondition::Present { content: Content::Any }),
        digest_strategy().prop_map(|d| FileCondition::Present { content: Content::Exactly(d) }),
    ];
    let evidence = prop_oneof![
        Just(FileEvidence::Absent),
        (digest_strategy(), 0u64..2).prop_map(|(digest, size)| FileEvidence::Present { digest, size }),
    ];
    let collection = prop_oneof![
        evidence.prop_map(Collection::Collected),
        (0usize..4).prop_map(|i| Collection::Failed(FAILURES[i])),
    ];
    let provenance = (0usize..3, any::<u64>())
        .prop_map(|(c, end)| provenance(["a", "b", "c"][c], end));
    let observation = (0u8..3, collection, provenance)
        .prop_map(|(p, c, v)| Observation::file(path(p), c, v));
    let condition = (0u8..3, requirement).prop_map(|(p, r)| Condition::file(path(p), r));
    (condition, prop::collection::vec(observation, 0..=10))
}

/// Generated inputs from the fixed seed: lists of up to 10 Observations over
/// 3 paths, 3 digests, 2 sizes, 3 collectors, and any window end. Each pair
/// that disagrees is then minimized by the runner's shrinking.
fn generated_assess() -> Domain {
    let mut d = Domain::new(
        "assess",
        "generated: 4096 cases, lists of length 0..=10 over 3 paths, seed 0x677601 then zeros",
    );
    let strategy = input();
    let mut rng = runner();
    for _ in 0..CASES {
        let (condition, observations) = strategy.new_tree(&mut rng).unwrap().current();
        let outputs: Vec<Assessment> =
            IMPLS.iter().map(|i| (i.assess)(&condition, &observations)).collect();
        d.record(&|| format!("{condition:?} vs {observations:?}"), &outputs);
    }
    for ((a, b), pair) in d.pairs.iter_mut() {
        if pair.disagreements == 0 {
            continue;
        }
        let (fa, fb) = (IMPLS[*a].assess, IMPLS[*b].assess);
        let result = runner().run(&strategy, |(condition, observations)| {
            prop_assert_eq!(fa(&condition, &observations), fb(&condition, &observations));
            Ok(())
        });
        if let Err(TestError::Fail(_, (condition, observations))) = result {
            pair.minimized = Some(format!(
                "{condition:?} vs {observations:?} => {}: {:?}; {}: {:?}",
                IMPLS[*a].name,
                fa(&condition, &observations),
                IMPLS[*b].name,
                fb(&condition, &observations)
            ));
        }
    }
    d
}

#[test]
fn compare() {
    let domains = [
        exhaustive_evidence(),
        exhaustive_collection(),
        exhaustive_assess(),
        generated_assess(),
    ];
    let record = serde_json::json!({
        "impls": IMPLS.iter().map(|i| i.name).collect::<Vec<_>>(),
        "domains": domains.iter().map(Domain::json).collect::<Vec<_>>(),
    });
    println!("GV-RECORD {record}");
}
"#;

/// The prefix of the comparison test's one output line.
const RECORD_PREFIX: &str = "GV-RECORD ";

/// What a `cargo test` run concluded.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Outcome {
    /// Tests ran and none failed.
    Accepted,
    /// At least one test failed.
    Rejected,
    /// The crate did not compile.
    DoesNotCompile,
    /// No test matched.
    RanNothing,
}

/// One `cargo test` run.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct TestRun {
    outcome: Outcome,
    passed: u64,
    failed: u64,
    failed_tests: Vec<String>,
    detail: String,
}

/// What a semantic mutant of the reference did to the candidate's tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MutantOutcome {
    /// The candidate's tests failed against the mutant.
    Caught,
    /// The candidate's tests passed against the mutant.
    Survived,
    /// The candidate's tests already fail against the unpatched reference,
    /// so a failure against the mutant says nothing.
    BaselineFails,
    /// The mutant with the candidate's tests did not compile.
    Unviable,
    /// The patch did not apply.
    PatchDoesNotApply,
}

/// One mutant against one candidate's tests.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct MutantRow {
    id: String,
    wrong_behavior: String,
    touched_item: Option<String>,
    candidate_tests_name_it: bool,
    outcome: MutantOutcome,
    failed_tests: Vec<String>,
}

/// One candidate's row.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CandidateRow {
    id: String,
    negative_control: bool,
    candidate_sha256: String,
    normalization_changed: bool,
    functions: Vec<String>,
    implementation_code_lines: usize,
    tests_declared: usize,
    own_tests: Option<TestRun>,
    oracle: Option<TestRun>,
    own_tests_against_reference: Option<TestRun>,
    mutants: Vec<MutantRow>,
    detail: String,
}

impl CandidateRow {
    fn own(&self) -> Option<Outcome> {
        self.own_tests.as_ref().map(|r| r.outcome)
    }

    fn oracle(&self) -> Option<Outcome> {
        self.oracle.as_ref().map(|r| r.outcome)
    }
}

/// The counts the design's Measurements table asks for, over the generated
/// candidates only; the negative control is reported on its own.
#[derive(Serialize, Debug)]
pub(crate) struct Summary {
    n: usize,
    compiled: usize,
    own_tests_accepted: usize,
    oracle_accepted: usize,
    own_accepted_oracle_rejected: usize,
    mutants_caught: BTreeMap<String, usize>,
    negative_control: Option<ControlOutcome>,
}

/// The negative control's result.
#[derive(Serialize, Debug)]
pub(crate) struct ControlOutcome {
    own_tests: Option<Outcome>,
    oracle: Option<Outcome>,
    as_required: bool,
}

/// The experiment's record.
#[derive(Serialize, Debug)]
pub(crate) struct Record {
    experiment: &'static str,
    reference: &'static str,
    reference_sha256: String,
    candidates: Vec<CandidateRow>,
    comparison: Option<Value>,
    summary: Summary,
}

impl Record {
    /// Why the harness is not measuring, if it is not.
    pub(crate) fn failures(&self) -> Vec<String> {
        let mut out = Vec::new();
        match &self.summary.negative_control {
            None => out.push(format!("no negative control ({CONTROL}) among the candidates")),
            Some(c) if !c.as_required => out.push(format!(
                "the negative control was reported own tests {:?}, oracle {:?}; required accepted, rejected",
                c.own_tests, c.oracle
            )),
            Some(_) => {}
        }
        if self.comparison.is_none() {
            out.push("the generated-input comparison produced no record".into());
        }
        out
    }
}

/// Where the harness works.
pub(crate) struct Options {
    /// The workspace root.
    pub(crate) root: PathBuf,
    /// The directory of candidate directories, each with `candidate.rs`.
    pub(crate) candidates: PathBuf,
    /// The scratch copy of the workspace.
    pub(crate) scratch: PathBuf,
    /// A target directory shared across runs, so builds are incremental.
    pub(crate) target_dir: PathBuf,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Formats `text` as a standalone file with `rustfmt`, from inside the
/// scratch workspace so its toolchain and edition apply.
fn normalize(scratch: &Path, name: &str, text: &str) -> Result<String, String> {
    let dir = scratch.join("target-generator-variance-normalize");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("{name}.rs"));
    write(&file, text)?;
    let out = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(&file)
        .current_dir(scratch)
        .output()
        .map_err(|e| format!("rustfmt: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "rustfmt {name}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let formatted = read(&file)?;
    let _ = std::fs::remove_file(&file);
    Ok(formatted)
}

/// Runs `cargo test -p nomos-core --locked --offline <args>` in the scratch copy.
fn cargo_test(opts: &Options, args: &[&str]) -> Result<(TestRun, String), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["test", "-p", PACKAGE, "--locked", "--offline"])
        .args(args)
        .env("CARGO_TARGET_DIR", &opts.target_dir)
        .current_dir(&opts.scratch)
        .output()
        .map_err(|e| format!("cargo test: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let run = match test_counts(&stdout) {
        None if stderr.contains("could not compile") || stderr.contains("error[E") => TestRun {
            outcome: Outcome::DoesNotCompile,
            passed: 0,
            failed: 0,
            failed_tests: Vec::new(),
            detail: stderr
                .lines()
                .find(|l| l.starts_with("error"))
                .unwrap_or("compile error")
                .to_owned(),
        },
        None => {
            return Err(format!(
                "cargo test {} produced no test result: {}",
                args.join(" "),
                stderr.trim()
            ));
        }
        Some((passed, failed)) => TestRun {
            outcome: if failed > 0 {
                Outcome::Rejected
            } else if passed == 0 {
                Outcome::RanNothing
            } else {
                Outcome::Accepted
            },
            passed,
            failed,
            failed_tests: failed_tests(&stdout),
            detail: format!("cargo test {}", args.join(" ")),
        },
    };
    Ok((run, stdout))
}

const OWN_TESTS: &[&str] = &["--lib", "--", "assessment::candidate_tests::"];
const ORACLE: &[&str] = &["--no-fail-fast"];

struct Mutant {
    id: String,
    wrong_behavior: String,
    patch: PathBuf,
}

fn mutants(root: &Path) -> Result<Vec<Mutant>, String> {
    let corpus_path = root.join(crate::semantic::CORPUS_PATH);
    let corpus: toml::Table = toml::from_str(&read(&corpus_path)?)
        .map_err(|e| format!("{}: {}", corpus_path.display(), e.message()))?;
    let base = corpus_path.parent().ok_or("the corpus has no parent")?;
    let entries = corpus
        .get("mutant")
        .and_then(toml::Value::as_array)
        .ok_or("the corpus lists no mutant")?;
    MUTANTS
        .iter()
        .map(|id| {
            let entry = entries
                .iter()
                .find(|m| m.get("id").and_then(toml::Value::as_str) == Some(id))
                .ok_or_else(|| format!("{id} is not in the corpus"))?;
            let field = |name: &str| {
                entry
                    .get(name)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{id} has no {name}"))
            };
            if field("status")? != "active" {
                return Err(format!("{id} is not active"));
            }
            Ok(Mutant {
                id: (*id).to_owned(),
                wrong_behavior: field("wrong_behavior")?,
                patch: base.join(field("patch")?),
            })
        })
        .collect()
}

/// Runs the experiment over every `*/candidate.rs` under `opts.candidates`.
pub(crate) fn run(opts: &Options) -> Result<Record, String> {
    let reference = read(&opts.root.join(REFERENCE_PATH))?;
    let reference_span = reference_span(&reference)?;
    let mutants = mutants(&opts.root)?;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&opts.candidates)
        .map_err(|e| format!("{}: {e}", opts.candidates.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("candidate.rs").is_file())
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        return Err(format!("{}: no */candidate.rs", opts.candidates.display()));
    }

    // One scratch copy for every candidate, removed whatever the outcome.
    let _ = std::fs::remove_dir_all(&opts.scratch);
    let judged = std::fs::create_dir_all(&opts.scratch)
        .map_err(|e| format!("{}: {e}", opts.scratch.display()))
        .and_then(|()| copy_tree(&opts.root, &opts.scratch))
        .and_then(|()| judge(opts, &reference, reference_span, &mutants, &dirs));
    let _ = std::fs::remove_dir_all(&opts.scratch);
    let (rows, comparison) = judged?;

    let summary = summarize(&rows);
    Ok(Record {
        experiment: "generator-variance",
        reference: REFERENCE_PATH,
        reference_sha256: sha256_hex(reference.as_bytes()),
        candidates: rows,
        comparison,
        summary,
    })
}

/// Judges every candidate in the scratch copy, then compares them.
fn judge(
    opts: &Options,
    reference: &str,
    reference_span: (usize, usize),
    mutants: &[Mutant],
    dirs: &[PathBuf],
) -> Result<(Vec<CandidateRow>, Option<Value>), String> {
    let module = opts.scratch.join(REFERENCE_PATH);
    let mut rows = Vec::new();
    let mut compiled: Vec<(String, String)> = vec![(
        "reference".to_owned(),
        reference[reference_span.0..reference_span.1].to_owned(),
    )];
    for dir in dirs {
        let id = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        eprintln!("generator-variance: {id}");
        let original = read(&dir.join("candidate.rs"))?;
        let mut row = CandidateRow {
            id: id.clone(),
            negative_control: id == CONTROL,
            candidate_sha256: sha256_hex(original.as_bytes()),
            normalization_changed: false,
            functions: Vec::new(),
            implementation_code_lines: 0,
            tests_declared: original.matches("#[test]").count(),
            own_tests: None,
            oracle: None,
            own_tests_against_reference: None,
            mutants: Vec::new(),
            detail: String::new(),
        };
        let normalized = match normalize(&opts.scratch, &module_name(&id), &original) {
            Ok(text) => text,
            Err(e) => {
                row.detail = e;
                rows.push(row);
                continue;
            }
        };
        row.normalization_changed = normalized != original;
        let candidate = match split_candidate(&normalized) {
            Ok(c) => c,
            Err(e) => {
                row.detail = e;
                rows.push(row);
                continue;
            }
        };
        row.functions = defined_functions(&candidate.implementation);
        row.implementation_code_lines = code_lines(&candidate.implementation);

        // Oracle: the candidate's functions, the reference's tests, laws, and
        // every other nomos-core test; the candidate's own module is absent.
        write(
            &module,
            &splice(reference, &candidate.implementation, None)?,
        )?;
        let (oracle, _) = cargo_test(opts, ORACLE)?;
        let viable = oracle.outcome != Outcome::DoesNotCompile;
        row.oracle = Some(oracle);

        // Own tests: the candidate's functions and its own module.
        write(
            &module,
            &splice(reference, &candidate.implementation, Some(&candidate.tests))?,
        )?;
        row.own_tests = Some(cargo_test(opts, OWN_TESTS)?.0);

        // The candidate's tests against the reference, then each mutant.
        let mut with_tests = reference.to_owned();
        append_tests(&mut with_tests, &candidate.tests);
        write(&module, &with_tests)?;
        let (baseline, _) = cargo_test(opts, OWN_TESTS)?;
        let baseline_passes = baseline.outcome == Outcome::Accepted;
        row.own_tests_against_reference = Some(baseline);
        for mutant in mutants {
            row.mutants.push(run_mutant(
                opts,
                reference,
                &candidate,
                mutant,
                baseline_passes,
            )?);
        }
        write(&module, reference)?;

        if viable {
            compiled.push((module_name(&id), candidate.implementation.clone()));
        }
        rows.push(row);
    }

    // The comparison: every candidate that compiled, with the reference.
    let comparison_path = opts
        .scratch
        .join("crates/core/nomos-core/tests")
        .join(format!("{COMPARISON_TEST}.rs"));
    write(&comparison_path, &comparison_source(&compiled))?;
    let (compare, stdout) = cargo_test(
        opts,
        &[
            "--test",
            COMPARISON_TEST,
            "--",
            "--nocapture",
            "--exact",
            "compare",
        ],
    )?;
    let comparison = stdout
        .lines()
        .find_map(|l| l.strip_prefix(RECORD_PREFIX))
        .map(|json| serde_json::from_str::<Value>(json).map_err(|e| e.to_string()))
        .transpose()?;
    if comparison.is_none() {
        eprintln!("generator-variance: the comparison produced no record: {compare:?}");
    }
    Ok((rows, comparison))
}

fn run_mutant(
    opts: &Options,
    reference: &str,
    candidate: &Candidate,
    mutant: &Mutant,
    baseline_passes: bool,
) -> Result<MutantRow, String> {
    let patch = read(&mutant.patch)?;
    let touched = touched_item(&patch);
    let mut row = MutantRow {
        id: mutant.id.clone(),
        wrong_behavior: mutant.wrong_behavior.clone(),
        candidate_tests_name_it: touched
            .as_deref()
            .is_some_and(|item| mentions(&candidate.tests, item)),
        touched_item: touched,
        outcome: MutantOutcome::PatchDoesNotApply,
        failed_tests: Vec::new(),
    };
    let module = opts.scratch.join(REFERENCE_PATH);
    write(&module, reference)?;
    let applied = Command::new("git")
        .args(["apply", "--whitespace=nowarn"])
        .arg(&mutant.patch)
        .current_dir(&opts.scratch)
        .output()
        .map_err(|e| format!("git apply: {e}"))?;
    if !applied.status.success() {
        return Ok(row);
    }
    let mut mutated = read(&module)?;
    append_tests(&mut mutated, &candidate.tests);
    write(&module, &mutated)?;
    let (result, _) = cargo_test(opts, OWN_TESTS)?;
    row.failed_tests = result.failed_tests;
    row.outcome = match result.outcome {
        Outcome::DoesNotCompile => MutantOutcome::Unviable,
        _ if !baseline_passes => MutantOutcome::BaselineFails,
        Outcome::Rejected => MutantOutcome::Caught,
        Outcome::Accepted | Outcome::RanNothing => MutantOutcome::Survived,
    };
    Ok(row)
}

fn summarize(rows: &[CandidateRow]) -> Summary {
    let generated: Vec<&CandidateRow> = rows.iter().filter(|r| !r.negative_control).collect();
    let accepted = |o: Option<Outcome>| o == Some(Outcome::Accepted);
    let mut mutants_caught: BTreeMap<String, usize> =
        MUTANTS.iter().map(|m| ((*m).to_owned(), 0)).collect();
    for row in &generated {
        for m in &row.mutants {
            if m.outcome == MutantOutcome::Caught {
                *mutants_caught.entry(m.id.clone()).or_default() += 1;
            }
        }
    }
    let negative_control = rows
        .iter()
        .find(|r| r.negative_control)
        .map(|r| ControlOutcome {
            own_tests: r.own(),
            oracle: r.oracle(),
            as_required: r.own() == Some(Outcome::Accepted)
                && r.oracle() == Some(Outcome::Rejected),
        });
    Summary {
        n: generated.len(),
        compiled: generated
            .iter()
            .filter(|r| r.oracle().is_some_and(|o| o != Outcome::DoesNotCompile))
            .count(),
        own_tests_accepted: generated.iter().filter(|r| accepted(r.own())).count(),
        oracle_accepted: generated.iter().filter(|r| accepted(r.oracle())).count(),
        own_accepted_oracle_rejected: generated
            .iter()
            .filter(|r| accepted(r.own()) && r.oracle() == Some(Outcome::Rejected))
            .count(),
        mutants_caught,
        negative_control,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        CANDIDATE_MODULE, CONTROL, MUTANTS, code_lines, comparison_source, defined_functions,
        failed_tests, mentions, module_name, reference_span, splice, split_candidate, touched_item,
    };

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn fixtures() -> PathBuf {
        root().join("tests/fixtures/generator-variance")
    }

    fn reference() -> String {
        std::fs::read_to_string(root().join(super::REFERENCE_PATH)).unwrap()
    }

    const SMALL: &str = "/// Docs.
pub fn assess_evidence(r: &FileCondition, e: &FileEvidence) -> Assessment {
    helper()
}

fn helper() -> Assessment {
    Assessment::Satisfied
}

pub fn assess_collection(r: &FileCondition, c: &Collection) -> Assessment {
    Assessment::Satisfied
}

pub fn assess(c: &Condition, o: &[Observation]) -> Assessment {
    Assessment::Satisfied
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_holds() {
        assert_eq!(assess_evidence(r, e), Assessment::Satisfied);
    }
}
";

    #[test]
    fn a_candidate_splits_at_its_test_module_and_the_module_is_renamed() {
        let c = split_candidate(SMALL).unwrap();
        assert!(c.implementation.ends_with("Assessment::Satisfied\n}\n"));
        assert!(!c.implementation.contains("#[cfg(test)]"));
        assert!(c.implementation.contains("fn helper()"));
        assert!(
            c.tests
                .starts_with(&format!("#[cfg(test)]\nmod {CANDIDATE_MODULE} {{\n"))
        );
        assert!(c.tests.ends_with("    }\n}\n"));
        assert!(!c.tests.contains("mod tests"));
        assert!(c.tests.contains("fn it_holds()"));
    }

    #[test]
    fn a_candidate_without_tests_or_a_function_is_refused() {
        let no_tests = SMALL.split("#[cfg(test)]").next().unwrap();
        assert!(split_candidate(no_tests).is_err());
        let no_assess = SMALL.replace("pub fn assess(", "pub fn judge(");
        assert!(
            split_candidate(&no_assess)
                .unwrap_err()
                .contains("pub fn assess(")
        );
        let trailing = format!("{SMALL}\nfn after() {{}}\n");
        assert!(split_candidate(&trailing).is_err());
    }

    #[test]
    fn the_reference_span_is_exactly_the_three_functions() {
        let text = reference();
        let (start, end) = reference_span(&text).unwrap();
        let span = &text[start..end];
        assert!(span.starts_with("/// Judges collected evidence against a requirement."));
        assert!(span.contains("pub fn assess_collection("));
        assert!(span.ends_with("        (true, None, None) => Assessment::Indeterminate(Reason::NoObservation),\n    }\n}\n"));
        assert!(
            text[..start].ends_with("    }\n}\n\n"),
            "the span starts after `impl Assessment`"
        );
        assert!(text[end..].starts_with("\n/// Every Assessment of a Canon's Conditions"));
    }

    #[test]
    fn splicing_replaces_the_functions_and_keeps_the_reference_tests() {
        let text = reference();
        let c = split_candidate(SMALL).unwrap();
        let spliced = splice(&text, &c.implementation, Some(&c.tests)).unwrap();
        assert_eq!(spliced.matches("\npub fn assess(").count(), 1);
        assert!(spliced.contains("fn helper()"));
        assert!(
            !spliced.contains("let mut least_failure"),
            "the reference body is gone"
        );
        assert!(spliced.contains("pub struct Report"));
        assert!(
            spliced.contains("fn the_evidence_truth_table_is_exhaustive_and_never_indeterminate()")
        );
        assert!(spliced.ends_with(&c.tests));
        let without = splice(&text, &c.implementation, None).unwrap();
        assert!(!without.contains(CANDIDATE_MODULE));
        assert_eq!(without.matches("mod tests {").count(), 1);
    }

    #[test]
    fn every_fixture_candidate_splits_and_splices() {
        let text = reference();
        let mut seen = Vec::new();
        for entry in std::fs::read_dir(fixtures()).unwrap() {
            let dir = entry.unwrap().path();
            let Ok(candidate) = std::fs::read_to_string(dir.join("candidate.rs")) else {
                continue;
            };
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let c = split_candidate(&candidate).unwrap_or_else(|e| panic!("{name}: {e}"));
            let spliced = splice(&text, &c.implementation, Some(&c.tests)).unwrap();
            assert_eq!(
                spliced.matches("pub fn assess_evidence(").count(),
                1,
                "{name}"
            );
            assert_eq!(
                spliced
                    .matches(&format!("mod {CANDIDATE_MODULE} {{"))
                    .count(),
                1
            );
            assert!(dir.join("NOTES.md").is_file(), "{name} has no NOTES.md");
            seen.push(name);
        }
        seen.sort();
        assert_eq!(seen.first().map(String::as_str), Some(CONTROL));
        assert_eq!(seen.len(), 6, "{seen:?}");
    }

    #[test]
    fn a_patch_names_the_item_its_hunk_sits_in() {
        let patches = root().join("tests/semantic-mutants/patches");
        let items: Vec<Option<String>> = MUTANTS
            .iter()
            .map(|m| {
                touched_item(&std::fs::read_to_string(patches.join(format!("{m}.diff"))).unwrap())
            })
            .collect();
        assert_eq!(
            items,
            [
                Some("assess".into()),
                Some("Report".into()),
                Some("assess".into())
            ]
        );
        assert_eq!(touched_item("no hunk"), None);
    }

    #[test]
    fn mentions_matches_whole_words_only() {
        assert!(mentions("let a = assess(&c, &o);", "assess"));
        assert!(!mentions("assess_collection(&r, &c)", "assess"));
        assert!(!mentions("reassess(x)", "assess"));
        assert!(mentions("Report::assess(&c, &o)", "Report"));
        assert!(!mentions("Reporter", "Report"));
    }

    #[test]
    fn the_small_helpers_read_what_they_claim() {
        let c = split_candidate(SMALL).unwrap();
        assert_eq!(
            defined_functions(&c.implementation),
            ["assess_evidence", "helper", "assess_collection", "assess"]
        );
        assert_eq!(code_lines("/// doc\n\nfn a() {\n    // c\n}\n"), 2);
        assert_eq!(module_name("cand-1"), "cand_1");
        assert_eq!(module_name("1st"), "_1st");
        let stdout =
            "test a::b ... ok\ntest a::c ... FAILED\ntest result: FAILED. 1 passed; 1 failed;";
        assert_eq!(failed_tests(stdout), ["a::c"]);
    }

    #[test]
    fn the_comparison_lists_every_implementation() {
        let source = comparison_source(&[
            ("reference".into(), "pub fn assess() {}\n".into()),
            ("cand_1".into(), "pub fn assess() {}\n".into()),
        ]);
        assert!(source.contains("mod reference {"));
        assert!(source.contains("mod cand_1 {"));
        assert!(source.contains("assess: cand_1::assess }"));
        assert!(source.contains("fn compare()"));
    }
}
