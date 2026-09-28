# ADR 0009: Warp Prerequisite, Activation, and Reservation Semantics

- **Status.** Accepted, 2026-09-28, by the project owner, on the evidence of milestones `04-warp-kernel` and `05-transition-kernel`
- **Date.** 2026-09-28
- **Candidate.** `warp-gates` in the 2026-09-28 research snapshot (recommendations `warp-semantics`, `durable-refresh`, `scheduler`)

## Context

Spec §62 has long listed edge semantics as needing an ADR. The research review then found that the draft readiness formula let an unchanged configuration trigger a service restart (counterexample `activation-missing`). It found that a satisfied prerequisite had no vertex to hang an edge on (finding `noop-anchor`), and that a crash between a file replacement and its restart lost the restart for good (counterexample `lost-refresh`). The corrected definitions live in [warp.md](../formal/warp.md) as working definitions. This ADR is what makes them binding once their truth tables have run. Research open questions `trigger` and `solver` end here.

## Decision

Proposed. [warp.md](../formal/warp.md) follows it as a working definition until it is accepted.

### 1. Readiness Is Two Questions

A vertex is ready when it is **Startable** and **Activated**. Startable is edge by edge: every `requires` source met, every `after` and `on_change` source terminal. Activated is decided over the `on_change` sources as a group: the vertex has none, or every source is met and at least one succeeded with a verified change. Disabled is a group result, when every source is met and none changed. A single unchanged source never vetoes a changed one; a source that is not met vetoes the group.

A source is **met** when it Succeeded, changed or not, or was Skipped. It is **not met** when it ended any other way: Failed, TimedOut, Cancelled, Rejected, an Indeterminate anchor, or Blocked. `requires` and `on_change` both ask whether their source is met, because both depend on what the source did: `requires` on its result, `on_change` on its change. `after` asks only whether the source is terminal.

### 2. Every Edge Resolves to One State

| Unit | States |
| --- | --- |
| `requires` edge | Waiting, Satisfied, Blocked |
| `after` edge | Waiting, Satisfied. An `after` edge has no Blocked state: it is ordering, not success |
| `on_change` group | Waiting, Activated, Disabled, Blocked |

A group is Blocked as soon as one source is not met, even while others are still running, as a `requires` edge is. A vertex runs, is Skipped, or is Blocked, with no fallthrough. It is Blocked when a `requires` edge or its group is Blocked, and Skipped when its group is Disabled.

**Skipped is met; Blocked is not.** A Skipped vertex had no reason to run and every one of its triggers ended well. Its dependents see it as they see a satisfaction anchor: a met `requires` prerequisite, a terminal `after` source, and an unchanged `on_change` source. A Blocked vertex will never run. Its `requires` and `on_change` dependents are Blocked in turn, and its `after` dependents may proceed, because it is terminal. Blocking therefore propagates along `requires` and `on_change` edges and stops at `after` edges.

### 3. Satisfied Prerequisites Are Anchors

Every resource named by a `requires`, `after`, or `on_change` edge has a vertex. A resource with a Variance or an Obligation gets its Action. A Satisfied resource gets an anchor, Succeeded on entry and unchanged. An Indeterminate resource gets an Indeterminate anchor, terminal on entry, not Succeeded, and unchanged, and its effect is per edge: a `requires` edge from it is Blocked, because a prerequisite that might be missing is not met; an `after` edge from it is Satisfied, because `after` is ordering; and an `on_change` edge from it is a source that is not met, so its group is Blocked: whether the resource changed is unknown, and a refresh that might be owed is neither run nor reported as unnecessary.

### 4. Change Consumption Survives a Crash

Where a service can report the configuration revision it loaded, that revision is a Condition and no trigger is needed. Where it cannot, the refresh is an Obligation, recorded durably before the change that creates it and discharged by the refresh. A transient `on_change` alone is never the only record of a required refresh.

### 5. Reservations Last Until Settlement

An Action holds its conflict keys from dispatch until its effect is Settled, past a timeout and past a satisfied re-observation. Admission is serialized, one path per Cell, and selection is deterministic and greedy over the reserved set. Maximality is claimed only for downward-closed constraints, and fairness is a separate policy. Constraint solving, with a Boolean satisfiability (SAT) or satisfiability modulo theories (SMT) solver, is not used until a precise objective and a benchmark justify it.

### Alternatives

- **All-sources activation.** Rejected: no use case yet, and it makes an unchanged file veto a changed one.
- **Transient triggers only.** Rejected by `lost-refresh`.
- **Release reservations on timeout.** Rejected: a timed-out effect may still be mutating.
- **Count a failed or unknown `on_change` source as unchanged.** Rejected. The group would be Disabled and the vertex Skipped, so a failed configuration write would let every `requires` dependent of the refresh run as if the refresh had been unnecessary, and a timed-out write would be converted into "no change" (N10).
- **Let a changed source activate the group beside a failed one.** Rejected. The refresh would load an input whose write failed or whose outcome is unknown, and a timed-out write may still be running under its reservation (§5). The refresh owed by the changed source is an Obligation (§4), so blocking the vertex delays it without losing it.

### Acceptance Criteria

- **`04-warp-kernel`, `warp-truth-table`.** Exhaustive predecessor-outcome tables over every source outcome, the Indeterminate anchor included, for each edge kind. Also the one-changed-one-unchanged case, the cycle-witness case, and insertion-order invariance, all against production functions and an independent reference evaluator. The Indeterminate-anchor rows in §3 are part of the table the tests must reproduce, not a detail left to the implementation.
- **`05-transition-kernel`, `refresh-recovery` and `scheduler-admission`.** No declared Obligation lost at any crash boundary, and no conflicting overlap or budget oversubscription under adversarial interleavings.

