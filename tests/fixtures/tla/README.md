# TLA+ Traces

Behaviors of the models in [`formal/tla/`](../../../formal/tla/), produced by the TLA+ model checker (TLC) and replayed through the transition kernel by the `kernel-conformance` tests: `crates/core/nomos-core/tests/conformance.rs` for `ActionLifecycle` and `Fencing`, and `crates/bin/nomos-cell/tests/kernel_conformance.rs` for `RefreshRecovery`. Every file here is TLC's own output. None is edited, except that the line in which the Java runtime echoes its proxy settings and the process identifier were removed from the counterexample outputs, because they describe the machine and not the model.

## Provenance

- **Tool.** `tla2tools.jar` from the TLA+ `v1.7.4` release, whose `sha256sum` is `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`; it reports itself as TLC2 Version 2.19 of 08 August 2024. Java: OpenJDK 21.0.10.
- **Models.** The modules and configurations in `formal/tla/` at the commit that adds these files.
- **Date.** 2026-09-28.

## Simulated Behaviors

Random behaviors of each model with every property of its configuration checked, from a fixed seed, run from `formal/tla/`:

```sh
java -cp tla2tools.jar tlc2.TLC -simulate file=<dir>/trace,num=150 -depth 20 -seed 7 -config ActionLifecycle.cfg ActionLifecycle.tla
java -cp tla2tools.jar tlc2.TLC -simulate file=<dir>/trace,num=60 -depth 14 -seed 7 -config Fencing.cfg Fencing.tla
java -cp tla2tools.jar tlc2.TLC -simulate file=<dir>/trace,num=80 -depth 30 -seed 7 -config RefreshRecovery.cfg RefreshRecovery.tla
```

| Directory | Behaviors | Depth |
| --- | --- | --- |
| `ActionLifecycle/` | 150 | up to 20 |
| `Fencing/` | 60 | up to 14 |
| `RefreshRecovery/` | 80 | up to 30 |

Sixty lifecycle behaviors did not take `VerifyFails` once, which the replay's cover check reported; 150 take every action.

## Counterexamples

The standard output of TLC on each negative-control configuration: `java -cp tla2tools.jar tlc2.TLC -config <Model>-<mutation>.cfg <Model>.tla`. Each ends in the violation its configuration exists to produce.

| File | Violated property | Last step |
| --- | --- | --- |
| `ActionLifecycle-shortcut.out` | `SucceededIsVerified` | `Complete` |
| `ActionLifecycle-timeout-fails.out` | `DeadlineEstablishesNothing` | `Deadline` |
| `ActionLifecycle-release-on-timeout.out` | `UnsettledIsReserved` | `Deadline` |
| `Fencing-split.out` | `StaleNeverExecutes` | `Effect` |
| `Fencing-stale-by-one.out` | `NoStaleAcceptance` | `Accept` |
| `RefreshRecovery-transient.out` | `ConvergedIsSound` | `Converge` |
| `RefreshRecovery-release-on-timeout.out` | `AdmissionSafety` | `DispatchOther` |
| `RefreshRecovery-discharge-on-dispatch.out` | `ObligationDischargedOnlyByRefresh` | `DispatchRefresh` |

A model or configuration change makes these stale. Regenerate them with the commands above and commit the output with the change that caused it.
