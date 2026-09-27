# Formal

Precise statements of the algorithms Nomos relies on and of the invariants it
must preserve, with proof sketches.

These documents are written arguments for humans. The machine-checked models
of the control protocol live in [`/formal/tla`](../../formal/tla/). When a
statement here is modelled in TLA+, the document links to the module.

| Document | Contents |
|---|---|
| [invariants.md](invariants.md) | N1–N12 stated as predicates, with how each one is established |
| [reconciliation.md](reconciliation.md) | The reconciliation loop, fixed point, bounded convergence, oscillation detection |
| [warp.md](warp.md) | Graph model, edge semantics, cycle detection, topological order, execution frontier, conflict keys |
| [fencing-and-idempotency.md](fencing-and-idempotency.md) | Generations, stale-Plan rejection, deduplication under at-least-once delivery |
| [event-log.md](event-log.md) | Ordering, causality, state as a fold, integrity chain |

## Status

Everything here is **draft v0.1**. It is derived from the
[project specification](../PROJECT-SPEC.md). A definition marked *working
definition* still needs an ADR before it binds the implementation.

## Notation

| Symbol | Meaning |
|---|---|
| $D_r$ | desired state of resource $r$ (from Canon) |
| $O_r$ | observed state of resource $r$ |
| $\mathrm{diff}(D, O)$ | Variance; $\varnothing$ when $O$ satisfies $D$ |
| $G = (V, E)$ | Warp graph; $V$ = Actions, $E$ = dependency edges |
| $g$ | Plan generation (fencing token) |
| $g^{acc}_n$ | highest generation accepted by node $n$ |
| $\sigma(a)$ | lifecycle state of Action $a$ |
| $\mathrm{seq}_w(e)$ | writer sequence of Event $e$ from writer $w$ |
| $H$ | cryptographic hash |
