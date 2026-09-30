# Formal

Precise statements of the algorithms Nomos relies on and the invariants it must never break, with proof sketches.

These are written arguments, for humans. The machine-checked models of the control protocol live in [formal/tla/](../../formal/tla/). When a statement here is modeled in TLA+, the document links to the module.

A result about a document here answers one of three questions, kept apart in the [verification matrix](verification-matrix.md): whether the specification says what we mean (specification correctness), whether the Rust does what it says (implementation conformance), and whether the host behaves as an Observation or an effect assumes (environment and adapter correspondence). A proof sketch or a TLA+ model speaks to the first; tests and bounded verifiers to the second; adapter suites on Linux to the third. None of them speaks to another.

| Document | Contents |
| --- | --- |
| [invariants.md](invariants.md) | N1–N12 as predicates, and how each one is established |
| [reconciliation.md](reconciliation.md) | The reconciliation loop, fixed point, bounded convergence, and oscillation detection |
| [warp.md](warp.md) | Graph model, edge semantics, cycle detection, topological order, execution frontier, and conflict keys |
| [fencing-and-idempotency.md](fencing-and-idempotency.md) | Generations, stale-Plan rejection, and deduplication under at-least-once delivery |
| [event-log.md](event-log.md) | Ordering, causality, state as a fold, and the integrity chain |
| [canon-ir.md](canon-ir.md) | The Canonical IR: data model, schema versions, semantic equivalence, the two candidate encodings, strict decoding, migration, and `CanonID` |
| [resource-families.md](resource-families.md) | The seven resource families: keys, requirements, evidence, truth tables, operations, and the admission check against two resources writing one property |
| [substrate-contract.md](substrate-contract.md) | The Substrate port's clauses every adapter satisfies, the conformance suite that checks them, and and how the Linux adapter serves each family |
| [cell-commands.md](cell-commands.md) | The `nomos-cell` binary: `traits`, `trace`, `enforce`, `import`, and `events`, its state directory, the Plan it enforces, what it prints, and its exit status |
| [core-purity.md](core-purity.md) | The purity contract of `crates/core/`: what a core function may depend on, what enforces each clause, and what nothing enforces |
| [verification-strategy.md](verification-strategy.md) | The twelve-layer ladder, what each layer establishes and does not, the policies on counterexamples, metamorphic, differential, and mutation testing, and what is not a pass |
| [verification-matrix.md](verification-matrix.md) | Which property is checked by which layer, with what result and which record; filled from receipts and result records only |

## Status

Everything here is **draft v0.1**, derived from the [project specification](../PROJECT-SPEC.md). Anything marked *working definition* needs an ADR before it binds the implementation.

The 2026-09-28 research snapshot reviewed these drafts and reproduced seven counterexamples against them, and a review of `main` at `013b9d0` found four further inconsistencies (multi-source activation, convergence with Obligations, settlement against satisfaction, and the cycle witness on blocked singletons). All are corrected here. What remains open is in each document's Known Gaps section and is assigned in the [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md). Nothing in the algorithm documents has been checked against Rust code, because the kernels do not exist; the matrix says so row by row.

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
