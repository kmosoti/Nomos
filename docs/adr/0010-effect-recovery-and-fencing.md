# ADR 0010: Effect Recovery, Authority Fencing, and Disruption Budgets

- **Status.** Accepted for the Cell, 2026-09-28, by the project owner, on the evidence of milestone `05-transition-kernel`. The Loom-transport criterion, `fence-interleavings`, is still owed
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

## Note, 2026-09-28: Working Definitions for the Transition Kernel

Milestone `05-transition-kernel` implements §1, §3, and §4 for one Cell and records these working definitions.

- **§1, the key.** An idempotency key is the Plan's identifier and generation, the reconciliation iteration, and the resource. Retries of one delivery share it; the next iteration or the next run plans the same repair under a new key, so a completed key never suppresses a later repair (counterexample `dedup-scope`).
- **§3, atomic re-check.** `step` handles one input at a time, so a Cell's admission path is serialized by construction. The fence is re-checked in the same `step` that admits the effect and records its intent, so no acceptance can fall between check and admission.
- **§3, draining.** A superseding Plan is accepted at once: the fence moves, so the old Plan can admit nothing more, and the old Plan's Actions that were not yet dispatched are Cancelled. Effects already dispatched keep running and keep their reservations. The new Plan observes and plans only once every effect is Settled. That is stricter than §3, which waits only for conflicting effects; it keeps the oscillation rule of [reconciliation.md](../formal/reconciliation.md#oscillation) true by construction and costs latency only when an effect is slow.
- **§4, the budget predicate.** Selection skips an Action $a$ when some failure domain $f$ has $\lvert U_f \cup R_f \cup D_f(a) \rvert > k_f$, where $U_f$ comes from the budget snapshot in the policy, $R_f$ is the nodes disrupted by every effect still holding a reservation plus every Action already chosen in this selection, and $D_f(a)$ is the nodes $a$ would disrupt. When the budget snapshot is older than the policy's maximum age at the instant of admission, no Action that disrupts a node is admitted. One selection runs per `step`, so admissions are serialized.

**Evidence.** [effect-recovery](../research/2026-09-28-typed-core/results/effect-recovery.md) and [scheduler-admission](../research/2026-09-28-typed-core/results/scheduler-admission.md) meet the first acceptance criterion for one Cell: no duplicate live effect, no suppressed repair, no unsafe retry, and no budget oversubscription. For the second, the `Fencing` model, checked by the TLA+ model checker (TLC), shows a paused check followed by a late effect violating N5 (the `split` control) and the atomic admission not, and `RefreshRecovery` shows no conflicting unreserved effect under superseding authority with crashes ([kernel-conformance](../research/2026-09-28-typed-core/results/kernel-conformance.md)). The third criterion, `fence-interleavings`, needs Loom. The ADR stays Proposed, for the Cell, until the project owner accepts it.

## Note, 2026-09-28: Acceptance

The project owner accepted this ADR for the Cell on 2026-09-28, on the records of [effect-recovery](../research/2026-09-28-typed-core/results/effect-recovery.md), [scheduler-admission](../research/2026-09-28-typed-core/results/scheduler-admission.md), and [kernel-conformance](../research/2026-09-28-typed-core/results/kernel-conformance.md). The decision accepted is §1 to §5 as the note above defines them, and one definition replaces §3's wording: a superseding Plan is accepted at once, so the old Plan admits nothing more, and it observes and plans only once every effect is Settled, not only the conflicting ones. That is stricter than §3 as written, keeps the oscillation rule true by construction, and costs latency only while an effect is slow; the revisit trigger below covers the case where it stalls a Cell. The third acceptance criterion, `fence-interleavings` across Loom transport races, is not met and waits for Loom; until it runs, the acceptance covers one Cell and its local authority.

## Consequences

- The N5 theorem loses its proviso once §3 is implemented and modeled.
- **Revisit trigger.** Reopen if draining stalls a Cell measurably, or if a provider can enforce fencing itself, for example by accepting a generation token on its own API.
