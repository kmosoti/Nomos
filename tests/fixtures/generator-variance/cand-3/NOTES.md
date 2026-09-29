# Candidate 3 Notes

## Ambiguities and How They Were Resolved

- **A failure beside collected evidence.** The `Reason::CollectionFailed` doc says "Every Observation of the resource failed," and `Conflicting` covers only *collected* Observations that disagree. So when at least one Observation of the resource was collected and some others failed, no Reason applies, and I judge the agreed collected evidence. The failures are ignored. A stricter reading would call any failure insufficient evidence and return Indeterminate. The test `a_failure_beside_collected_evidence_does_not_hide_it` pins my reading as a regression test.
- **What counts as disagreement.** Two collected Observations conflict when their `FileEvidence` values differ in any way, size included. Present evidence with the same digest and a different size counts as a conflict. When any two collected Observations disagree, the result is `Conflicting`, even if failures are also present. Conflict takes precedence over failure.
- **Freshness.** The formal definition makes freshness part of "sufficient", but the signature takes no policy and no reference instant. I therefore apply no freshness check, and windows and collectors play no part in the judgment. Two collectors that saw different content at different times still count as conflicting.
- **Choosing among failures.** When every Observation failed, the reported failure is the minimum under the derived `Ord` of `CollectionFailure`, as the `Reason` doc requires. That minimum is independent of order.
- **Size under `Content::Exactly`.** Only the digest is compared. Size is never a Variance.

## Scratch Crate Result

The pack's types are reproduced in a scratch crate with the `kani` cfg attributes stripped. All 17 tests passed: 17 passed, 0 failed, 0 ignored. `cargo clippy --all-targets -- -D warnings` was clean, and `rustfmt --check` was clean after formatting. I also ran clippy on non-test code with `unwrap_used`, `expect_used`, `panic`, and `indexing_slicing` enabled. It found nothing in the candidate. It flagged only two indexing sites in the pack's own `resource.rs`.

The tests that draw their oracle from outside the code are these. The truth table for `assess_evidence` was written independently. The N13 checks are `a_failed_collection_is_indeterminate` and `only_failures_never_assess_as_satisfied_or_variance`. The `partial-assessment` counterexample is covered by `assessments_do_not_aggregate_across_resources`. Three tests check metamorphic relations: single Observation equals `assess_collection`, duplicates change nothing, and every permutation gives the same result. The rest are regression tests.
