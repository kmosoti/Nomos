# Result: systemd-refresh, and units on Linux

- **Experiment.** `systemd-refresh`, owned by milestone `11-systemd-unit` of the [Phase 1 plan](../phase-1-masterless-cell.md), with the part of `linux-families` the milestone owns: units.
- **Question.** Does a refresh survive a crash between the change and the job, on a real service manager? And does the unit family on Linux pass the same suite as the mock?
- **Outcome.** Yes to both, on Debian 12 (systemd 252) and Debian 13 (systemd 257) in containers with systemd as PID 1. A Cell killed after each of the 12 inputs of the refresh scenario, 8 of them with a refresh owed, recovered its snapshot from its journal and ended with the service restarted on the new configuration. A failed start job left the Action Failed, and a unit systemd cannot load was Indeterminate with the reason `unavailable`.

## Context

| Field | Value |
| --- | --- |
| Commit | `24a8dcf`, where every receipt was recorded |
| Hosts | Debian GNU/Linux 12 (bookworm), systemd 252, `running`; Debian GNU/Linux 13 (trixie), systemd 257, `degraded`; both on kernel 6.18.44 in a privileged container of `tests/fixtures/debian/Dockerfile`, run by `cargo xtask debian` |
| Oracle | The unit table of [resource-families.md](../../formal/resource-families.md), judged by core on a new Observation; the ground truth read with `systemctl show`, mapped from systemd's documentation in the test, not by the adapter; for the refresh, the configuration the service itself copied when it started |
| Harness | `crates/bin/nomos-cell/tests/systemd_units.rs`; `crates/adapters/nomos-substrate-linux/src/units.rs` |
| Receipts | `verification/receipts/2026-09-30-systemd-unit.ndjson` |

## What Was Built

The Linux adapter serves units through `org.freedesktop.systemd1` over `zbus`'s blocking client, as [substrate-contract.md](../../formal/substrate-contract.md), Units on Linux, states it. A unit is examined with `GetUnit`, or `GetUnitFileState` once systemd has unloaded it. A convergence or refresh sets the unit-file state, then starts, stops, or restarts the unit, and polls the job's own object until the job is gone; the unit is read again, and the receipt is `Failed` when it did not end where the job was to take it. A job still pending at the settle-by instant is cancelled, and no receipt settles the execution.

Two findings shaped it. systemd unloads a disabled unit the moment its stop job ends, so the unit's `Job` property is no witness of completion: the first run failed there, and the adapter now polls the job. And the kernel's clock starts at zero until a `Tick`: a driver that enforces without one gives the adapter a settle-by instant in the past, and every job is cancelled. The tests tick before they enforce; the Cell's command does the same from `14-cell-commands`.

## The Suite for Units

`the_linux_adapter_passes_the_suite_for_units` ran S1 to S9 for the unit family on both releases. The starting points are the stable ones: `active`, `inactive`, and `failed`, each `enabled`, `disabled`, and `static`, arranged with `systemctl` from unit files the test writes. A denied read was arranged with a mandatory D-Bus policy that denies every message to the unit's object path, which binds root too, and was observed as permission denied. S7 refused a requirement of another family, a unit systemd does not know, and a static unit asked to be enabled.

A unit that is `activating`, `deactivating`, or `reloading` leaves that state on its own, so the suite cannot hold it still between two readings of the ground truth. `a_unit_in_transition_is_observed_and_converged` holds a unit in each for two seconds with slow start, stop, and reload commands: each was observed as such, and a convergence waited for the job and ended with the unit where the requirement asked.

## The Milestone's Exit

| Test | Oracle | Result, Debian 12 and 13 |
| --- | --- | --- |
| `a_refresh_survives_a_crash_between_the_change_and_the_restart` | The service's own copy of the configuration it started with | For every one of 12 kill points, the reopened Cell held the snapshot it had, then converged with the service on the new configuration |
| `a_job_that_fails_leaves_the_action_failed` | A unit whose start command fails | The adapter's receipt is `Failed`; the run ends `Failed` with the unit named, never `Converged` |
| `a_unit_systemd_cannot_load_is_indeterminate` | A unit with no unit file | The run ends `Indeterminate` with `CollectionFailed(Unavailable)`, and nothing is executed (N13) |
| `a_job_pending_at_the_deadline_is_cancelled_and_unsettled` | A start that takes 30 seconds, due in 1 | Receipts `Accepted`, `Started`, and nothing that settles (N10) |

## Semantic Mutants

The unit behavior exists only on a real host, so its mutants name `host = "debian"`: the runner builds the patched workspace and runs the one named test in a fresh Debian 12 container.

| Mutant | Wrong behavior | Caught by |
| --- | --- | --- |
| `SM-SUBSTRATE-009` | A unit with no unit file reported as inactive and disabled | `a_unit_systemd_cannot_load_is_indeterminate` |
| `SM-SUBSTRATE-010` | A job not waited for | `a_unit_in_transition_is_observed_and_converged` |
| `SM-SUBSTRATE-011` | A start completed without checking that the unit runs | `a_job_that_fails_leaves_the_action_failed`, strengthened to read the adapter's receipt, since the kernel's verification alone also fails the Action |
| `SM-SUBSTRATE-012` | A cancelled job settled | `a_job_pending_at_the_deadline_is_cancelled_and_unsettled` |
| `SM-SUBSTRATE-013` | A static unit enabled rather than refused | `the_linux_adapter_passes_the_suite_for_units` |

All 51 active mutants of the corpus were caught; `SM-SUBSTRATE-002` and `003` were regenerated against the adapter's new lines. The `mutants-semantic` receipt's test counts, 5 run and 5 failed, are those the Debian harness printed for the five mutants run there, each failing as it must; the runner's verdict is its exit status.

## What This Does Not Establish

- A Debian host that is not a container, or a unit whose job takes longer than the 90 seconds the adapter waits at most.
- Units of other types than services in the suite: sockets, timers, targets, paths, and mounts are named by the family and served by the same calls, and not tested.
- `masked` and `other` unit-file states on Linux; the mock covers them.
- A crash in the middle of a D-Bus call, or of systemd itself: the kill is in-process, and the journaled Cell and the adapter's cancellation are what is tested.
- Kernel parameters, users, and packages on Linux: `12` and `13`.

## Decision Fed

Units are served on Debian, and the refresh Obligation of `05-transition-kernel` holds against a real service manager. The production command must tick the kernel's clock before it enforces.