## Note, 2026-09-28: The Warp Kernel's Truth Tables

Milestone `04-warp-kernel` produced the tables this ADR's first acceptance criterion asks for, against production functions and an independent reference evaluator ([record](../research/2026-09-28-typed-core/results/warp-truth-table.md)). It found two points the text left open and recorded them as working definitions: a Skipped vertex is met, and an `after` edge has no Blocked state. The second came from a Kani harness, which showed that a state type shared with `requires` let an `after` edge marked Blocked reach the vertex predicate and report the vertex Ready.

Reviewing those definitions exposed a gap. The text counted every terminal `on_change` source that did not change toward Disabled, a failed or timed-out one included. A failed configuration write therefore Skipped its refresh, and the refresh's `requires` dependents ran as if nothing had been needed. A changed source beside a failed one activated the refresh with the failed input.

The project owner decided both on the same day, and §1 to §3 now carry the decision: a source is met when it Succeeded or was Skipped; an `on_change` group is Blocked when any source is not met; Skipped therefore means every trigger ended well and is met; an `after` edge has no Blocked state; and blocking propagates along `requires` and `on_change` edges, not along `after` edges. The Indeterminate-anchor row for `on_change` in §3 changed with it, from "counts toward Disabled" to Blocked. The kernel, its tables, its reference evaluator, and its harnesses were revised to match, and the record carries the before-and-after runs.

The second acceptance criterion, `refresh-recovery` and `scheduler-admission`, still waits for `05-transition-kernel`; this ADR stays Proposed.

## Note, 2026-09-28: Obligations as Warp Vertices

Milestone `05-transition-kernel` implements §4 and §5 as these working definitions.

- **What an Obligation is.** A record that names the resource to refresh and the execution, by idempotency key, whose change may require it.
- **When it is recorded.** In the Decision that dispatches an Action with an `on_change` edge, one Obligation per target, as an Event ahead of the Action's effect request. Events are appended before effects are issued ([ADR 0012](0012-event-history.md) §1), so the log holds the Obligation before the change can happen.
- **When it is withdrawn.** When that execution is verified with no change, or the Substrate refuses it. A failed, timed-out, or cancelled-after-dispatch execution keeps its Obligation: the file may have changed, and the cost of keeping it is at most one unnecessary refresh.
- **When it is discharged.** When an Action on the target resource is verified Succeeded, for the Obligations pending when that Action was dispatched. A later Obligation needs a later refresh.
- **The vertex.** A resource with a pending Obligation gets an Action vertex marked *owed*. An owed vertex resolves like any other vertex, except that a Disabled `on_change` group makes it Ready instead of Skipped. Blocked and Waiting still apply, so an owed refresh waits for its triggers and never runs with an input whose write failed. A resource that is the `on_change` target of an Action in the same Plan also gets an Action vertex, not owed: its source records the Obligation on dispatch, and if the source changes nothing the Obligation is withdrawn and the vertex is Skipped.
- **Its footprint.** A refresh reads the files its `on_change` sources write, so its conflict keys include theirs. It cannot run beside an unsettled write of its own input, which a crash can leave behind.
- **A Blocked refresh re-runs once its trigger is repaired.** The Obligation outlives the run in which its trigger failed. The next run plans the owed vertex; once the trigger is met, whether or not the repair changed anything, the refresh is Ready.

The TLA+ model `RefreshRecovery` checks the four properties the grounding plan names over one write, its refresh, a conflicting Action, crashes, and superseding authority, with three negative controls: no Obligation, a reservation released at timeout, and an Obligation discharged at dispatch. The last one loses the refresh when a superseding Plan arrives after the refresh was dispatched and before it ran, which is why discharge waits for verification.

**Evidence.** The second acceptance criterion ran: [refresh-recovery](../research/2026-09-28-typed-core/results/refresh-recovery.md) loses no declared Obligation at any of 22 crash points, while the transient design loses six refreshes and the harness sees each; [scheduler-admission](../research/2026-09-28-typed-core/results/scheduler-admission.md) finds no conflicting overlap and no budget oversubscription across 256 generated interleavings with crashes, loss, duplication, and supersession. Both criteria are now met by records. The ADR stays Proposed until the project owner accepts it.

## Note, 2026-09-28: Acceptance

The project owner accepted this ADR on 2026-09-28. Both acceptance criteria are met by records: the truth tables of [warp-truth-table](../research/2026-09-28-typed-core/results/warp-truth-table.md), and [refresh-recovery](../research/2026-09-28-typed-core/results/refresh-recovery.md) with [scheduler-admission](../research/2026-09-28-typed-core/results/scheduler-admission.md). The decision accepted is §1 to §5 as amended by the notes above: a source is met when it Succeeded or was Skipped; an `on_change` group whose source is not met is Blocked; an `after` edge has no Blocked state; a refresh that no Condition can observe is an Obligation, recorded before the change, discharged only by a verified refresh, and planned as an owed vertex until then; a refresh holds the keys of the files it reads; and reservations last until settlement. The working definitions in the notes are no longer working definitions.

## Consequences

- Spec §62's edge-semantics question is closed by this ADR's acceptance.
- **Revisit trigger.** Reopen if a real Canon needs all-sources activation, or if derived footprints for `package` and `systemd_unit` Actions cannot be computed in core.
