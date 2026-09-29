# Result: generator-variance

- **Experiment.** `generator-variance`, designed in milestone `02-verification-foundation`.
- **Status.** **Run** on 2026-09-29, at commit `576d50f`. Exploratory: its result never gates and no threshold is set.
- **Question.** When several independent generations implement one kernel property, how often does a fixed oracle reject a candidate its own generated tests accept, and how much do the candidates differ?
- **Rival.** Generated tests and the fixed oracle agree, and [ADR 0015](../../../adr/0015-generator-verifier-development-model.md) §5's rule that generated tests are regression tests is unnecessary caution.
- **Outcome.** On $n = 5$, the disagreement rate is zero. Every candidate passed its own tests and the fixed oracle, and no two candidates, nor any candidate and the reference, disagree on any input compared. The candidates' tests are weaker detectors than the oracle, though. All five catch `SM-ASSESS-001`. One of five catches `SM-ASSESS-003`, and only by accident. None reaches `SM-ASSESS-002`, which patches code the pack did not ask for. The rival holds for acceptance on this property and fails for detection, so §5 stays as written.

## Why

The digest's oracle results are paper-reported, for Java and Python, with older models: oracles biased toward the implementation shown, tests that compile 42% of the time, property-based tests correct for 21% of documented properties. None of it is a Nomos measurement. This experiment produces one, on one property, and reports it as a research finding.

## Harness

The design, as planned before any candidate existed:

1. **Property.** One pure kernel function with a fixed oracle that predates every candidate: the exhaustive truth table for `assess` on the first resource family, from `03-assessment-kernel`, plus the semantic mutants `SM-ASSESS-001` and `SM-ASSESS-002`.
2. **Candidates.** $n \ge 5$ independent generations of the function from the same specification text: separate sessions, no shared context, at least one human-written. Each generation also writes its own tests, as it would in ordinary work.
3. **Normalization.** `cargo fmt` on every candidate.
4. **Judgment.** Every candidate is run against its own tests, against the fixed oracle, and against the active semantic mutants (its own code patched). Each result is recorded with a receipt.
5. **Variance.** For candidates that pass the fixed oracle, record the differences that remain: lines, public shape, and which unlisted inputs they disagree on under a generated-input comparison.

## Measurements

| Measure | Meaning |
| --- | --- |
| Own-test acceptance | Candidates accepted by the tests they wrote |
| Oracle acceptance | Candidates accepted by the fixed oracle |
| Disagreement rate | Accepted by own tests, rejected by the oracle |
| Mutant catch rate per candidate's tests | Whether the generated tests would have caught the semantic mutants |
| Residual variance | Differences among oracle-accepted candidates on generated inputs |

## Negative Control

One candidate is written to be wrong in a way the oracle names (the `SM-ASSESS-001` behavior) with tests that agree with it. The harness must report it as own-tests-accepted and oracle-rejected. If it does not, the harness is not measuring.

## Decision Rule

None that gates. The rates are recorded here, with their $n$, and the digest's open question on generated-test bias is updated. If the disagreement rate is zero on $n \ge 5$, that is a finding worth recording as well, and ADR 0015 §5 stays as written until it recurs.

## Unchecked, When It Runs

- Properties other than the one chosen.
- Generators other than the ones used, on other days.
- Anything about candidates that do not compile; they are counted and set aside.

## What Ran

| Field | Value |
| --- | --- |
| Reference | `crates/core/nomos-core/src/assessment.rs`: `assess_evidence`, `assess_collection`, and `assess`, 65 code lines |
| Pack | [`tests/fixtures/generator-variance/pack.md`](../../../../tests/fixtures/generator-variance/pack.md): spec §8, the opening of `reconciliation.md`, ADR 0005's decision, N13, and the type definitions of `assessment.rs`, `observation.rs`, `condition.rs`, and `resource.rs`, with no function bodies, no tests, and no module documentation of `assessment.rs` |
| Candidates | [`tests/fixtures/generator-variance/`](../../../../tests/fixtures/generator-variance/README.md): `cand-1` to `cand-5`, generated on 2026-09-29 from the pack by five separate agent sessions with no shared context and no repository access; `cand-0`, the negative control |
| Fixed oracle | Every `nomos-core` test that predates the candidates, 50 in all: the module's `mod tests` (the exhaustive truth table and the named tests of `SM-ASSESS-001`, `002`, and `003`), `tests/laws.rs`, `tests/conformance.rs`, the other modules' unit tests, the compile-fail tests, and the doc tests |
| Mutants | `SM-ASSESS-001`, `002`, and `003` from `tests/semantic-mutants/corpus.toml` |
| Harness | `cargo xtask generator-variance`, `crates/bin/nomos-xtask/src/generator_variance.rs`; its splicing logic has unit tests |
| Comparison | A test file generated into the scratch copy, with the reference and every candidate that compiles in its own module; proptest 1.11 with ChaCha, seed `0x67 0x76 0x01` then zeros |
| Record | [generator-variance.json](generator-variance.json), the harness's output at commit `576d50f`; its digest, `5c4252d3…`, is the `stdout_sha256` of the `generator-variance` receipt |
| Receipts | `verification/receipts/2026-09-29-generator-variance.ndjson` |

