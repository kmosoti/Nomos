# Verification Matrix

Which property has been checked, by which layer of the [verification strategy](verification-strategy.md), with what result, and where the record is. Every row is filled from a receipt under `verification/receipts/` or a result record under `docs/research/<snapshot>/results/`, and from nothing else ([ADR 0007](../adr/0007-verification-gates.md) §1, [ADR 0015](../adr/0015-generator-verifier-development-model.md) §11).

Statuses: **not run** (no record exists), **planned** (a milestone names it; no record exists), **passed**, **failed**, **inconclusive** (a timeout or an operational failure), **not applicable**. A finite-model check is never called a proof; a status says which question it answers.

## Gates and Policies

These check the repository, not Nomos semantics. They are here because a semantic result depends on them: a matrix filled from records means nothing if the records or the layers could change unnoticed.

| Property | Question | Layer | Mechanism | Negative controls | Status | Record | Unchecked |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Dependency rule (ADR 0000) | Implementation conformance | 5 | `cargo xtask check-layers` over declared and resolved graphs | 16 fixture cases, each asserted by `Rule` code | passed | `verification/receipts/2026-09-28-verification-foundation.ndjson`, `check-layers` | Edges Cargo cannot see, such as a build script fetching code |
| Research evidence integrity | Implementation conformance | 5 | `cargo xtask research verify-all`, `research frozen` | Corrupted, symlinked, and rewritten snapshots, each asserted by `Code` or `FreezeCode` | passed | same file, `research-verify-all`; `research-frozen` | The meaning of the evidence |
| Core purity (ADR 0016) | Implementation conformance | 2, 3, 5 | `#![no_std]`, panic lints, `cargo xtask check-core-purity` | 14 fixture cases asserted by `PurityCode`; a doc comment naming `no_std` does not count | passed | same file, `check-core-purity`; [core-purity](../research/2026-09-28-typed-core/results/core-purity.md) | Effects reached through a caller-supplied function; anything a future allowlist entry does |
| Oracle changes declared (ADR 0015 §3) | Implementation conformance | 5 | `cargo xtask check-trust-boundary` | 13 fixture commits asserted by `TrustCode`, including a loosened gate test and a removed CI step | passed | same file, `check-trust-boundary`; [agent-proof-gate](../research/2026-09-28-typed-core/results/agent-proof-gate.md) | Semantic weakening inside a declared commit; a squash merge that drops the declaration |
| Receipts well formed (ADR 0015 §11) | Implementation conformance | 5 | `cargo xtask receipts validate` | 8 fixture cases asserted by `ReceiptCode`, `passed` without evidence among them | passed | same file, `receipts-validate` | A receipt written by hand with fabricated but well-formed evidence; no signature |
| Tests notice a wrong tooling implementation | Implementation conformance | 8 | `cargo-mutants` on three `nomos-xtask` modules; seven hand-written breaks of the four new gates | The run itself | passed: every non-equivalent tested mutant is caught, 4 survivors classified equivalent or excluded, and each hand-written break is caught by a named test | [mutation-calibration](../research/2026-09-28-typed-core/results/mutation-calibration.md) | Modules not mutated; mutants the tool does not generate |
| Semantic mutants caught | Implementation conformance | 8 | `cargo xtask mutants semantic` | The fixture corpus earns every outcome once | passed: `SM-ASSESS-001` to `003` and `SM-WARP-001` to `005` active and caught; three planned | `verification/receipts/2026-09-28-warp-kernel-semantics.ndjson`, `mutants-semantic` | The three planned mutants' kernel |

## Invariants

