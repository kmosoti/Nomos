# Invariants

The constitutional layer (spec §58). Each invariant appears as a predicate, next to the mechanism that establishes it and the tests that check it.

| # | Statement | Predicate | Established by | Tested by |
| --- | --- | --- | --- | --- |
| N1 | Trace does not mutate Substrate | $S_{before}(\mathrm{Trace}) = S_{after}(\mathrm{Trace})$ | Trace holds only the observe capability of the Substrate port | Property (mock), integration |
| N2 | Successful Enforce converges toward Canon | $\mathrm{Enforce}(C,S)=S' \wedge \mathrm{ok} \Rightarrow \mathrm{diff}(C,S')=\varnothing$ | Verify after apply; re-observe loop ([reconciliation](reconciliation.md)) | Property, integration |
| N3 | Re-enforcing converged Canon mutates nothing | $\mathrm{diff}(C,S)=\varnothing \Rightarrow \mathrm{mut}(\mathrm{Enforce}(C,S))=\varnothing$ | Plan is a function of Variance only | Property (fixed point) |
| N4 | No Action runs before its hard dependencies succeed | $\mathrm{Running}(a) \Rightarrow \forall d \in \mathrm{Req}(a): \sigma(d)=\mathrm{Succeeded}$ | Execution frontier ([warp](warp.md)) | Graph, TLA+ `PlanExecution` |
| N5 | A stale fenced Plan cannot execute | $\mathrm{Execute}(a) \Rightarrow g(a) \ge g^{acc}_n$ | Generation check before execution ([fencing](fencing-and-idempotency.md)) | TLA+ `Fencing`, failure |
| N6 | Succeeded requires verified postconditions | $\sigma(a)=\mathrm{Succeeded} \Rightarrow \mathrm{Verified}(a)$ | Only `Verifying → Succeeded` exists | TLA+ `ActionLifecycle` |
| N7 | Events are never mutated after append | $e \in L_t \Rightarrow \forall t' > t: L_{t'}[\mathrm{pos}(e)] = e$ | Store port exposes append only | Property, crash |
| N8 | Cipher plaintext never persists | $\forall e \in L: \mathrm{plaintext}(c) \not\sqsubseteq \mathrm{ser}(e)$ | Cipher values are opaque secret types that cannot be serialized | Property (secret non-disclosure) |
| N9 | Failure budgets are never intentionally exceeded | $\forall f: \mathrm{Unavailable}(f) \le k_f$ | Admission control in Loom scheduling ([warp](warp.md#runnable-set)) | TLA+, failure |
| N10 | Unknown outcomes stay unknown | $\mathrm{TimedOut}(a) \Rightarrow \neg\mathrm{assume}(\mathrm{Succeeded}) \wedge \neg\mathrm{assume}(\mathrm{Failed})$ | TimedOut forces re-observation | Failure |
| N11 | Losing Loom does not invalidate Cell state | Cell observations are independent of Loom liveness | Cell owns observation and local Event Log | Failure |
| N12 | Canon compilation is deterministic | $\mathrm{compile}(C) = \mathrm{compile}(C') \text{ when } \mathrm{canon}(C)=\mathrm{canon}(C')$ | Canonical IR encoding; content-derived `CanonID` | Property (determinism) |

## Architectural Leverage

The cheapest way to enforce an invariant is to make its violation fail to compile. These proposals need an ADR:

- **N1.** Split the Substrate port into an *observe* capability and a *mutate* capability. Trace gets only the first, so the compiler rules out mutation.
- **N6.** `Succeeded` can only be constructed from a `Verification` value.
- **N7.** The Store port has no update or delete operations.
- **N8.** Secret bytes are a type whose `Serialize`, `Debug`, and `Display` implementations, if any, reveal nothing.
