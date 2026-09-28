# Result: mutation-calibration

- **Experiment.** `mutation-calibration`, milestone `02-verification-foundation`.
- **Question.** What does `cargo-mutants` cost on this repository, what does it generate, and what do its survivors mean?
- **Outcome.** Measured. Two runs on three tooling modules; seven survivors classified; three tests written; four survivors named as equivalent or excluded. No score threshold is set, by [ADR 0015](../../../adr/0015-generator-verifier-development-model.md) §8.

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1 (`rust-toolchain.toml`), `cargo-mutants` 27.1.0 |
| Target | `nomos-xtask`, files `manifest.rs`, `strict_json.rs`, `snapshot.rs` |
| Command | `cargo mutants -p nomos-xtask -f manifest.rs -f strict_json.rs -f snapshot.rs --no-shuffle -j 2 -C --locked --output <dir>` |
| Machine | The milestone's cloud container, 2 parallel jobs; timings are indicative of this machine only |
| Run 1 tree | Commit `cb53404` with the trust-boundary work uncommitted; none of the mutated files differed from `cb53404` |
| Run 2 tree | The working tree that became commit `ee1621c`, with `.cargo/mutants.toml` and the new tests in place |

## Measurements

| Measure | Run 1 | Run 2 |
| --- | --- | --- |
| Mutants generated | 66 | 41 (25 excluded by pattern; see below) |
| Baseline | 27 s build, 2 s test | 26 s build, 4 s test |
| Wall time | 2 min 38 s | 3 min 23 s |
| Caught | 31 | 34 |
| Unviable | 28 | 3 |
| Timed out | 0 | 0 |
| Survived (`missed`) | 7 | 4 |

CI suitability: a run over three files costs about three minutes on two jobs; the whole tooling crate would cost proportionally more and the kernel crates are unmeasured. That is acceptable on a schedule and not on the required path, which is where it is.

## Survivor Classification

Every survivor of run 1, by hand:

| Mutant | Class | Disposition |
| --- | --- | --- |
| `manifest.rs:49` `&&` to `\|\|` in the line filter | **survived**: a real gap | A 64-character non-hex digest, or a digest with no name, passed the filter and fell through to `checksum-mismatch` or `manifest-names-missing-file` instead of `manifest-malformed`. Two tests added; caught in run 2 |
| `strict_json.rs:71` `visit_bool` returns `Null` | **survived**: a real gap | No test read a bool back through the strict parser. Test added; caught in run 2 |
| `strict_json.rs:75` `visit_i64` returns `Null` | **survived**: a real gap | No test read a negative integer back. Same test; caught in run 2 |
| `strict_json.rs:67` `expecting` returns `Ok(())` | **excluded**: message text | It changes the text of an error nobody asserts on. Asserting on message text would make the tests brittle for no gain in evidence |
| `strict_json.rs:97` `visit_unit` returns `Ok(Default::default())` | **equivalent** | `Value::default()` is `Value::Null`, which is what `visit_unit` returns. No test can distinguish them |
| `strict_json.rs:93` `visit_string` returns `Null` | **equivalent** under `serde_json` | `serde_json` delivers strings through `visit_str`, including escaped ones; `visit_string` is not reached. A test that called the visitor directly could distinguish it, and would test nothing the parser does |
| `strict_json.rs:101` `visit_none` returns `Null` | **equivalent** under `serde_json` | `serde_json` never calls `visit_none` from `deserialize_any`. Same reasoning |

Unviable mutants of run 1: 25 of the 28 replaced a `Verified<T>`-returning function with `Verified::new(..)`, `Verified::from(..)`, or `Verified::from_iter(..)`, which do not exist because `Verified<T>` is a `Result` alias. They cost a build attempt each and carry no information, so `.cargo/mutants.toml` now excludes the pattern `with Verified::`. The exclusion was written after every one of them had been recorded as unviable, which is the order rule 11 requires. The remaining three unviable in run 2 are a deleted `!` and two `Default::default()` replacements that do not type-check.

## Deliberate Breaks of the New Gates

`cargo-mutants` mutated only tooling that existed before this milestone. The four gates the milestone added were broken by hand, one semantic change each, on the final tree, and the tests that failed are named. The source was restored from Git after each break; `git status` was clean and `cargo fmt --check` passed afterward.

| Break | Tests that failed |
| --- | --- |
| Purity ignores the deny classes | `a_randomness_crate_is_rejected_by_class`, `an_async_runtime_is_rejected_by_class` |
| Purity accepts a missing `#![no_std]` | `dropping_no_std_is_rejected_even_when_a_doc_comment_mentions_it` |
| Trust skips the escape-hatch scan | `an_ignored_test_is_an_undeclared_escape_hatch`, `a_declared_escape_hatch_passes_for_review`, `every_commit_in_the_range_is_checked_and_named` |
| Trust requires no declaration for verifier paths | `an_undeclared_verifier_change_is_rejected`, `loosening_a_gate_test_is_an_undeclared_verifier_change`, `removing_a_ci_step_is_an_undeclared_verifier_change`, `a_declaration_without_a_matching_change_is_rejected` |
| Receipts drop the passed-without-evidence rule | `passed_without_evidence_is_rejected_by_schema_and_by_rule` |
| Receipts skip the schema | five tests, `a_missing_required_field_is_a_schema_violation` among them |
| Semantic runner reports a passing named test as caught | `every_outcome_is_reported_for_the_mutant_that_earns_it` |

Seven breaks, seven detections, each by a test whose name says what it protects. This is the section-27 control "mutation of existing gate semantics is detected by existing tests," for the gates this milestone added; the `cargo-mutants` runs above are the same control for the gates it inherited.

## Negative Control

The run itself: a mutation run in which every mutant is caught, on a suite that has never been mutated, would be suspicious. Run 1 produced seven survivors and 28 unviable mutants, three of the survivors were real, and the tests written for them changed the outcome in run 2. The tool distinguishes tested from untested code here.

## Unchecked

- Modules other than the three mutated; the kernel crates, which have no code.
- Mutants `cargo-mutants` does not generate: it replaces function bodies and flips operators; it does not reorder statements or change constants in general.
- Runtime on other machines.

## Decision

`cargo-mutants` is adopted as configured in `.cargo/mutants.toml`, on a weekly schedule and by hand ([CI](../../../../.github/workflows/ci.yml), job `mutation`), never on the required path. Every survivor is classified with the table above's classes. The calibration is rerun on the Assessment Kernel when it exists. Semantic mutants, chosen from the invariants, are the other half and are on the required path ([corpus](../../../../tests/semantic-mutants/README.md)).
