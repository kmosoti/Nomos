# ADR 0009: Warp Prerequisite, Activation, and Reservation Semantics

- **Status.** Proposed
- **Date.** 2026-09-28
- **Candidate.** `warp-gates` in the 2026-09-28 research snapshot (recommendations `warp-semantics`, `durable-refresh`, `scheduler`)

## Context

Spec §62 has long listed edge semantics as needing an ADR. The research review then found that the draft readiness formula let an unchanged configuration trigger a service restart (counterexample `activation-missing`). It found that a satisfied prerequisite had no vertex to hang an edge on (finding `noop-anchor`), and that a crash between a file replacement and its restart lost the restart for good (counterexample `lost-refresh`). The corrected definitions live in [warp.md](../formal/warp.md) as working definitions. This ADR is what makes them binding once their truth tables have run. Research open questions `trigger` and `solver` end here.

## Decision

Proposed. [warp.md](../formal/warp.md) follows it as a working definition until it is accepted.

### 1. Readiness Is Two Questions

A vertex is ready when it is **Startable** and **Activated**. Startable is edge by edge: every `requires` source Succeeded, every `after` and `on_change` source terminal. Activated is decided over the `on_change` sources as a group: the vertex has none, or at least one succeeded with a verified change. Disabled is a group result, when every source is terminal and none changed. A single unchanged source never vetoes a changed one.

### 2. Every Edge Resolves to One State

| Unit | States |
| --- | --- |
| `requires` edge | Waiting, Satisfied, Blocked |
| `after` edge | Waiting, Satisfied |
| `on_change` group | Waiting, Activated, Disabled |

A vertex runs, is Skipped as terminal without change, or is Blocked, with no fallthrough.

### 3. Satisfied Prerequisites Are Anchors

Every resource named by a `requires`, `after`, or `on_change` edge has a vertex. A resource with a Variance or an Obligation gets its Action. A Satisfied resource gets an anchor, Succeeded on entry and unchanged. An Indeterminate resource gets an Indeterminate anchor, terminal on entry, not Succeeded, and unchanged, and its effect is per edge: a `requires` edge from it is Blocked, because a prerequisite that might be missing is not met; an `after` edge from it is Satisfied, because `after` is ordering; and an `on_change` edge from it is a source without a verified change, so it never activates and counts toward Disabled.

### 4. Change Consumption Survives a Crash

Where a service can report the configuration revision it loaded, that revision is a Condition and no trigger is needed. Where it cannot, the refresh is an Obligation, recorded durably before the change that creates it and discharged by the refresh. A transient `on_change` alone is never the only record of a required refresh.

### 5. Reservations Last Until Settlement

An Action holds its conflict keys from dispatch until its effect is Settled, past a timeout and past a satisfied re-observation. Admission is serialized, one path per Cell, and selection is deterministic and greedy over the reserved set. Maximality is claimed only for downward-closed constraints, and fairness is a separate policy. Constraint solving, with a Boolean satisfiability (SAT) or satisfiability modulo theories (SMT) solver, is not used until a precise objective and a benchmark justify it.

### Alternatives

- **All-sources activation.** Rejected: no use case yet, and it makes an unchanged file veto a changed one.
- **Transient triggers only.** Rejected by `lost-refresh`.
- **Release reservations on timeout.** Rejected: a timed-out effect may still be mutating.

### Acceptance Criteria

- **`04-warp-kernel`, `warp-truth-table`.** Exhaustive predecessor-outcome tables over every source outcome, the Indeterminate anchor included, for each edge kind. Also the one-changed-one-unchanged case, the cycle-witness case, and insertion-order invariance, all against production functions and an independent reference evaluator. The Indeterminate-anchor rows in §3 are part of the table the tests must reproduce, not a detail left to the implementation.
- **`05-transition-kernel`, `refresh-recovery` and `scheduler-admission`.** No declared Obligation lost at any crash boundary, and no conflicting overlap or budget oversubscription under adversarial interleavings.

## Note, 2026-09-28: The Warp Kernel's Truth Tables

Milestone `04-warp-kernel` produced the tables this ADR's first acceptance criterion asks for, against production functions and an independent reference evaluator ([record](../research/2026-09-28-typed-core/results/warp-truth-table.md)). Two points the text above left open are recorded as working definitions and stand until this ADR is accepted or amended:

- **A Skipped vertex is met.** A vertex whose `on_change` group is Disabled is Skipped: terminal without change and, as §2 says, not failed. Its `requires` dependents see a met prerequisite, its `after` dependents see a terminal source, and its `on_change` dependents see a source without a change. The alternative reading, Skipped as "ended other than Succeeded", would block every prerequisite chain through a refresh that had no reason to run.
- **An `after` edge has no Blocked state.** The kernel gives `after` edges their own two-state type. A Kani harness on the vertex-resolution predicate found that a state type shared with `requires` let an `after` edge marked Blocked reach the predicate, which then reported the vertex Ready with an unmet edge. The type now makes the row unwritable.

The second acceptance criterion, `refresh-recovery` and `scheduler-admission`, still waits for `05-transition-kernel`; this ADR stays Proposed.

## Consequences

- Spec §62's edge-semantics question closes when this ADR is accepted.
- **Revisit trigger.** Reopen if a real Canon needs all-sources activation, or if derived footprints for `package` and `systemd_unit` Actions cannot be computed in core.
