# Candidate 1 Notes

## Ambiguities and How They Were Resolved

- **A failure next to collected evidence.** The pack does not say what happens when some Observations of the resource were collected and others failed. `Reason::CollectionFailed` applies when "every Observation of the resource failed", and a failed collection carries no claim about the resource, so it cannot contradict anything. I resolved it this way: when at least one Observation was collected, failures are ignored and the collected evidence decides. Only the test `a_failure_beside_collected_evidence_does_not_decide` covers this, and it is a regression test for that reading, not an oracle.
- **What counts as disagreement.** Collected evidence conflicts when the `FileEvidence` values differ, compared by full equality. So `Absent` against `Present`, two different digests, or the same digest with different sizes all give `Indeterminate(Conflicting)`. This holds even when both values would satisfy the Condition, for example two digests under `Content::Any`. Contradiction concerns the resource, not the verdict. Identical duplicates agree.
- **Conflict takes precedence over failures.** Any disagreement among collected evidence gives `Conflicting`, whatever failures sit beside it.
- **Freshness and time.** The pack gives no freshness policy and no clock, so windows and collectors play no part in the decision. Observations with different windows or collectors that disagree are still `Conflicting`. The code does not treat the later one as authoritative.
- **Order independence.** Arrival order cannot change the result. All collected evidence is compared with the first item, and equality is an equivalence relation. The reported failure is the `min` under `CollectionFailure`'s `Ord`.
- **The `Vec` import.** The host module keeps `use alloc::vec::Vec;`, so `assess` collects the evidence and the failures into `Vec`s. Without that use, `-D warnings` would reject the module for an unused import.
- **Panics.** Non-test code has no `unwrap`, `expect`, indexing, `debug_assert`, or `panic!`.

## Test Results

I tested in a scratch crate at `scratch/`: a `no_std` library with `alloc`, holding the pack's types copied verbatim and `candidate.rs` appended to `assessment.rs`.

- `cargo test --offline`: 14 passed, 0 failed, 0 ignored.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- Strict clippy (`unwrap_used`, `expect_used`, `panic`, `indexing_slicing`): nothing in `assessment.rs`. It does flag the pack's own `resource.rs`.

## Test Oracles

Each test comment names its oracle:

- A hand-written truth table.
- An independently written `satisfies` predicate for the soundness requirement.
- N13, the requirement that a failed collection is Indeterminate.
- The docs of `Reason`, `assess`, and `assess_collection`.
- The partial-assessment counterexample.
- Invariance under permutation, as a metamorphic relation.