For each candidate the harness formats the file with `rustfmt` and splits it at its `#[cfg(test)] mod tests`. It splices the functions into one scratch copy of the workspace in place of the reference's three and keeps the rest of the module, the reference's tests included. It renames the candidate's test module `mod candidate_tests`. It then runs, with `--locked --offline`:

1. **Own tests.** The candidate's functions with `candidate_tests`, filtered to that module.
2. **Oracle.** The candidate's functions without `candidate_tests`: `cargo test -p nomos-core --no-fail-fast`.
3. **Tests against the reference.** The reference's functions with `candidate_tests`, as the baseline for the mutants.
4. **Mutants.** The reference patched with each mutant, with `candidate_tests`. A mutant is caught when a candidate test fails.

## Results

| Measure | Result, $n = 5$ |
| --- | --- |
| Compiled | 5 of 5 |
| Own-test acceptance | 5 of 5 |
| Oracle acceptance | 5 of 5, each passing all 50 oracle tests |
| Disagreement rate | 0 of 5 |
| Mutant catch rate of the candidates' tests | `SM-ASSESS-001`: 5 of 5. `SM-ASSESS-002`: 0 of 5, not exercised. `SM-ASSESS-003`: 1 of 5, by accident |
| Residual variance | None. No disagreement on any input, over 15 pairs among the reference and the five candidates |

| Candidate | Code lines | Private helper | Tests | Own tests | Oracle | Tests on reference | `001` | `002` | `003` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `cand-1` | 57 | `content_assessment` | 14 | Accepted, 14 | Accepted, 50 | Accepted | Caught | Survived | Survived |
| `cand-2` | 65 | `judge_content` | 17 | Accepted, 17 | Accepted, 50 | Accepted | Caught | Survived | Caught |
| `cand-3` | 60 | `assess_content` | 17 | Accepted, 17 | Accepted, 50 | Accepted | Caught | Survived | Survived |
| `cand-4` | 63 | `assess_content` | 16 | Accepted, 16 | Accepted, 50 | Accepted | Caught | Survived | Survived |
| `cand-5` | 60 | `assess_content` | 14 | Accepted, 14 | Accepted, 50 | Accepted | Caught | Survived | Survived |
| `cand-0`, control | 46 | None | 3 | Accepted, 3 | **Rejected**, 8 of 50 failed | Rejected, 1 of 3 failed | Baseline fails | Baseline fails | Baseline fails |

Normalization changed none of the six files: every candidate arrived formatted.

**The mutants, one at a time.**

- **`SM-ASSESS-001`**, a failed Observation assessed as a Variance. Every candidate wrote tests for N13 and every one caught it, with three or four tests each: least-failure ordering, a single Observation agreeing with `assess_collection`, and the denied-read test.
- **`SM-ASSESS-002`**, an Indeterminate Assessment in the Plan's input. The patch is in `Report::variances`. The pack asked for three functions and no `Report`, and no candidate's tests name `Report`, so the tests never run the patched code. The survival says nothing about the tests' quality; the mutant is out of their reach.
- **`SM-ASSESS-003`**, a stale Observation assessed as a Variance, with staleness meaning a window that ends before instant 1. No candidate wrote a freshness test: all five read freshness as outside `assess`. `cand-2` caught the mutant because its default test Observation has a window from instant 0 to instant 0. The three tests that failed are about single Observations, failures beside evidence, and other resources. Every window the other four candidates' tests build ends at 1 or later, and the defect passed their tests.

## Negative Control Outcome

