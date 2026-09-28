# Result: scheduler-admission

- **Experiment.** `scheduler-admission`, milestone `05-transition-kernel`.
- **Question.** Is serialized, reservation-based greedy selection safe under declared footprints, budgets, and involuntary failures, with reservations held until settlement?
- **Outcome.** Yes, within the generated interleavings and the named scripts: no conflicting overlap, no budget oversubscription, no host work without a reservation, no Action dispatched before its prerequisites succeeded, and no false `Converged`, after every step of 256 generated schedules. Each property fails on a hand-made break of the rule that keeps it.

## Context

| Field | Value |
| --- | --- |
| Selection | `nomos_warp::select::select` over the frontier, the reserved keys and nodes, the capacity, and the budgets (`nomos_warp::budget`) |
| Reservation rule | `nomos_core::action::holds_reservation`: held while the effect is unsettled or the Action is live |
| Admission | One selection per `step`, with the fence re-checked in the same step (ADR 0010 §3) |
| Harness | `crates/bin/nomos-cell/tests/scheduler_admission.rs`; Warp law 4 and `select` unit tests in `nomos-warp` |
| Receipts | `verification/receipts/2026-09-28-transition-kernel.ndjson` |

## Named Scripts

| Test | What it shows |
| --- | --- |
| `a_timeout_keeps_its_reservation` | A and B share a key. A's receipt never comes; at tick 5 A is TimedOut, and B is not admitted until A's settle-by instant at tick 20 settles A's effect. This is the milestone's exit criterion |
| `two_disruptive_restarts_are_admitted_one_at_a_time` | Two restarts Ready together in a rack that may lose one node are never reserved together; the most reservations held at once is one |
| `a_stale_budget_snapshot_admits_nothing_disruptive` | With the budget snapshot past its maximum age, nothing that disrupts is admitted and the run waits rather than failing; a Plan with a fresh snapshot converges |
| `a_superseding_plan_drains_before_it_acts` | A newer Plan cancels the old run's undispatched Action, which is never issued, and waits for the old in-flight effect before observing |
| `select::tests::one_selection_admits_within_the_budget`, `reserved_disruption_counts_against_the_budget`, `unavailable_and_stale_snapshots_hold_back_disruption` | The predicate of ADR 0010 §4 in one selection, across selections, and against the snapshot |

## Generated Interleavings

`generated_interleavings_never_overlap_oversubscribe_or_lie`: 256 cases from the ChaCha seed `4e3501`, each a Canon of three files with generated conflict keys and disruption over a three-node rack with a budget of one, plus the refresh pair, with capacity three; and a schedule of up to 80 operations drawn from: deliver any item in flight, drop one, duplicate one, let 1 to 7 ticks pass, crash (at most twice), and supersede (at most once). After every step it checks:

| Property | Check |
| --- | --- |
| No conflicting overlap | The kernel's reservations are pairwise key-disjoint |
| No oversubscription (N9) | At most one rack node is disrupted by reserved effects |
| A timeout frees nothing | Every piece of host work that can still happen, before its settle-by instant, has a reservation in the kernel |
| N4, lifecycle side | Every Action dispatched in the step has every `requires` source Succeeded or anchored |
| `Converged` is sound | The host satisfies every Condition, the service loaded what is on disk, nothing is owed, every effect is Settled |
| The snapshot is a fold | The snapshot equals the replay of the log, and no key is requested twice (asserted by the simulator on every step) |

Once the schedule ends, every settle-by instant passes, and a fresh Plan must converge. All 256 cases pass.

## Negative Controls

Each break was made by hand in `step.rs` against the committed kernel, the property run, and the kernel restored; each failed with the property's own message, and the minimized case is in the test output:

| Break | Property that failed |
| --- | --- |
| Release a reservation once its Action is not live, settled or not | "live host work without a reservation" |
| Record no Obligation at dispatch | "Converged with the refresh lost" |
| Select with no budgets | "budget oversubscribed" |

`budget-not-world` is the Warp law's and the named tests' subject: a node unavailable in the snapshot counts against the budget, and an involuntary failure past the budget pauses disruption without failing anything. Semantic mutants `SM-TRANSITION-005` (reservation released at timeout) and `SM-TRANSITION-007` (reserved disruption left out of the budget) are caught by their named tests.

## Unchecked

- Starvation and fairness: the greedy pass is claimed safe and deterministic, not fair.
- Derived footprints beyond the refresh's: `package` and `systemd_unit` Actions do not exist yet.
- Admission at Loom across Cells: the budget here is one Cell's view, with a snapshot the policy supplies.
- Larger Canons and longer schedules than generated.

## Decision

Reservations last until settlement, as ADR 0009 §5 says, and budgets bound admission, as ADR 0010 §4 says. The second acceptance criterion of ADR 0009, with [refresh-recovery](refresh-recovery.md), is met by these records.
