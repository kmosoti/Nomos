# TLA+ Models

Machine-checked models of the control protocol. They model the protocol, not Linux, and not the Rust.

Modules, written for milestone `05-transition-kernel` ([grounding plan](../../docs/research/2026-09-28-typed-core/grounding-plan.md#05-transition-kernel)). Each has a configuration for the model and one per negative control; the constant `Mutation` selects the known-bad transition.

| Module | Checks | Negative controls |
| --- | --- | --- |
| `ActionLifecycle.tla` | N6, N10, and that an unsettled effect or a live Action holds its reservation | `shortcut` (Running to Succeeded), `timeout-fails`, `release-on-timeout` |
| `Fencing.tla` | N5 at the effect, generation monotonicity, no stale acceptance, one Plan per generation | `split` (the `fence-race` check-then-act gap), `stale-by-one` |
| `RefreshRecovery.tla` | Admission safety, uncertainty preservation, recovery safety, and completion soundness over a write, its refresh, an Obligation, crashes, and superseding authority | `transient` (the `lost-refresh` design), `release-on-timeout`, `discharge-on-dispatch` |

`PlanExecution.tla`, the frontier over a small Plan for N4, was planned and is not written: the frontier is checked exhaustively by truth tables, laws against a reference, and Kani harnesses in `04-warp-kernel`, and the lifecycle side of N4 is in `ActionLifecycle` and the kernel's tests. A model of the frontier would repeat those at a smaller bound.

Run a model with the TLA+ model checker (TLC) 1.7.4, from `tla2tools.jar` whose `sha256sum` is `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`, from this directory:

```sh
java -cp tla2tools.jar tlc2.TLC -config RefreshRecovery.cfg RefreshRecovery.tla
```

Results, with every run's bounds and state counts, the traces replayed through the kernel, and what the replay does not establish: [kernel-conformance](../../docs/research/2026-09-28-typed-core/results/kernel-conformance.md). The traces are fixtures under [`tests/fixtures/tla/`](../../tests/fixtures/tla/README.md).

## What a Model Result Is

A run of the TLA+ model checker (TLC) establishes that, within the stated bounds on Actions, generations, and interleavings, no reachable state of the model violates the checked property. It is a result about **specification correctness**: the transitions as written are consistent with the invariant. It says nothing about the Rust until the model's counterexample traces are replayed through `step` and the kernel's decisions agree on every modeled transition, which is the `kernel-conformance` experiment. It says nothing about Linux at all. The [verification matrix](../../docs/formal/verification-matrix.md) records a model result in its own column, never as an implementation or environment result, and never as a proof.

Every model ships with a negative control: a known-bad transition inserted into the model must produce a trace the checker rejects. A model whose properties hold on the first run and whose negative control also passes is not checking anything ([ADR 0007](../../docs/adr/0007-verification-gates.md) §2).

See [spec §48](../../docs/PROJECT-SPEC.md#48-formal-specification). Written statements and proof sketches of the same properties live in [docs/formal/](../../docs/formal/).