| Invariant | Specification correctness | Implementation conformance | Environment correspondence | Record |
| --- | --- | --- | --- | --- |
| N1 Trace does not mutate | not run | planned: `07-substrate-conformance` | planned: `07-substrate-conformance` | none |
| N2 Successful Enforce converges | not run | planned: `05-transition-kernel`, `bounded-convergence` | not run | none |
| N3 Re-enforcing converged Canon mutates nothing | not run | partial: a matching Observation is Satisfied whatever its age (`SM-ASSESS-003` caught); the fixed point itself is planned for `05-transition-kernel` | not run | [assessment-algebra](../research/2026-09-28-typed-core/results/assessment-algebra.md) |
| N4 No Action before hard dependencies | not run | passed for the frontier: edge and group truth tables, laws 1 to 3 (512 cases each, seeds `4e3401` to `4e3403`) against a reference by another route, three Kani harnesses, `SM-WARP-001` to `003` caught. The lifecycle side, that Running follows Ready, waits for `05-transition-kernel` | not applicable | [warp-truth-table](../research/2026-09-28-typed-core/results/warp-truth-table.md); receipts `cargo-test`, `kani`, `mutants-semantic` |
| N5 Stale fenced Plan cannot execute | planned: `05-transition-kernel` TLA+ `Fencing` | planned: `SM-TRANSITION-003` | not run | none |
| N6 Succeeded requires verified postconditions | planned: `05-transition-kernel` TLA+ `ActionLifecycle` | planned: `SM-TRANSITION-001` | not run | none |
| N7 Events never mutated after append | not run | planned: `05-transition-kernel` in-memory store | not run | none |
| N8 Cipher plaintext never persists | not run | partial: `Secret` has no `Display` or `Serialize` (compile-fail), redacts `Debug` (sentinel test), and `expose` must be used; every other sink is Phase 6 `secret-nondisclosure` | not run | [assessment-algebra](../research/2026-09-28-typed-core/results/assessment-algebra.md) |
| N9 Budgets never intentionally exceeded | planned: `05-transition-kernel` model | planned: `scheduler-admission` | not run | none |
| N10 Unknown outcomes stay unknown | not run | partial: the Warp frontier never reads a timed-out or failed trigger as no change (group truth table, law 3, Kani group harness, `SM-WARP-004` and `SM-WARP-005` caught); the lifecycle side, `SM-TRANSITION-002` and `effect-recovery`, is planned for `05-transition-kernel` | not run | [warp-truth-table](../research/2026-09-28-typed-core/results/warp-truth-table.md), revision of 2026-09-28 |
| N11 Losing Loom keeps Observations | not run | not run | not run | none |
| N12 Canon compilation is deterministic | not run | planned: `06-canon-artifact`, `canonical-encoding`. The permutation relation holds for the assessment Report (law 4, seed `4e313304`) and for the Warp graph, order, and frontier (law 2, seed `4e3402`) | not applicable | [assessment-algebra](../research/2026-09-28-typed-core/results/assessment-algebra.md) for the Report only |
| N13 Unknown evidence does not imply noncompliance | not run | passed for the file family: exhaustive truth table, laws 2 and 3 (512 cases each, seeds `4e313302` and `4e313303`), Kani harness `a_failed_collection_is_indeterminate` (covers satisfied), `SM-ASSESS-001` and `SM-ASSESS-002` caught. The Obligation clause waits for a Plan | not run | [assessment-algebra](../research/2026-09-28-typed-core/results/assessment-algebra.md); receipts `cargo-test`, `mutants-semantic`, `kani` |

## Formal Claims

| Claim | Where stated | Specification correctness | Implementation conformance | Record |
| --- | --- | --- | --- | --- |
| Reconciliation reaches a fixed point or reports the bound | [reconciliation.md](reconciliation.md) | counterexample models reproduced (`final-check`, `fingerprint-limits`, `bound-not-termination`) | planned: `bounded-convergence` | `cargo xtask research reproduce`; the [typed-core snapshot](../research/2026-09-28-typed-core/README.md) |
| Warp activation and cycle witnesses | [warp.md](warp.md) | counterexample models reproduced (`activation-missing`, `noop-anchor`, `scc-not-cycle`) | passed: `warp-truth-table`, the three counterexamples as unit tests against production functions | same; [warp-truth-table](../research/2026-09-28-typed-core/results/warp-truth-table.md) |
| Fencing at the moment of effect | [fencing-and-idempotency.md](fencing-and-idempotency.md) | counterexample model reproduced (`fence-race`) | planned: `05-transition-kernel` | same |
| Event Log ordering and fold | [event-log.md](event-log.md) | not run | planned: `05-transition-kernel` | none |

The counterexample models are Python models of the draft documents, reproduced in Rust by `nomos-xtask`. They show the drafts, read literally, admitted the bad outcomes, and that the corrected text does not. They are not tests of Nomos code, because there is none.

## How to Update

Run the check through `cargo xtask receipts record <check-id> --out verification/receipts/<file>.ndjson -- <command>`, or write the result card, then change the row and cite the record. A row without a record says `not run`. Removing a record or loosening a check is a verifier change and goes in its own commit with a `Trust-Boundary: verifier` line.
