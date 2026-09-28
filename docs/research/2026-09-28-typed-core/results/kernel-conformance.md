# Result: kernel-conformance

- **Experiment.** `kernel-conformance`, milestone `05-transition-kernel`.
- **Question.** Can the `05-transition-kernel` kernel be modeled in TLA+ with explicit bounds, and do its counterexample traces replay through `step`?
- **Outcome.** Yes for the Action lifecycle and the fence, whose model traces replay through the production transition rules step by step with the kernel refusing exactly what the models refuse; partly for `RefreshRecovery`, whose environment steps replay through the simulator with the model's four properties holding on the kernel at every replayed step, while the model's scheduling freedom stops a replay where the kernel's deterministic order differs.

## Context

| Field | Value |
| --- | --- |
| Tool | The TLA+ model checker (TLC) from the `v1.7.4` release (`tla2tools.jar`, reporting TLC2 Version 2.19), OpenJDK 21.0.10; Kani 0.68.0 |
| Models | `formal/tla/ActionLifecycle.tla`, `Fencing.tla`, `RefreshRecovery.tla`, each with its negative-control configurations |
| Traces | `tests/fixtures/tla/`, TLC's own output, with provenance in its README |
| Replay | `crates/core/nomos-core/tests/conformance.rs`, `crates/bin/nomos-cell/tests/kernel_conformance.rs` |
| Receipts | `verification/receipts/2026-09-28-transition-kernel.ndjson`, check `tlc` for every model and configuration |

## Specification Correctness: TLC

Each result is about the model within its bounds, never about the Rust.

| Model | Bounds | Properties | Result |
| --- | --- | --- | --- |
| `ActionLifecycle` | One Action | `TypeOK`, `SucceededIsVerified` (N6), `UnsettledIsReserved`, `TimedOutStaysUnknown` and `DeadlineEstablishesNothing` (N10) | No error: 30 states generated, 19 distinct, depth 7 |
| `Fencing` | Generations 1 to 3, Plans `a` and `b`, two Actions per Plan | `TypeOK`, `StaleNeverExecutes` (N5), `Monotonic`, `NoStaleAcceptance`, `OnePlanPerGeneration` | No error: 1,099 generated, 571 distinct, depth 11 |
| `RefreshRecovery` | Three generations, two crashes | `TypeOK`, `AdmissionSafety`, `ConvergedIsSound`, `UncertaintyPreserved`, `ObligationDischargedOnlyByRefresh` | No error: 7,122 generated, 2,685 distinct, depth 17 |

| Negative control | Violation TLC reports | Length |
| --- | --- | --- |
| `ActionLifecycle-shortcut`: Running to Succeeded | `SucceededIsVerified` | 5 states |
| `ActionLifecycle-timeout-fails` | `DeadlineEstablishesNothing` | 3 |
| `ActionLifecycle-release-on-timeout` | `UnsettledIsReserved` | 3 |
| `Fencing-split`: check, then effect later (`fence-race`) | `StaleNeverExecutes` | 5 |
| `Fencing-stale-by-one` | `NoStaleAcceptance` | 3 |
| `RefreshRecovery-transient` (`lost-refresh`) | `ConvergedIsSound` | 6 |
| `RefreshRecovery-release-on-timeout` | `AdmissionSafety` | 4 |
| `RefreshRecovery-discharge-on-dispatch` | `ObligationDischargedOnlyByRefresh`; without that property, `ConvergedIsSound` by way of a superseding Plan | 5 |

Two of these found something the text had not said. The transient design loses the refresh without any crash: a superseding Plan after the write discards the in-memory activation. And discharging the Obligation at dispatch loses the refresh when a superseding Plan arrives after the refresh is dispatched and before it runs. ADR 0009's note records the second as the reason discharge waits for verification.

## Implementation Conformance: Trace Replay

