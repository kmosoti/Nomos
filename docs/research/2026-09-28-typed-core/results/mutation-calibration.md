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

## Re-Run, 2026-09-29

The grounding plan's exit audit found that neither run above has a receipt, and the first ran on a tree with uncommitted changes. This section records the same three modules mutated again, after milestones `03` to `07`, under receipts.

| Field | Value |
| --- | --- |
| Command | `cargo mutants -p nomos-xtask -f manifest.rs -f strict_json.rs -f snapshot.rs --no-shuffle -j 3 --gitignore true --shard K/4 --output <dir>`, for `K` from 0 to 3 |
| Configuration | `.cargo/mutants.toml` as committed: the `with Verified::` and `::for_harness` exclusions, `--locked`, a timeout multiplier of 3 |
| Trees | Commits `21f5602` (shards 0 and 1), `9835e5a` (shard 2), and `c7e167e` (shard 3), each clean; they differ from one another only in receipt files, and none changed a mutated file since `997df85` |
| Machine | The development container, 4 cores, 3 parallel jobs |
| Receipts | `verification/receipts/2026-09-29-mutation-calibration.ndjson` (the failed first attempt) and `2026-09-29-mutation-calibration-shards.ndjson` (one receipt per shard) |

### What Happened

- **The first attempt tested nothing.** The command above, as run 2 wrote it, passes `-C --locked`. `.cargo/mutants.toml` has added `--locked` since, so cargo refused the flag given twice and the unmutated baseline did not build. Its receipt is kept, as failed. The re-run drops `-C --locked` and takes `--locked` from the configuration alone.
- **The run is split in four.** Each mutant now runs the whole `nomos-xtask` test suite, and the suite takes about 155 seconds on the unmutated tree: `the_repository_corpus_parses_and_has_no_active_mutant_that_is_not_caught` applies every semantic mutant and builds the workspace for each. At two jobs, 41 mutants come to about 65 minutes, longer than the development container stays up between tool calls; a detached attempt died when the container was reclaimed, and so did the first run of shard 3. `cargo mutants --shard K/4` splits the list deterministically; each shard ran to completion under its own receipt, with its own baseline.

### Measurements

| Measure | Run 2 (2026-09-28) | Re-run |
| --- | --- | --- |
| Mutants generated | 41 | 41 |
| Baseline test | 4 s | 155 s |
| Wall time | 3 min 23 s on two jobs | 78 minutes on three jobs, in four shards of 23, 21, 20, and 15 minutes |
| Caught | 34 | 34 |
| Unviable | 3 | 3 |
| Timed out | 0 | 0 |
| Survived (`missed`) | 4 | 4 |

### Survivors

| Mutant | Class | Disposition |
| --- | --- | --- |
| `strict_json.rs:67` `expecting` returns `Ok(Default::default())` | **excluded**: message text | As in run 1: it changes the text of an error nobody asserts on |
| `strict_json.rs:93` `visit_string` returns `Ok(Default::default())` | **equivalent** under `serde_json` | As in run 1: `serde_json` delivers strings through `visit_str` |
| `strict_json.rs:97` `visit_unit` returns `Ok(Default::default())` | **equivalent** | As in run 1: `Value::default()` is `Value::Null`, which `visit_unit` returns |
| `strict_json.rs:101` `visit_none` returns `Ok(Default::default())` | **equivalent** under `serde_json` | As in run 1: `deserialize_any` never calls `visit_none` |

The three unviable mutants are the ones run 2 recorded: a deleted `!` in `manifest.rs` `verify` and two `Default::default()` replacements that do not type-check.

### Findings

- **The calibration reproduces.** The counts equal run 2's, and the four survivors are run 2's four, each in the class run 1 gave it. Three milestones of new tooling left these three modules' tests as strong as the calibration left them.
- **Mutation now costs minutes per mutant.** The tooling crate's own suite includes the semantic-mutant runner's self-test, so every syntactic mutant of `nomos-xtask` pays for 29 semantic mutants. A targeted test selection per mutated module would restore seconds per mutant; it is a verifier change and is not made here.
- **The scheduled job cannot finish.** CI's `mutation` job runs `cargo mutants -p nomos-xtask --no-shuffle` over the whole crate: 716 mutants today, at about three minutes each on one job, roughly 36 hours, against GitHub's six-hour job limit. It has never run on its schedule. Sharding it across a job matrix, or selecting tests per module, would bring it back within the limit; the choice is the owner's, since the workflow is protected.

## Decision

`cargo-mutants` is adopted as configured in `.cargo/mutants.toml`, on a weekly schedule and by hand ([CI](../../../../.github/workflows/ci.yml), job `mutation`), never on the required path. Every survivor is classified with the table above's classes. The calibration is rerun on the Assessment Kernel when it exists. The re-run of 2026-09-29 reproduces run 2 under receipts; the scheduled job's cost is an open question for the owner (see its Findings). Semantic mutants, chosen from the invariants, are the other half and are on the required path ([corpus](../../../../tests/semantic-mutants/README.md)).
