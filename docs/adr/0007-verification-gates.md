# ADR 0007: Verification Scope and Development Gates

- **Status.** Accepted
- **Date.** 2026-09-28
- **Candidate.** `verification-gates` in the 2026-09-28 research snapshot (recommendations `formal`, `agent-proof`, `layer-enforcement`)

## Context

Nomos states twelve invariants and a set of formal claims, and none of them has been checked against code. The research snapshot pointed out two ways that gap turns into false confidence. A checker that passes without a negative control may not be checking anything. And a specification can be weakened until the check passes, which is the shortcut automated agents find first (finding `agent-spec-gaming`). ADR 0000 also claimed that Cargo enforces the dependency rule, which it does not (finding `layer-policy`).

Milestone `01-foundation-gates` built the first two gates. This ADR records the rules every gate follows, so later gates, from property tests to TLA+ models, are held to the same standard.

## Decision

### 1. A Claim Needs a Record, and a Record Needs a Run

A property counts as checked only when a verification record exists for it, with these fields:

| Field | Purpose |
| --- | --- |
| Property ID and specification revision | Exactly what was checked |
| Production function and harness | Where the model meets the implementation |
| Toolchain, dependency lock, target, tool version | Reproduction context |
| Bounds, seeds, fixtures, assumptions | Scope of exploration |
| Positive and negative-control results | Whether the harness tells correct from incorrect |
| Unchecked behavior | What the result does not cover |

`docs/formal/verification-matrix.md` is filled from records only. A planned check says *not run*. A timeout says *inconclusive*. A finite-model check is never called a proof.

### 2. Every Gate Has Negative Controls, Asserted by Reason

A gate ships with inputs it must reject, and each test asserts *why* it rejected them, through a stable code, not merely that it failed. A gate also ships with an input it must accept, so it cannot pass by rejecting everything. Restoring a corrupted input must make the gate pass again.

The codes cover policy failures: `Code` in the snapshot verifier, `Rule` in the layer checker, and `FreezeCode` in the snapshot freeze. Operational failures, such as a Git revision that is not available or `cargo metadata` that cannot run, carry a message and no policy code. They fail closed: a gate that could not check never reports success.

### 3. The Dependency Rule Is a Check

`cargo xtask check-layers` enforces ADR 0000 over the declared and the resolved dependency graphs, with no exemption by dependency kind, and CI runs it. Exceptions, if one is ever needed, are named by package and dependency kind in this ADR's successor, never added to the checker silently.

### 4. Research Evidence Is Verified and Frozen

`cargo xtask research verify-all` verifies every snapshot as a whole through one function the tests also call, and `cargo xtask research frozen` fails any change to an accepted snapshot. It runs on every push and pull request, not only on pull requests, because the checksum stage cannot tell an accepted manifest from a rewritten one and a direct push to the default branch would otherwise go unchecked. A push to the default branch is compared with its previous head; a revision that cannot be resolved fails the job.

### 5. Specifications Are Protected

AGENTS.md rule 11 binds: a check that passes because a postcondition was loosened, an assumption added, a test ignored, or code moved out of view proves nothing, and such changes are trust-boundary changes made in their own commit. A mechanical gate for the detectable part of this, experiment `agent-proof-gate`, is owed. Until it exists, rule 11 is enforced by review.

### 6. Verifiers Are Adopted One Question at a Time

Unit tests, exhaustive truth tables, property tests, and compile-fail tests come first, because they run against production functions. The first TLA+ model arrives with the transition kernel and is replayed through it. One Kani or Verus harness on one pure predicate precedes any wider adoption, with cover checks, because an assertion on an unreachable path passes vacuously. Assumptions in a harness are allowed when explicit, justified, reviewed apart from the proof, and shown non-vacuous. The `loom` model checker waits for a real shared-memory synchronization point.

### Alternatives

- **Trust green CI.** Rejected. CI was green while the snapshot verifier skipped its manifest and the layer rule was unchecked.
- **Adopt every verifier at once.** Rejected. Each tool adds toolchain, maintenance, and trust assumptions; a tool with nothing meaningful to check produces confidence, not evidence.
- **Ban every `assume`.** Rejected. Some assumptions are necessary. Hidden ones are the problem.

### Evidence

Milestone `01-foundation-gates`, pull request #2 merged as `0136a93`: `verify_snapshot` and `check-layers` with their negative controls, 58 passing tests on commit `55206a2`, and three deliberate breaks of the verifiers (ignoring optional dependencies, allowing any single port, skipping the manifest stage), each caught by the intended tests.

## Consequences

- Every later gate follows §1 and §2, and a pull request that adds a gate without a negative control is incomplete.
- Changing what a gate accepts is a trust-boundary change under rule 11.
- The verification matrix exists before anything in it is checked, and it starts empty by design.
- **Failure behavior.** A gate failure names its code and stops the run. A gate that cannot run, for example `cargo metadata` failing on a stale lockfile, fails closed; it never reports success.
- **Revisit trigger.** Reopen when the agent-proof gate lands, when a second verifier is adopted, or when a gate needs its first exception.

## Amendment, 2026-09-28

The agent-proof gate landed in milestone `02-verification-foundation`, and this ADR is amended narrowly, without changing its decisions:

- **§1.** The record is now a concrete format: a verification receipt under `verification/receipts/`, written by `cargo xtask receipts record` and held to `verification/receipt.schema.json` by `cargo xtask receipts validate`. The verification matrix is filled from receipts and result records only ([ADR 0015](0015-generator-verifier-development-model.md) §11).
- **§2.** Three more code families exist: `PurityCode` in the core purity check, `TrustCode` in the trust-boundary check, and `ReceiptCode` in the receipt validator. Each ships with negative controls asserted by code and a positive control.
- **§5.** The mechanical gate owed here is `cargo xtask check-trust-boundary`, in CI on every push and pull request. It detects and demands a declaration; semantic weakening remains a review item under rule 11, as this section said it would ([ADR 0015](0015-generator-verifier-development-model.md) §3).
- **Vocabulary.** Where this ADR says "pull request 1" it means milestone `01-foundation-gates`; the roadmap now uses stable milestone names.

Everything else in this ADR stands as written.
