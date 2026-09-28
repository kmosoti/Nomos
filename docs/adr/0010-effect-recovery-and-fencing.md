# ADR 0010: Effect Recovery, Authority Fencing, and Disruption Budgets

- **Status.** Proposed
- **Date.** 2026-09-28
- **Candidate.** `recovery-authority` in the 2026-09-28 research snapshot (recommendations `effect-recovery`, `fencing`, `budget`)

## Context

Three draft claims were stronger than their mechanisms. First, a generation check before dispatch does not fence the effect that follows it (counterexample `fence-race`). Second, an idempotency key derived from an Action's content suppresses every later repair of the same file (counterexample `dedup-scope`). Third, N9 bounded the world rather than admission (counterexample `budget-not-world`). The corrected statements are in [fencing-and-idempotency.md](../formal/fencing-and-idempotency.md) and [invariants.md](../formal/invariants.md). Research open questions `fencing`, `recovery-effects`, and `freshness` end here.

## Decision

Proposed. The formal documents follow it as a working definition until it is accepted.

### 1. Keys Name Executions

An idempotency key names one execution of one Action within one Plan. Retries of that delivery share it. A later reconciliation run that plans the same repair gets a new key. The semantic digest of an Action is its content identity, never its deduplication key.

### 2. Every Effect Declares a Recovery Class

| Class | Example | After a lost outcome |
| --- | --- | --- |
| Inspectable, repeatable | Atomic file replacement | Re-observe; re-apply only from a new Variance |
| Inspectable, not repeatable | A service restart with an observable job | Re-observe; no retry until the job is Settled |
| Not inspectable | An opaque effect | Stays unresolved; operator resolution; policy may refuse the class |

An unknown outcome stays unknown (N10) and keeps its reservation.

### 3. The Fence Is Checked Where the Effect Is Admitted

The Cell admits effects through one serialized path, for Loom Plans and standalone local Plans alike. The generation is re-checked inside that path, atomically with effect admission. A superseding Plan whose footprint conflicts with an in-flight effect is not accepted until that effect is Settled or cancelled: the Cell drains rather than races. Authority is validated before a fence is persisted.

### 4. Budgets Bound Admission

N9 is stated over admission: a disruptive Action is admitted only if the nodes unavailable in a fresh snapshot, plus those reserved, plus those it would disrupt, fit the budget. Admissions per failure domain are serialized. Involuntary failures past the budget are reported and pause further disruption. They are never denied.

### 5. A Plan Is Applied Against Its Assumptions

A Plan records the Observations its Actions depend on. If one of those changes before an Action is admitted, the Action is re-planned from fresh Observations. Unrelated drift does not invalidate the Plan, and irrelevant telemetry is never part of the check.

### Alternatives

- **Reject rather than drain.** Viable for short effects, and kept as a policy option. Draining is the default because rejecting a superseding Plan while old work runs leaves the Cell stuck on stale authority.
- **Content-addressed keys.** Rejected by `dedup-scope`.

### Acceptance Criteria

- **`05-transition-kernel`, `effect-recovery` and `scheduler-admission`.** No duplicate live effect, no suppressed repair, and no unsafe retry across duplicated, delayed, and dropped receipts. No budget oversubscription under concurrent admission.
- **`05-transition-kernel` model.** A TLA+ model of competing authority in which a paused old Action, resumed after a newer Plan is accepted, causes no conflicting unreserved effect.
- **Before remote execution, `fence-interleavings`.** The same property across Loom transport races.

## Consequences

- The N5 theorem loses its proviso once §3 is implemented and modeled.
- **Revisit trigger.** Reopen if draining stalls a Cell measurably, or if a provider can enforce fencing itself, for example by accepting a generation token on its own API.
