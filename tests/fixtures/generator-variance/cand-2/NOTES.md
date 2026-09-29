# Candidate 2 Notes

## Ambiguities and Resolutions

- **Some Observations failed and some were collected.** `Reason::CollectionFailed` is documented as "every Observation of the resource failed", and sufficiency is defined as "collected, fresh, and does not contradict another Observation of r". A failed collection carries no evidence, so it contradicts nothing. Resolution: failures beside collected evidence are ignored, and the collected evidence is judged. Only when nothing was collected is the result `CollectionFailed(min failure)`.
- **What "disagree" means for `Conflicting`.** Resolution: two collected `FileEvidence` values of the resource that are not equal conflict, whatever the requirement. That covers Absent against Present, different digests even under `Content::Any` (both would satisfy, but the evidence contradicts itself), and the same digest with a different size. The alternative, comparing per-Observation Assessments, would accept contradictory evidence whenever the requirement cannot tell the difference. I rejected it.
- **Freshness.** The pack gives no freshness policy and no "now", so freshness is not judged. Provenance (collector, window) plays no part: no later-supersedes-earlier rule, and identical evidence from any collectors or windows agrees.
- **Size.** `size` is ignored when the requirement is judged (only `Content::Exactly` digests are compared). It does count toward evidence equality for `Conflicting`.
- **Unused import.** `Vec` is in the module's imports, so `assess` collects the relevant collections into a `Vec` to avoid an unused-import warning under `-D warnings`.

## Scratch Results

The scratch crate at `scratch/` reproduces the pack's types with the candidate spliced in. It is std and has a `cfg(kani)` check-cfg added.

- `cargo test --offline`: 17 passed, 0 failed, 0 ignored.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `rustfmt --check`: clean.
- Hand-made mutants: `.min()` changed to `.max()`, failure mapped to `Variance`, and digest equality forced true. All three were caught. One no-op mutant survived, as expected.

The test oracles are an exhaustive truth table for `assess_evidence` written from the definitions, the N13 property over every requirement and failure, and order independence as a metamorphic relation over every permutation of small mixed sets. The mixed-failure and conflict tests are regression tests for the interpretations above.
