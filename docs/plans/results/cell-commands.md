# Result: cell-commands

- **Milestone.** `14-cell-commands` of the [Phase 1 plan](../phase-1-masterless-cell.md): `nomos-cell traits`, `trace`, `enforce`, `import`, and `events` on a `.cbor` artifact, as [cell-commands.md](../../formal/cell-commands.md) defines them.
- **Question.** Does `trace` print an Indeterminate Assessment and plan nothing for it, and change nothing on a Debian host; and does `enforce` end as [reconciliation.md](../../formal/reconciliation.md) defines, with the exit status the command table names?
- **Outcome.** Yes, on Debian 12 and 13 in containers. `trace` of a Canon with a directory, a configuration file from a bundle, a kernel parameter, a system account, and a service restarted when its configuration changes showed five Variances with five planned Actions and left the host's projection unchanged; `enforce` converged it with five executions; a second `enforce` converged with none; `trace` then found every Condition Satisfied. A denied read was Indeterminate in both commands with no Action; a failed start ended `enforce` Failed, status 4.

## Context

| Field | Value |
| --- | --- |
| Commit | `480efc2`, where every receipt was recorded |
| Hosts | Debian GNU/Linux 12 (bookworm) and 13 (trixie) in privileged containers of `tests/fixtures/debian/Dockerfile`, run by `cargo xtask debian`; the development host, Ubuntu 24.04.4, for the tests that change nothing |
| Oracle | The command table and exit-status table of cell-commands.md; for N1, a projection of the managed resources read with the host's own tools (`stat`, `systemctl show`, `getent`, `/proc/sys`), never through the adapter |
| Harness | `crates/bin/nomos-cell/tests/cell_commands.rs`, running `nomos_cell::cli::run`, which is all the binary's `main` calls |
| Receipts | `verification/receipts/2026-09-30-cell-commands.ndjson` |

## What Was Built

The binary is the Cell's composition root: the Linux Substrate on `/`, the content store and the journal in the state directory, and systemd when the bus answers. `trace` restores the Cell's snapshot from the journal, steps a Tick and the Enforce input, answers the kernel's request for Observations through the observe capability alone, and prints the Assessments and the round the kernel planned; the Apply requests are dropped, and nothing is journaled. `enforce` imports a bundle, recovers a non-empty journal, ticks the kernel on the host's monotonic clock, enforces a Plan whose generation follows the journal's fence, and ticks every second until the run ends. `traits` reports seven Traits with their source and stability, a new `nomos-core` type; `events` prints the Event Log recomputed from the journal.

## On Debian

The fixed-point test printed, on Debian 13:

```text
unit:nomos-cmd.service
  VARIANCE unit-differs: activity, enablement
  action: refresh

0 satisfied, 5 variance, 0 indeterminate; 5 actions planned
```

and then `outcome: converged`, `executions: 5`; `outcome: converged`, `executions: 0`; and `5 satisfied, 0 variance, 0 indeterminate; 0 actions planned`. The unit's Action is a refresh because the configuration file it depends on changes in the same run. `events` listed 54 Events for the two runs, among them `obligation-recorded` and `obligation-discharged` for the unit, and the second run's `recovered` before its `plan-accepted cell generation 2`.

| Test | Result, Debian 12 and 13 |
| --- | --- |
| `trace_changes_nothing_and_enforce_converges_to_a_fixed_point` | As above; the projection before and after `trace` equal |
| `a_denied_read_is_indeterminate_and_plans_nothing` | A file of mode `0000`, read by a thread without the capability to override permissions: `trace` status 2 with `INDETERMINATE collection-failed: permission-denied` and `action: none`; `enforce` status 2, `executions: 0`, the file unchanged |
| `a_failed_start_ends_enforce_failed` | Status 4, `failed: unit:nomos-cmd.service` |
| `trace_plans_nothing_for_an_indeterminate_resource_and_executes_nothing` | Here, on the mock: a Variance with an Action, a denied read with none, and the mock unchanged |

`NonConvergent` is not provoked on a host: its status is checked by the exit-status table's unit test, and the kernel's bound and oscillation detection by `05-transition-kernel`'s tests.

## Semantic Mutants

| Mutant | Wrong behavior | Caught by |
| --- | --- | --- |
| `SM-CELL-001` | `trace` prints converge for a resource with no planned Action | `trace_plans_nothing_for_an_indeterminate_resource_and_executes_nothing` |
| `SM-CELL-002` | Every Plan gets generation 1 | `trace_changes_nothing_and_enforce_converges_to_a_fixed_point`, on Debian 12: the second `enforce` is fenced out |
| `SM-CELL-003` | An Indeterminate outcome exits 0 | `render::tests::each_outcome_exits_with_the_status_the_table_names` |
| `SM-CELL-004` | `enforce` without a Tick | the same fixed-point test, on Debian 12: every unit job is cancelled at a settle-by instant already past |

All 63 active mutants of the corpus were caught.

## What This Does Not Establish

- The binary as a separate process: the tests run `cli::run` in process, which is all `main` does. The installed binary, its service, and its timer are `16-alpha-release`'s.
- `NonConvergent` on a host, and a journal recovered after a real crash of the command; the kill-and-recover property is `10-durable-cell`'s and `11-systemd-unit`'s.
- Traits beyond the seven, and targeting by Traits, which is Loom's.
- Every starting state of every family: that is `15-debian-convergence`.

## Decision Fed

The Cell has its commands. `15-debian-convergence` enumerates the starting states and runs them through `enforce`.
