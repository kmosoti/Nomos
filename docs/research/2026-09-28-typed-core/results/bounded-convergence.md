# Result: bounded-convergence

- **Experiment.** `bounded-convergence`, milestone `05-transition-kernel`.
- **Question.** Does the loop report the right outcome at the bound without hiding effects still in flight, and does it distinguish Indeterminate from Converged and Failed?
- **Outcome.** Yes, for the file family and the service refresh on the mock. Every script ends in the outcome [reconciliation.md](../../../formal/reconciliation.md) names for it; the last permitted execution converges; a slow completion is not oscillation; and no run reports Converged with an effect unsettled or an Obligation pending.

## Context

| Field | Value |
| --- | --- |
| Kernel | `nomos_app::kernel::step`: the loop observes only once every effect is Settled, checks convergence before the bound, and ends `Indeterminate` when everything left is Blocked behind an unknown (reconciliation.md, transition-kernel note) |
| Harness | `crates/bin/nomos-cell/tests/bounded_convergence.rs` on the simulator and the mock |
| Receipts | `verification/receipts/2026-09-28-transition-kernel.ndjson` |

## Scripts and Outcomes

| Script | Test | Outcome | Executions |
| --- | --- | --- | --- |
| One drifted file, bound 1, 2, and 3 | `the_final_permitted_execution_converges` | `Converged` for each bound | 1 |
| The same, with a writer that undoes the first repair, bound 1 | same test | `NonConvergent(Bound)` | 1 |
| Bound 0 | `a_bound_of_zero_observes_once_and_executes_nothing` | `NonConvergent(Bound)` | 0 |
| A writer that restores the old content after each repair | `a_writer_that_restores_the_old_content_is_oscillation` | `NonConvergent(Oscillation)` | 1 |
| A writer whose content never repeats, bound 3 | `a_writer_that_never_repeats_reaches_the_bound` | `NonConvergent(Bound)` | 3 |
| The restoring writer, with telemetry outside the Canon changing on every observation | `unrelated_telemetry_does_not_hide_oscillation` | `NonConvergent(Oscillation)` | 1 |
| The write completes four ticks late, inside its deadline | `a_slow_completion_is_not_oscillation` | `Converged` | 1 |
| A denied read and nothing else to do | `a_denied_read_with_nothing_else_to_do_is_indeterminate` | `Indeterminate` with `CollectionFailed(PermissionDenied)` | 0 |
| A Variance whose `requires` prerequisite cannot be read | `a_variance_blocked_behind_an_unknown_is_indeterminate` | `Indeterminate` with the prerequisite's reason | 0 |
| The refresh scenario | `converged_holds_nothing` | `Converged` with no reservation and no Obligation | 2 |

A known failure and an unknown outcome are the two lists of `Failed`; their scripts are in [effect-recovery](effect-recovery.md) and [refresh-recovery](refresh-recovery.md).

## Negative Controls

- **`final-check`.** Semantic mutant `SM-TRANSITION-006` checks the bound before convergence, as the draft did; `the_final_permitted_execution_converges` catches it, because the second observation of a bound-1 run is the last one permitted and finds the host converged.
- **A slow completion must not read as oscillation.** The loop never observes while an effect is unsettled, so a late completion cannot look like a reverted state; the script shows it converging.
- **A denied read must end `Indeterminate`**, not `Converged` and not `Failed`; it does.
- **Soundness of `Converged`** is also checked after every step of the generated interleavings in [scheduler-admission](scheduler-admission.md): whenever the kernel says Converged, the host satisfies every Condition, the service loaded what is on disk, nothing is owed, and every effect is Settled.

## Unchecked

- Oscillation that repeats no exact state is caught only by the bound, as the formal document says.
- Two controllers are not modeled; the scripted writer is not a controller with its own loop (`controller-composition`).
- Conditional liveness, that a fair, unobstructed host converges within some bound, is not claimed.

## Decision

The loop of reconciliation.md holds as implemented, with the three working definitions its transition-kernel note records. `Converged`, `NonConvergent` by bound and by oscillation, `Indeterminate`, a known failure, and an unknown outcome are distinguished.