| Model | Replayed through | Behaviors | Steps | Result |
| --- | --- | --- | --- | --- |
| `ActionLifecycle` | `advance`, `signals_for`, `Settlement`, `holds_reservation` | 150 simulated, depth up to 20, seed 7 | 374 | Every step agrees on every variable, on the enabled lifecycle actions, and on when a reservation may be released; every one of the 14 actions occurs |
| `Fencing` | `Fence::accept`, `Fence::permits` at the effect | 60 simulated, depth up to 14, seed 7 | 780 | Every step agrees on the accepted generation and Plan, on which of the six Plans would be accepted, and on which may admit an effect |
| `RefreshRecovery` | The simulator and the mock, with the kernel deciding | 80 simulated, depth up to 30, seed 7 | 386 of 906 | The four properties hold on the kernel at every replayed step; 11 behaviors replay whole |

The first run of the lifecycle replay used 60 behaviors and its cover check failed: none of them took `VerifyFails`. The fixture was regenerated with 150, which take every action; the README says so.

`RefreshRecovery` replays stop where the model is free and the kernel is not. The model may dispatch the conflicting Action before the write; the kernel selects in topological order with ties by resource, and always dispatches the write first. Of the 69 behaviors that stop early, 53 stop at `OtherDone`, 13 at `WriteReceipt`, 2 at `HostWrites`, and 1 at `HostRestarts`. Model steps that are kernel decisions (dispatching, converging, failing a run) are not asserted to happen at the model's moment: the kernel takes them when its inputs allow. This is a property check on the kernel's behavior under the model's environment, not a refinement mapping.

Every counterexample diverges from the kernel. The lifecycle and fence counterexamples are refused at their bad step. The `RefreshRecovery` ones replay to kernel states the mutated models reach and the kernel does not: after the `transient` schedule the Obligation is still owed and the run then converges with the refresh done; after `release-on-timeout` the conflicting Action is never issued; after `discharge-on-dispatch` the refresh is dispatched and the Obligation is still held.

## Negative Control: A Bad Kernel

The plan's negative control runs the other way: a known-bad transition inserted into the kernel must be rejected against the model. Each semantic-mutant patch was applied and the replay run:

| Kernel mutant | Lifecycle and fence replay | `RefreshRecovery` replay |
| --- | --- | --- |
| `SM-TRANSITION-001`, Running to Succeeded | Fails | Fails |
| `SM-TRANSITION-002`, deadline to Failed | Fails | Passes |
| `SM-TRANSITION-003`, stale by one accepted | Fails | Passes |
| `SM-TRANSITION-004`, no Obligation recorded | Passes | Fails |
| `SM-TRANSITION-005`, reservation released at timeout | Fails | Fails |
| `SM-TRANSITION-006`, bound before convergence | Passes | Passes |
| `SM-TRANSITION-008`, content-derived key | Passes | Fails |

`SM-TRANSITION-006` is not modeled: no model has a bound. It is caught by its named test in [bounded-convergence](bounded-convergence.md).

The first version of the fence and lifecycle replay did not catch `SM-TRANSITION-003` or `SM-TRANSITION-005`. A model trace contains only steps the model allows, so a kernel that allows more replays it cleanly. The replay now also checks, at every state, that the kernel accepts exactly what the model's guards enable; both mutants then fail. The guards are transcribed from the models into the test by hand, which the test's comments say.

## Bounded Verifier

| Harness | Crate | Result | Negative control |
| --- | --- | --- | --- |
| `succeeded_only_from_a_holding_verification` (N6) | `nomos-core` | Verified, 1 of 1 cover | Fails on `SM-TRANSITION-001` |
| `deadlines_establish_nothing_and_terminal_stages_absorb` (N10) | `nomos-core` | Verified, 2 of 2 covers | Fails on `SM-TRANSITION-002` |
| `the_fence_never_moves_backwards` (N5) | `nomos-core` | Verified, 2 of 2 covers | Fails on `SM-TRANSITION-003` |
| `settlement_needs_evidence_and_is_permanent` | `nomos-core` | Verified, 1 of 1 cover | None run |
| `vertex_resolution_is_ready_only_when_startable_and_activated`, with the owed input | `nomos-warp` | Verified, 4 of 4 covers | None run in this milestone |

