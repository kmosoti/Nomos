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
| Tests notice a wrong tooling implementation | Implementation conformance | 8 | `cargo-mutants` on three `nomos-xtask` modules | The run itself | passed, with 4 survivors classified equivalent or excluded | [mutation-calibration](../research/2026-09-28-typed-core/results/mutation-calibration.md) | Modules not mutated; mutants the tool does not generate |
| Semantic mutants caught | Implementation conformance | 8 | `cargo xtask mutants semantic` | The fixture corpus earns every outcome once | not applicable: nine planned mutants, none active | same file, `mutants-semantic` | Everything, until a kernel exists |

## Invariants

| Invariant | Specification correctness | Implementation conformance | Environment correspondence | Record |
| --- | --- | --- | --- | --- |
| N1 Trace does not mutate | not run | planned: `07-substrate-conformance` | planned: `07-substrate-conformance` | none |
| N2 Successful Enforce converges | not run | planned: `05-transition-kernel`, `bounded-convergence` | not run | none |
| N3 Re-enforcing converged Canon mutates nothing | not run | planned: `05-transition-kernel`; semantic mutant `SM-ASSESS-003` | not run | none |
| N4 No Action before hard dependencies | not run | planned: `04-warp-kernel`, `warp-truth-table`; `SM-WARP-001` to `SM-WARP-003` | not applicable | none |
| N5 Stale fenced Plan cannot execute | planned: `05-transition-kernel` TLA+ `Fencing` | planned: `SM-TRANSITION-003` | not run | none |
| N6 Succeeded requires verified postconditions | planned: `05-transition-kernel` TLA+ `ActionLifecycle` | planned: `SM-TRANSITION-001` | not run | none |
| N7 Events never mutated after append | not run | planned: `05-transition-kernel` in-memory store | not run | none |
| N8 Cipher plaintext never persists | not run | planned: `03-assessment-kernel` compile-fail and sentinel tests | not run | none |
| N9 Budgets never intentionally exceeded | planned: `05-transition-kernel` model | planned: `scheduler-admission` | not run | none |
| N10 Unknown outcomes stay unknown | not run | planned: `SM-TRANSITION-002`, `effect-recovery` | not run | none |
| N11 Losing Loom keeps Observations | not run | not run | not run | none |
| N12 Canon compilation is deterministic | not run | planned: `06-canon-artifact`, `canonical-encoding`; permutation relation | not applicable | none |
| N13, proposed: unknown evidence does not imply noncompliance | not run | planned: `03-assessment-kernel`, `assessment-algebra`; `SM-ASSESS-001`, `SM-ASSESS-002`; Indeterminate monotonicity relation | not run | none |

## Formal Claims

| Claim | Where stated | Specification correctness | Implementation conformance | Record |
| --- | --- | --- | --- | --- |
| Reconciliation reaches a fixed point or reports the bound | [reconciliation.md](reconciliation.md) | counterexample models reproduced (`final-check`, `fingerprint-limits`, `bound-not-termination`) | planned: `bounded-convergence` | `cargo xtask research reproduce`; the [typed-core snapshot](../research/2026-09-28-typed-core/README.md) |
| Warp activation and cycle witnesses | [warp.md](warp.md) | counterexample models reproduced (`activation-missing`, `noop-anchor`, `scc-not-cycle`) | planned: `warp-truth-table` | same |
| Fencing at the moment of effect | [fencing-and-idempotency.md](fencing-and-idempotency.md) | counterexample model reproduced (`fence-race`) | planned: `05-transition-kernel` | same |
| Event Log ordering and fold | [event-log.md](event-log.md) | not run | planned: `05-transition-kernel` | none |

The counterexample models are Python models of the draft documents, reproduced in Rust by `nomos-xtask`. They show the drafts, read literally, admitted the bad outcomes, and that the corrected text does not. They are not tests of Nomos code, because there is none.

## How to Update

Run the check through `cargo xtask receipts record <check-id> --out verification/receipts/<file>.ndjson -- <command>`, or write the result card, then change the row and cite the record. A row without a record says `not run`. Removing a record or loosening a check is a verifier change and goes in its own commit with a `Trust-Boundary: verifier` line.
