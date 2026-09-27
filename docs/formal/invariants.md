# Invariants

The constitutional layer (spec §58). Each invariant is stated as a predicate
and paired with the mechanism and the test layer that establish it.

| # | Statement | Predicate | Established by | Tested by |
|---|---|---|---|---|
| N1 | Trace does not mutate Substrate | $S_{before}(\mathrm{Trace}) = S_{after}(\mathrm{Trace})$ | Trace holds only the observe capability of the Substrate port | property (mock), integration |
| N2 | Successful Enforce converges toward Canon | $\mathrm{Enforce}(C,S)=S' \wedge \mathrm{ok} \Rightarrow \mathrm{diff}(C,S')=\varnothing$ | verify after apply; re-observe loop ([reconciliation](reconciliation.md)) | property, integration |
| N3 | Re-enforcing converged Canon mutates nothing | $\mathrm{diff}(C,S)=\varnothing \Rightarrow \mathrm{mut}(\mathrm{Enforce}(C,S))=\varnothing$ | plan is a function of Variance only | property (fixed point) |
| N4 | No Action runs before its hard dependencies succeed | $\mathrm{Running}(a) \Rightarrow \forall d \in \mathrm{Req}(a): \sigma(d)=\mathrm{Succeeded}$ | execution frontier ([warp](warp.md)) | graph, TLA+ `PlanExecution` |
| N5 | A stale fenced Plan cannot execute | $\mathrm{Execute}(a) \Rightarrow g(a) \ge g^{acc}_n$ | generation check before execution ([fencing](fencing-and-idempotency.md)) | TLA+ `Fencing`, failure |
| N6 | Succeeded requires verified postconditions | $\sigma(a)=\mathrm{Succeeded} \Rightarrow \mathrm{Verified}(a)$ | only `Verifying → Succeeded` exists | TLA+ `ActionLifecycle` |
| N7 | Events are never mutated after append | $e \in L_t \Rightarrow \forall t' > t: L_{t'}[\mathrm{pos}(e)] = e$ | Store port exposes append only | property, crash |
| N8 | Cipher plaintext never persists | $\forall e \in L: \mathrm{plaintext}(c) \not\sqsubseteq \mathrm{ser}(e)$ | Cipher values are opaque secret types that cannot be serialized | property (secret non-disclosure) |
| N9 | Failure budgets are never intentionally exceeded | $\forall f: \mathrm{Unavailable}(f) \le k_f$ | admission control in Loom scheduling ([warp](warp.md#runnable-set)) | TLA+, failure |
| N10 | Unknown outcomes stay unknown | $\mathrm{TimedOut}(a) \Rightarrow \neg\mathrm{assume}(\mathrm{Succeeded}) \wedge \neg\mathrm{assume}(\mathrm{Failed})$ | TimedOut forces re-observation | failure |
| N11 | Losing Loom does not invalidate Cell state | Cell observations are independent of Loom liveness | Cell owns observation and local Event Log | failure |
| N12 | Canon compilation is deterministic | $\mathrm{compile}(C) = \mathrm{compile}(C') \text{ when } \mathrm{canon}(C)=\mathrm{canon}(C')$ | canonical IR encoding; content-derived `CanonID` | property (determinism) |

## Architectural leverage

Several invariants are enforced most cheaply by the shape of the types.
These are proposals to confirm by ADR:

- **N1**: split the Substrate port into an *observe* capability and a
  *mutate* capability. The Trace use case is constructed with only the first,
  so the compiler rules out mutation.
- **N6**: make `Succeeded` constructible only from a `Verification` value.
- **N7**: the Store port has no update or delete operations.
- **N8**: secret bytes are a type with no `Serialize`, `Debug` or `Display`
  implementation that reveals content.