The first N6 harness built its `Verified` through `verify` over a validated path, and the C Bounded Model Checker (CBMC) under Kani had not finished after 900 seconds: inconclusive. The harness now takes a `Verified` over an empty, unvalidated path from a constructor that exists only under `cfg(kani)`, and verifies in about three seconds. The lifecycle's state space is finite, and `the_lifecycle_table_is_exhaustive` enumerates it in a unit test as well.

## Mutation Testing

`cargo-mutants` 27.1.0 over the modules this milestone added or changed, twice: once on the code as first committed, and again after the tests the first run found missing. The kernel's mutants run against the tests of `nomos-app` and `nomos-cell`, where the simulator lives.

| Target | First run | Second run |
| --- | --- | --- |
| `nomos-core`: `action.rs`, `plan.rs`, `effect.rs` | 78 mutants: 63 caught, 13 unviable, 2 missed | 77: 64 caught, 13 unviable, 0 missed |
| `nomos-warp`: `budget.rs`, `select.rs`, `frontier.rs`, `graph.rs` | 133: 82 caught, 50 unviable, 1 missed | 133: 83 caught, 50 unviable, 0 missed |
| `nomos-app`: `kernel/step.rs`, `apply.rs`, `mod.rs`, `driver.rs` | 148: 96 caught, 30 unviable, 15 missed, 7 timeouts | 148: 107 caught, 30 unviable, 6 missed, 5 timeouts |

Classification of the first run's survivors:

| Class | Survivors | Disposition |
| --- | --- | --- |
| **Survived: test gaps** | An acceptance receipt settling the effect; an accepted Action with no deadline; the verification deadline replaced by the receipt deadline; recovery not re-observing a Verifying Action; a stale observation deciding an iteration; a late settling receipt also recorded as ignored; the refresh's derived keys; `KernelSnapshot::released`, `EffectKey::iteration`, and `Budget::limit` asserted only with the value the mutant returns | A test each, committed as `Add the tests the transition kernel's mutation run found missing`; the second run catches them all |
| **Excluded** | `Verified::for_harness`, compiled only under `cfg(kani)` | `.cargo/mutants.toml` excludes `for_harness` constructors, in a declared verifier commit |
| **Equivalent** | `>` to `>=` on the monotone clock in `apply` and `tick` (an Event that changes nothing); `&&` to `\|\|` in the completion instant, which is reset on every entry to Verifying; `&&` to `\|\|` in the Verifying freshness pre-check, which `verify` repeats; the deadline kept on a terminal stage, which nothing reads; `Cancelled` in the frontier's outcome map, reached only in a run already ended | Kept; the second run's six survivors are these |
| **Timeout** | Three mutants make `step` itself never return (the anticipation loop, the progress loop, and a round that never finishes); two make the simulator's runs hang (an inverted observation-coverage check and inverted budget freshness) | Inconclusive under the tool's rules. The first three are non-termination, which any test with a deadline notices; the last two now also fail two fast unit tests, committed as `Test that a complete observation decides and that budget age matters` |

## Unchecked

- A refinement mapping from the kernel to `RefreshRecovery`: the kernel's decisions are not shown to be model steps one by one.
- Liveness in any model.
- Anything the models abstract: observation, assessment, planning, and verification are guards in `RefreshRecovery`, and the fence model has no effects beyond a record.
- Bounds beyond those in the tables.

## Decision

The first executable models exist, their negative controls fail as they must, and their traces agree with the kernel's decisions on the lifecycle and the fence. For `RefreshRecovery`, the agreement is on the modeled properties under the modeled environment. A refinement check, or a model that fixes the kernel's selection order, is the next step if the owner wants the stronger claim.