As required: own tests accepted (3 of 3), oracle rejected. The eight oracle tests that failed were `a_failed_observation_is_indeterminate` and `a_denied_read_is_neither_satisfied_nor_variance` (SM-ASSESS-001's named test and its companion), `an_indeterminate_assessment_does_not_hide_a_variance`, `indeterminate_assessments_do_not_reach_the_plan`, and `two_failures_give_the_same_reason_in_either_order` in the module, law 2 `only_failures_is_indeterminate_with_the_least_failure`, the conformance test `collected_evidence_is_judged_when_it_agrees_and_indeterminate_when_it_does_not`, and `action::tests::a_fresh_variance_fails_and_a_failed_read_establishes_nothing`.

The comparison saw it too: the control disagrees with the reference and with every candidate on 96,832 of the 383,912 exhaustive cases and on 835 of the 4,096 generated ones. Minimized by proptest's shrinking, the input is one failed Observation, `PermissionDenied`, of the Condition's own resource: the reference gives `Indeterminate(CollectionFailed(PermissionDenied))`, the control `Variance(Missing)`. The comparison can see a disagreement, so its silence on the five generated candidates carries information.

Its mutant rows are `baseline-fails`: its own tests fail against the unpatched reference, so a failure against a mutant means nothing. Against `SM-ASSESS-001` every one of its tests passes, because that mutant is its defect.

## Residual Variance

| Domain | Cases | Disagreeing pairs among the reference and `cand-1` to `cand-5` |
| --- | --- | --- |
| `assess_evidence`, exhaustive: 4 requirements × 5 evidences (3 digests, 2 sizes) | 20 | 0 of 15 |
| `assess_collection`, exhaustive: 4 requirements × 9 collections | 36 | 0 of 15 |
| `assess`, exhaustive: every list of up to 3 Observations over 2 paths, 9 collections, and 2 provenances, against 8 Conditions | 383,912 | 0 of 15 |
| `assess`, generated: lists of up to 10 Observations over 3 paths, 3 digests, 2 sizes, 3 collectors, and any window end | 4,096 | 0 of 15 |

What differs is form, not behavior. Each candidate adds one private helper for content or digest comparison; the reference has none. All five collect the resource's evidence into a `Vec` and compare every item with the first. The reference streams the Observations once and returns `Conflicting` at the first mismatch. Code lines run from 57 to 65; the reference has 65. The public shape is the pack's three signatures in every case.

**The ambiguous points converged.** Each `NOTES.md` states its readings, and all five chose the same ones:

| Point | Every candidate's reading | Reference |
| --- | --- | --- |
| A failed Observation beside collected evidence | The failure contributes nothing and contradicts nothing; the collected evidence decides | Agrees: `a_failure_beside_evidence_does_not_contradict_it` and the module documentation |
| What counts as conflicting | Collected `FileEvidence` values that differ in any way, size included, even when every one would satisfy the Condition; conflict takes precedence over failures | Agrees: `assess` compares whole `FileEvidence` values and returns `Conflicting` before it looks at failures |
| Freshness | Not judged in `assess`: no policy and no reference instant reach it, and provenance plays no part | Agrees: `a_matching_observation_is_satisfied_whatever_its_age`; the module documentation places freshness before assessment |
| Several failures | The least under `CollectionFailure`'s derived order | Agrees; the pack's `Reason` documentation states it |

The pack left out the module documentation that states the first and third readings. The candidates reached them from `Reason`'s documentation ("Every Observation of the resource failed", "Collected Observations of the resource disagree") and from the signatures, which carry no freshness policy.

## Deviations From the Design

- **No human-written candidate.** The design asked for at least one; none was available.
- **One generator.** All five candidates came from one generator, in five separate sessions on one day, with no shared context and no repository access.
- **Mutants patch the reference, not the candidate.** The design said "its own code patched". The corpus patches are anchored to the reference's lines and do not apply to a candidate's code. The harness instead measures whether each candidate's tests catch the reference's semantic mutants, the measure the design's table names.
- **Three mutants, not two.** `SM-ASSESS-003` joined the corpus after the design and is included.
- **A wider oracle.** The design named the truth table and the mutants. The oracle run is every `nomos-core` test that predates the candidates, 50 in all, a superset. Tests of other modules pass for any candidate that compiles, unless, like the `action` test the control failed, they call `assess`.
- **One receipt for the run.** The design asked for a receipt per result. The per-candidate results are in the JSON record, and the receipt of the whole run carries the record's digest.
- **The control was written with the reference in view.** It is not a blind generation, and it is excluded from $n$.

## Findings

- **Acceptance agreed; detection did not.** On this property, the candidates' tests and the fixed oracle agreed on every candidate. As detectors, the tests fell short of the oracle: four of five would have passed the freshness defect, and none can see a defect in `Report`. The oracle catches all three mutants by name (`cargo xtask mutants semantic`). This is the gap ADR 0015 §5 describes: a test the generator wrote shows what the code does, and what the tests fail to state goes unchecked.
- **The one freshness catch was an accident.** A test catches a defect by design only when it states the property. `cand-2` caught `SM-ASSESS-003` because its default window ends at instant 0, not because any test states that age does not matter. A catch rate counts both kinds the same, which is one reason this record sets no threshold.
- **The candidates converged, and the reference agrees.** Five sessions made the same choice on every ambiguous point. That is weak evidence of a clear pack and equally consistent with one generator's shared prior; one generator cannot separate the two.

## Unchecked

- Properties other than this one: larger functions, functions with state, and properties that the type documentation constrains less than it constrains `assess`.
- Generators other than the one used, human authors, and other days. Convergence from one generator is not independence.
- A pack without the reference's type documentation. `Reason`'s documentation carries part of the reference's reading, and a pack without it was not tried.
- Candidates that do not compile: none did, so the `does-not-compile` path ran on no real candidate. The unit tests cover the splicing, not that path.
- Inputs outside the comparison's domains. Lists longer than 3 are sampled, not enumerated, and the domains use 2 or 3 paths, 3 digests, and 2 sizes.
- Whether a candidate's tests *exercise* a mutant's code. The record states only whether the tests *name* the item the patch sits in (`assess` or `Report`), a textual check, not coverage.
- The workspace's lint gate on the spliced candidates. `cargo clippy` was not run on them; the candidates report running it in their own scratch crates.
- The bounded-verifier harnesses, which were not run against any candidate.

## Decision

None that gates. The disagreement rate is zero on $n = 5$, recorded here as a finding, and ADR 0015 §5 stays as written. The mutant results argue for keeping it: tests written by a generator agreed with the oracle about what they checked and left unchecked what they did not state.
