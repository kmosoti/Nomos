# TLA+ Models

Machine-checked models of the control protocol. They model the protocol, not Linux, and not the Rust.

Planned modules, each arriving with milestone `05-transition-kernel` ([grounding plan](../../docs/research/2026-09-28-typed-core/grounding-plan.md#05-transition-kernel)):

- `ActionLifecycle.tla`: the lifecycle of one Action, for N6.
- `PlanExecution.tla`: the execution frontier over a small Plan, for N4.
- `Fencing.tla`: competing authority and generation acceptance, for N5 and N9.

## What a Model Result Is

A run of the TLA+ model checker (TLC) establishes that, within the stated bounds on Actions, generations, and interleavings, no reachable state of the model violates the checked property. It is a result about **specification correctness**: the transitions as written are consistent with the invariant. It says nothing about the Rust until the model's counterexample traces are replayed through `step` and the kernel's decisions agree on every modeled transition, which is the `kernel-conformance` experiment. It says nothing about Linux at all. The [verification matrix](../../docs/formal/verification-matrix.md) records a model result in its own column, never as an implementation or environment result, and never as a proof.

Every model ships with a negative control: a known-bad transition inserted into the model must produce a trace the checker rejects. A model whose properties hold on the first run and whose negative control also passes is not checking anything ([ADR 0007](../../docs/adr/0007-verification-gates.md) §2).

See [spec §48](../../docs/PROJECT-SPEC.md#48-formal-specification). Written statements and proof sketches of the same properties live in [docs/formal/](../../docs/formal/).
