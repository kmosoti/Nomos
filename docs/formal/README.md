# Formal

Precise statements of the algorithms Nomos relies on and the invariants it must never break, with proof sketches.

These are written arguments, for humans. The machine-checked models of the control protocol live in [formal/tla/](../../formal/tla/). When a statement here is modeled in TLA+, the document links to the module.

| Document | Contents |
| --- | --- |
| [invariants.md](invariants.md) | N1–N12 as predicates, and how each one is established |
| [reconciliation.md](reconciliation.md) | The reconciliation loop, fixed point, bounded convergence, and oscillation detection |
| [warp.md](warp.md) | Graph model, edge semantics, cycle detection, topological order, execution frontier, and conflict keys |
| [fencing-and-idempotency.md](fencing-and-idempotency.md) | Generations, stale-Plan rejection, and deduplication under at-least-once delivery |
| [event-log.md](event-log.md) | Ordering, causality, state as a fold, and the integrity chain |

## Status

Everything here is **draft v0.1**, derived from the [project specification](../PROJECT-SPEC.md). Anything marked *working definition* needs an ADR before it binds the implementation.

The 2026-09-28 research snapshot reviewed these drafts and reproduced seven counterexamples against them, and a review of `main` at `013b9d0` found four further inconsistencies (multi-source activation, convergence with Obligations, settlement against satisfaction, and the cycle witness on blocked singletons). All are corrected here. What remains open is in each document's Known Gaps section and is assigned in the [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md).

## Notation

| Symbol | Meaning |
| --- | --- |
| $C_r$ | Condition on resource $r$ (from Canon) |
| $O_r$ | Observation of resource $r$ |
| $\mathrm{assess}(C, O)$ | Assessment: $\mathrm{Satisfied}$, $\mathrm{Variance}(\delta)$, or $\mathrm{Indeterminate}(\rho)$ |
| $S$ | The host itself, as Substrate exposes it; $\pi(S)$ its managed-resource projection |
| $G = (V, E)$ | Warp graph; $V$ = Actions, $E$ = dependency edges |
| $g$ | Plan generation (fencing token) |
| $g^{acc}_n$ | Highest generation accepted by node $n$ |
| $\sigma(a)$ | Lifecycle state of Action $a$ |
| $\mathrm{seq}_w(e)$ | Writer sequence of Event $e$ from writer $w$ |
| $H$ | Cryptographic hash |
