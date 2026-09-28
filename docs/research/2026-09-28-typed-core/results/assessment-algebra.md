# Result: assessment-algebra

- **Experiment.** `assessment-algebra`, milestone `03-assessment-kernel`.
- **Question.** Are three Assessment outcomes per Condition, kept unaggregated, enough for the first resource family, and is uncertainty never converted into satisfaction or Variance?
- **Outcome.** Yes for the file family. Three outcomes were enough; no case needed a fourth. Every path from insufficient evidence leads to Indeterminate with a reason, shown by an exhaustive table, five laws on generated inputs, two Kani harnesses, and three semantic mutants. N13 is numbered ([spec §58](../../../PROJECT-SPEC.md#58-core-safety-invariants)).

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1; proptest 1.11; trybuild 1.0.121; Kani 0.68.0 with its bundled C Bounded Model Checker (CBMC); cargo-mutants 27.1.0 |
| Crate | `nomos-core`, `no_std` with `alloc`, allowlist empty |
| Production functions | `assess_evidence`, `assess_collection`, `assess`, `Report::assess`, `Report::variances` in `crates/core/nomos-core/src/assessment.rs` |
| Receipts | `verification/receipts/2026-09-28-assessment-kernel.ndjson`: `cargo-test`, `kani`, `mutants-semantic`, and the rest of the required path |

## What Was Built

`ResourcePath` (absolute, normalized, no zero byte) and `Digest` (32 bytes) with fallible constructors; `FileCondition` as `Absent | Present { content: Any | Exactly(Digest) }`, so an absent file with content is unwritable; `Condition` with private fields; `Observation` with `Collection::Collected(FileEvidence) | Failed(CollectionFailure)`, a collector name, and a window of two `Instant`s that the kernel compares and never reads from a clock; `Assessment` with `Variance { Missing, Unexpected, ContentDiffers }` and `Reason { NoObservation, CollectionFailed, Conflicting }`; `Report` sorted by resource, with `variances()` as the only Plan input; `Secret<T>` for N8.

Sufficiency, as implemented: at least one collected Observation of the resource, and every collected one agrees. A failure beside evidence does not contradict it; whether it should count against sufficiency is a policy for `05-transition-kernel`. Freshness is a policy on the window applied before assessment, never inside it, so a matching Observation is Satisfied whatever its age.

## Evidence

| Layer | Harness | Result |
| --- | --- | --- |
| 4, truth table | `the_evidence_truth_table_is_exhaustive_and_never_indeterminate`: 3 requirements by 3 evidences, 9 rows, asserted one by one | Passed; no row Indeterminate |
| 4, unit | Every `CollectionFailure` against every requirement; denied read; nobody observed; another resource observed; conflict; agreement; failure beside evidence; age; `partial-assessment`; Plan input; order | 20 tests passed |
| 2, compile-fail | `private-fields`, `unvalidated-path`, `absent-with-content`, `secret-display`, `secret-serialize`, `secret-expose-unused` | 6 cases fail to compile with the expected diagnostic |
| 6, laws | `tests/laws.rs`, 512 cases each, ChaCha seeds `4e313301` to `4e313305` then zeros | 5 laws passed after the fix below |
| 11, Kani | `evidence_assessment_is_sound` (Satisfied iff a reference `holds`, never Indeterminate; covers Satisfied and Variance) and `a_failed_collection_is_indeterminate` (covers PermissionDenied and Present) | 2 harnesses verified, 4 of 4 covers satisfied, 2.8 s |
| 8, semantic mutants | `SM-ASSESS-001`, `SM-ASSESS-002`, `SM-ASSESS-003` | Each caught by its named test |
| 8, `cargo-mutants` | `cargo mutants -p nomos-core -- -- --skip the_type_boundaries_hold`, 44 s | 74 mutants: 37 caught, 27 unviable, 10 survived, classified below |

The laws are stated from spec §3 and N13 with a reference predicate written apart from the implementation, so they are evidence for the invariant, not regression tests (ADR 0015 §5). The Kani harnesses carry the same reference predicate; a harness with `holds` replaced by `true` fails under Kani, which shows the harness is not vacuous beyond its cover checks.

## Counterexample Found

Law 4, permutation invariance of the Report, failed on its first run: with two failed Observations of one resource, the reported reason was whichever failure arrived first, so reversing the Observations changed the Report. Minimized input: Condition `/r/0` Absent; Observations `Failed(PermissionDenied)` and `Failed(TimedOut)`. The fix reports the least failure under `CollectionFailure`'s ordering. The fixture is `two_failures_give_the_same_reason_in_either_order` in `assessment.rs`, with its provenance in its doc comment: found by, seed, input, property, fix.

## Survivor Classification

| Mutant | Class | Disposition |
| --- | --- | --- |
| Five `Display` or `Debug` impls returning `Ok(())` (`PathError`, `DigestError`, `WindowError`, `ResourcePath` twice) | **survived**: text nobody asserted | `display_forms_say_what_they_are` and the window test now assert them |
| `\|` to `^` in `Digest::from_hex` | **equivalent** | `hi << 4` and `lo` have disjoint bits; `\|` and `^` agree on every input |
| `holds` to `true`, `holds` to `false`, and each harness body to `()` in `verification.rs` | **excluded**: outside the test suite | `cfg(kani)` code is compiled only by `cargo kani`, which fails on the `true` mutation; `.cargo/mutants.toml` now excludes the file, and the exclusion is a declared verifier change |

Unviable mutants are `Default::default()` replacements on types with no `Default`, `Box::leak` replacements, and two arithmetic mutants the tool reports as not compiling. They carry no information.

## Negative Controls

- Every `CollectionFailure` against every requirement is Indeterminate with that failure, in the unit table, in law 2, and in Kani.
- A denied read is neither Satisfied nor a Variance.
- A Variance beside an Indeterminate is reported (`partial-assessment`), and the Plan input holds the Variance only.
- Each active semantic mutant is caught; each compile-fail case fails.
- The Kani harness fails on a wrong reference predicate.

## Unchecked

- Every resource family but files; every requirement but presence and one digest.
- The Obligation clause of N13: no Plan exists to take input.
- Whether a failure beside evidence should count against sufficiency; a policy for `05-transition-kernel`.
- Freshness: the window is carried and compared, and no policy uses it yet.
- ADR 0013 §1's Trace observe-only compile-fail case, which needs the Substrate port's capability split. Moved to the milestone that writes the port.
- Bounded typed bindings: not needed for the file family and not measured.
- `Secret` closes the formatting and serde paths; a caller can still copy the bytes out of `expose` into any sink, which Phase 6 `secret-nondisclosure` tests on the real sinks.

## Decision

Three outcomes stand (ADR 0005, note added). Kani is the bounded verifier (ADR 0015, note added). N13 is an invariant (spec §58, `invariants.md`). The next kernel, `04-warp-kernel`, takes `Report::variances` as its input shape.
