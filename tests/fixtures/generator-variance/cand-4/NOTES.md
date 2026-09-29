# Candidate 4 Notes

## Ambiguities and Resolutions

- **Mixed failed and collected Observations of one resource.** `Reason::CollectionFailed` applies only when *every* Observation failed. When at least one collected, I ignore the failures: a failed Observation carries no evidence, so it cannot contradict a collected one, and the agreed collected evidence is judged. Test: `a_failure_beside_collected_evidence_does_not_hide_it`.
- **What "disagree" means for `Conflicting`.** I compare whole `FileEvidence` values, not the verdicts they produce. Absent against Present, two different digests, or the same digest with different sizes all count as Conflicting, even when every one of them would satisfy the Condition (for example `Content::Any` with two digests). The soundness requirement defines sufficiency as evidence that "does not contradict another Observation of r", which is about the evidence, not the verdict. Conflicting takes precedence over any failures that are also present.
- **Freshness.** The spec requires evidence that is "fresh under the policy", but these signatures take no policy and no current instant, so windows and provenance are not examined. Agreeing Observations from different collectors or windows are sufficient, and a later window does not override an earlier, disagreeing one; both yield Conflicting.
- **Smallest failure.** The smallest failure is chosen under the derived `Ord` of `CollectionFailure`, which is declaration order: PermissionDenied < TimedOut < Unsupported < Io.
- **Path matching.** Paths match by `ResourcePath` equality, which is textual, as its docs state. A prefix or a child path is a different resource.
- `Vec` is used in `assess` for the collected evidence, so the module's existing `Vec`, `Content`, and `Digest` imports are all used and trigger no unused-import warnings.

## Scratch Crate Results

I reproduced the pack's types verbatim in a `no_std` + `alloc` scratch crate at `scratch/`, with `candidate.rs` spliced into `assessment.rs` through `include!`.

- `cargo test --offline`: 16 passed, 0 failed, 0 ignored.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo clippy --lib -- -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`: clean, so non-test code does not panic.
- `rustfmt --check`: clean.

The tests' oracles are an independently written truth-table reference for `assess_evidence`, invariant N13 (failed collection is Indeterminate, and a denied read is never Satisfied or a Variance), the `partial-assessment` counterexample, and metamorphic relations: a single Observation agrees with `assess_collection`, and reversing or rotating every triple of a small domain leaves the result unchanged. The remaining tests are regression tests of the interpretive choices above.
