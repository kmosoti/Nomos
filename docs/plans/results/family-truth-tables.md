# Result: family-truth-tables

- **Experiment.** `family-truth-tables`, milestone `08-resource-families` of the [Phase 1 plan](../phase-1-masterless-cell.md).
- **Question.** Does each family's assessment match its written truth table on every listed case, with Indeterminate never collapsing to Variance?
- **Outcome.** Yes, for the seven families of [resource-families.md](../../formal/resource-families.md), on the cases the tables list, in `nomos-core`; and the mock passes every Substrate clause for each of the six Phase 1 families. Kani establishes the file and unit tables over every value of their bounded inputs. The Linux adapter serves only files, and refuses the rest.

## Context

| Field | Value |
| --- | --- |
| Commit | `6b0d1e4`, where every receipt was recorded |
| Oracle | The tables of resource-families.md, restated in the tests as independent predicates (`activity_holds`, `enablement_holds`, and the rows of each table), not read from `assess_*` |
| Harness | `crates/core/nomos-core/tests/families.rs`; `crates/core/nomos-core/src/verification.rs` under `cargo kani`; the per-family Substrate suite in `crates/bin/nomos-cell/tests/conformance/families.rs` |
| Receipts | `verification/receipts/2026-09-30-resource-families.ndjson` |

## The Tables

| Family | Test | Cases |
| --- | --- | --- |
| File and directory metadata | `metadata_rows_of_the_file_and_directory_tables` | Every stated and unstated owner, group, and mode against named and numbered observed owners and groups; the first differing field in the order owner, group, mode |
| File content before metadata | `content_is_judged_before_metadata` | A wrong digest with a wrong mode reports `content-differs` |
| Directory presence | `the_directory_table_presence_rows` | Absent and present against absent and present |
| Unit | `the_unit_table_is_exhaustive` | 3 activities by 3 enablements against 6 active states by 5 unit-file states: 270 cases, each axis named when it differs |
| Sysctl | `a_sysctl_value_is_compared_after_normalization` | Values equal after joining tokens with one space are Satisfied; others `value-differs` |
| User | `the_user_table` | Absent and present in each class, with and without home and shell, against absent and present accounts on both sides of ID 999 |
| Package | `the_package_table_is_exhaustive` | 3 requirements against not-installed, installed at two versions, and broken |
| Every family, N13 | `a_failed_collection_is_indeterminate_in_every_family` | Every family by every failure, `unavailable` included: Indeterminate with the failure, never a Variance |
| Every family, wrong evidence | `evidence_of_another_family_is_never_a_verdict` | Evidence of another family is Indeterminate (`wrong-family`), never Satisfied or a Variance |

## Bounded Verification

`cargo kani -p nomos-core`: 7 harnesses verified, 0 failures, and all 12 cover checks satisfied, so none is vacuous. The two new ones:

- `evidence_assessment_is_sound`: `assess_file` is Satisfied exactly when a direct reading of the file table holds, over every content requirement, every mode requirement stated or not, every digest, size, numeric owner and group, and mode, and it is never Indeterminate. Owner and group requirements are left out, because an account name allocates; they are compared by the same equality as the mode and are covered by the table test. The harness carries `#[kani::unwind(34)]`, whose sufficiency Kani checks.
- `unit_assessment_is_sound`: `assess_unit` against the unit table, over every requirement and every evidence.

`a_failed_collection_is_indeterminate` now ranges over file and unit requirements and every failure, `unavailable` included. `cargo kani -p nomos-warp`: 3 harnesses verified.

## The Mock Per Family

`the_mock_passes_the_suite_for_every_family` runs S1 to S9 for directory, file, package, sysctl, unit, and user; every clause passed for every family. S5 converges every requirement of the family from every starting point its table lists, 178 executions in all, and core judges each Satisfied on a new Observation, with `changed` true exactly when the evidence changed; the pairs the contract refuses (a user of the other class, enabling or disabling a static unit) are S7's. S7 checks each family's refusals: a requirement of another family, a directory with an entry asked to be absent, a file where a directory is and the reverse, a refresh of a file or a package, an unknown unit or parameter, a static unit asked to be enabled, and a system account asked to be regular.

`the_linux_adapter_refuses_the_families_it_does_not_serve` observes a key of each family but `file` as a failed collection, `unsupported`, and refuses every requirement on it without an execution.

## Semantic Mutants

| Mutant | Wrong behavior | Caught by |
| --- | --- | --- |
| `SM-FAMILY-001` | An unstated mode is judged against `0644` | `metadata_rows_of_the_file_and_directory_tables` |
| `SM-FAMILY-002` | A static unit satisfies `disabled` | `the_unit_table_is_exhaustive` |
| `SM-FAMILY-003` | A broken package satisfies `installed` | `the_package_table_is_exhaustive` |
| `SM-SUBSTRATE-005` | The mock removes a directory with entries | `the_mock_passes_the_suite_for_every_family` |
| `SM-TRANSITION-009` | A unit asked to refresh is converged, not restarted | `kernel::tests::a_unit_refreshes_exactly_when_its_source_acts` |
| `SM-WARP-007` | A resource with its own Variance is planned as a plain Action and Skipped | the same |

Each was run with `cargo xtask mutants semantic`; the receipt records the run.

## What This Does Not Establish

- Any family on a real host but files. The tables are core's; a host's evidence is `09` to `13`'s.
- Owner and group requirements under Kani.
- That the lists of starting points are every state a host can be in; they are the states the tables name.

## Decision Fed

The families' requirement, evidence, and assessment types are fixed for `09` to `13`, which implement their host adapters against this suite.
