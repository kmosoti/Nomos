# Candidate 5 Notes

## Ambiguities and Resolutions

- **Mixed failed and collected Observations of one resource.** `Reason::CollectionFailed` applies when *every* Observation failed, and "sufficient" excludes only Observations that contradict one another. A failed collection contradicts nothing, so the collected Observations decide and the failures are ignored. The more conservative reading, which returns Indeterminate whenever any collection failed, was rejected.
- **What counts as disagreement.** Two collected Observations conflict when their `FileEvidence` values differ in any way: presence, digest, or size. The same digest with a different size is a contradiction. Disagreement is `Indeterminate(Conflicting)` even when each piece of evidence alone would satisfy the Condition, for example two different digests under `Content::Any`, because the soundness rule requires non-contradictory evidence before any verdict.
- **Freshness.** The formal model requires fresh evidence "under the policy", but `assess` receives no policy and no reference instant, and the kernel reads no clock. Freshness is therefore not judged here. Provenance, both collector and window, is ignored, and Observations that agree from different collectors are one piece of evidence.
- **Several failures.** The Reason is the minimum under `CollectionFailure`'s derived `Ord`, which is its declaration order (`PermissionDenied < TimedOut < Unsupported < Io`), so the result is independent of arrival order.
- **Path matching** is `ResourcePath` equality, which is textual, so `/etc` and `/etc/a/b` are other resources for `/etc/a`.
- `Vec` is used for the collected evidence, which also keeps the module's existing `alloc::vec::Vec` import from being unused.

## Test Results

The scratch crate at `cand-5/scratch/` reproduces the pack's types verbatim with `assessment.rs` = the pack's type definitions + `candidate.rs`. In it, `cargo test --offline` reported 14 passed, 0 failed, 0 ignored; `cargo clippy --offline --all-targets -- -D warnings` reported no findings; and `rustfmt --check` passed on the module. The toolchain was cargo 1.94.1, not the project's pinned 1.98.1. Each test's comment names its oracle: the truth table, N13, the Reason documentation, metamorphic relations (a single Observation reduces to `assess_collection`, and the result is the same for every permutation and subset), or, for the mixed-failure choice, that it is a regression test of an interpretation.
