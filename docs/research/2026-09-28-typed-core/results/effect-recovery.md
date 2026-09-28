# Result: effect-recovery

- **Experiment.** `effect-recovery`, milestone `05-transition-kernel`.
- **Question.** Do run-scoped idempotency keys deduplicate retries within one execution while allowing later repairs?
- **Outcome.** Yes. Duplicate receipts before, during, and after completion change nothing; a later run repairs the same drift under a new key; an effect with no readable postcondition stays unresolved; a receipt for nothing the kernel dispatched is recorded and ignored.

## Context

| Field | Value |
| --- | --- |
| Key | `nomos_core::effect::EffectKey`: the Plan, its generation, the iteration, and the resource ([ADR 0010](../../../adr/0010-effect-recovery-and-fencing.md) note) |
| Host | The mock performs an execution at most once per key and returns its first receipts to any repeat |
| Harness | `crates/bin/nomos-cell/tests/effect_recovery.rs`; every simulator step also asserts that no key is requested twice |
| Receipts | `verification/receipts/2026-09-28-transition-kernel.ndjson` |

## Scripts and Measurements

| Script | Test | Duplicate live effects | Suppressed repairs | Unsafe retries | Unresolved |
| --- | --- | --- | --- | --- | --- |
| Every receipt delivered twice, then every receipt once more after release | `duplicate_receipts_change_nothing` | 0: two executions, one write and one refresh | 0 | 0 | 0 |
| Converge, drift the file back, run a second Plan | `a_later_run_repairs_the_same_drift` | 0 | 0: two executions under two keys | 0 | 0 |
| The write completes, then the file cannot be read | `an_effect_without_a_readable_postcondition_stays_unresolved` | 0 | 0 | 0: one execution | 1: `Failed` with the write unknown |
| A receipt naming a key never dispatched | `a_receipt_for_an_unknown_execution_changes_nothing` | 0 | 0 | 0 | 0; the snapshot is unchanged and the Event says `Unknown` |
| The write is refused | `a_refused_request_withdraws_its_obligation` | 0 | 0 | 0 | 0; `Failed` with the write a known failure and its Obligation withdrawn |
| The write fails, then a second Plan | `a_blocked_refresh_reruns_once_its_trigger_is_repaired` | 0 | 0 | 0 | 0; the refresh, Blocked in the first run, runs in the second |

Duplicate receipts are ignored as `Late` while the Action is still verifying and as `Duplicate` once its effect is released. The kernel's released keys are its idempotency ledger; how long it keeps them is open (spec §62).

## Negative Controls

- **`dedup-scope`.** Semantic mutant `SM-TRANSITION-008` builds the key from the resource alone, as a content-derived key would. `a_later_run_repairs_the_same_drift` catches it: the simulator refuses a second request for the same key, and the mock would have returned the first run's receipt and done nothing.
- **An effect without settlement evidence stays unresolved.** The write whose postcondition cannot be read is never retried inside the run and ends as an unknown outcome, not a failure (N10).

## Unchecked

- The key's retention: the ledger grows without bound in memory.
- Retries of a delivery by a real transport; here a duplicate is a second delivery of the same receipt, not a second request on the wire.
- Recovery classes other than inspectable and repeatable (ADR 0010 §2): a service restart is modeled as a refresh with a settle-by instant, not as a job whose result the kernel queries.

## Decision

Keys name executions. ADR 0010 §1 holds as implemented, and the counterexample `dedup-scope` is excluded by a caught semantic mutant.
