# Result: refresh-recovery

- **Experiment.** `refresh-recovery`, milestone `05-transition-kernel`.
- **Question.** Which design keeps a service refresh across a crash between file replacement and restart: a durable Obligation recorded before the replacement, a loaded-revision Condition, or the draft's transient `on_change`?
- **Outcome.** The durable Obligation and the loaded-revision Condition both keep the refresh at every crash point of the script; the transient design loses it at six of them, and the harness sees each loss. The durable Obligation costs five unnecessary refreshes across the sweep and claims no exactly-once refresh.

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1 |
| Kernel | `nomos_app::kernel::step`, with `nomos-core`'s lifecycle and settlement and `nomos-warp`'s owed vertices ([ADR 0009](../../../adr/0009-warp-activation-semantics.md) note) |
| Host | `nomos-substrate-mock`: a configuration file on disk and a service that loaded its own revision when it last started, kept apart |
| Harness | `crates/bin/nomos-cell/tests/refresh_recovery.rs` on the simulator in `tests/support/mod.rs` |
| Receipts | `verification/receipts/2026-09-28-transition-kernel.ndjson` |

## Script

The file holds revision 1, the service runs revision 1, and the Canon asks for revision 2. A Plan with bound 3 is presented. Without a crash, each design converges in 11 Decisions with one refresh.

The sweep crashes the Cell at each of the 11 Decisions in two ways: after the Decision's Events are appended and before its effects are issued, and after its effects are issued with everything that comes back lost. That is 22 crash points per design. A crash drops every Observation and receipt in flight, keeps any work already handed to the host, rebuilds the snapshot by replaying the log, and steps `Recovered`. If the recovered run does not converge, the harness lets every settle-by instant pass and presents a second Plan.

| Design | Canon | What survives a crash |
| --- | --- | --- |
| 1, durable Obligation | The file, and the service with an `on_change` edge from it | The Obligation, recorded as an Event before the write's effect request |
| 2, loaded-revision Condition | The file, and the service's loaded revision as a Condition, with a `requires` edge | Nothing needs to: the revision is observed |
| 3, transient `on_change` | Design 1's, with an Event Log that stores no Obligation Event | Nothing: the Obligation lives in memory and dies with the process |

## Measurements

| Design | Crash points | Runs | Lost refreshes | Unnecessary refreshes | Unknown outcomes | Not converged |
| --- | --- | --- | --- | --- | --- | --- |
| 1, durable Obligation | 22 | 34 | 0 | 5 | 12 | 0 |
| 2, loaded-revision Condition | 22 | 34 | 0 | 0 | 12 | 0 |
| 3, transient `on_change` | 22 | 34 | 6 | 0 | 12 | 0 |

A lost refresh is a final `Converged` with the service's loaded revision different from the file. An unnecessary refresh reloads the revision the service already had. An unknown outcome is a first run that ended `Failed` with a TimedOut Action: 12 of 22 crash points land while an effect is in flight, and the kernel reports each as unknown rather than guessing (N10).

The five unnecessary refreshes in design 1 come from crashes after the refresh ran and before its verification was recorded: the Obligation is still pending, so the next run refreshes again. That is the cost ADR 0009's note accepts in exchange for never losing one, and it is why the record claims no exactly-once refresh. Design 2 has none, because it observes what design 1 has to remember.

## The Exit Criterion

`a_crash_after_replacement_cannot_lose_the_refresh`: the write is performed, the Cell crashes before its receipt arrives, the first run ends `Failed` with the write's outcome unknown, the Obligation survives the replay, and the second run refreshes the service and converges with the new revision loaded. Semantic mutant `SM-TRANSITION-004`, which dispatches without recording the Obligation, is caught by it.

## Negative Controls

- **`lost-refresh`.** Design 3 loses the refresh at 6 of 22 crash points, and `the_transient_design_loses_the_refresh_and_the_harness_sees_it` asserts that it does. A harness that could not see the loss would pass design 3.
- **The model's version.** `RefreshRecovery.tla`'s `transient` configuration loses the refresh without a crash: a superseding Plan after the write wipes the in-memory activation. Replaying that trace's environment through the kernel does not lose it ([kernel-conformance](kernel-conformance.md)).

## Unchecked

- One service, one file, one crash per scenario; two crashes in one scenario are covered only by the generated interleavings of [scheduler-admission](scheduler-admission.md).
- The mock performs work at delivery and honors settle-by exactly. A real service manager's job semantics, and a service that exposes its loaded revision on a real host, are `07-substrate-conformance`.
- Physical durability: the Event Log is in memory, and "appended" means appended to a vector. Crash and fsync testing waits for a storage adapter (`event-crash-replay`).

## Decision

Design 1 is the default for services that cannot report the configuration they loaded, and design 2 is preferred wherever a service can. Design 3 is rejected, as ADR 0009 already says. The retry policy for the uncertainty design 1 leaves is: keep the Obligation until a refresh is verified, and accept the unnecessary refreshes that crashes in the refresh's window cause, of which the sweep counted five.
