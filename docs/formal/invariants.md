# Invariants

The constitutional layer (spec §58). Each invariant appears as a predicate, next to the mechanism that establishes it and the tests that check it.

| # | Statement | Predicate | Established by | Tested by |
| --- | --- | --- | --- | --- |
| N1 | Trace does not mutate Substrate | $\pi(S_{before}(\mathrm{Trace})) = \pi(S_{after}(\mathrm{Trace}))$ | Trace holds only the observe capability of the Substrate port | Property (mock), integration |
| N2 | Successful Enforce converges toward Canon | $\mathrm{Enforce}(C,S)=S' \wedge \mathrm{ok} \Rightarrow \mathrm{diff}(C,S')=\varnothing$ | Verify after apply; re-observe loop ([reconciliation](reconciliation.md)) | Property, integration |
| N3 | Re-enforcing converged Canon mutates nothing | $\mathrm{diff}(C,S)=\varnothing \Rightarrow \mathrm{mut}(\mathrm{Enforce}(C,S))=\varnothing$ | Plan is a function of Variance only | Property (fixed point) |
| N4 | No Action runs before its hard dependencies succeed | $\mathrm{Running}(a) \Rightarrow \forall d \in \mathrm{Req}(a): \sigma(d)=\mathrm{Succeeded}$ | Execution frontier ([warp](warp.md)) | Graph, TLA+ `PlanExecution` |
| N5 | A stale fenced Plan cannot execute | $\mathrm{Execute}(a) \Rightarrow g(a) \ge g^{acc}_n$ | Generation check before execution ([fencing](fencing-and-idempotency.md)) | TLA+ `Fencing`, failure |
| N6 | Succeeded requires verified postconditions | $\sigma(a)=\mathrm{Succeeded} \Rightarrow \mathrm{Verified}(a)$ | Only `Verifying → Succeeded` exists | TLA+ `ActionLifecycle` |
| N7 | Events are never mutated after append | $e \in L_t \Rightarrow \forall t' > t: L_{t'}[\mathrm{pos}(e)] = e$ | Store port exposes append only | Property, crash |
| N8 | Cipher plaintext never persists | $\forall e \in L: \mathrm{plaintext}(c) \not\sqsubseteq \mathrm{ser}(e)$ | Cipher values are opaque secret types that cannot be serialized | Property (secret non-disclosure) |
| N9 | Failure budgets are never intentionally exceeded | $\mathrm{admit}(a, f) \Rightarrow \lvert \mathrm{Unavailable}_{snap}(f) \cup \mathrm{Reserved}(f) \cup \{a\} \rvert \le k_f$ | Admission control in Loom scheduling against a fresh observation snapshot ([warp](warp.md#runnable-set)) | TLA+, failure |
| N10 | Unknown outcomes stay unknown | $\mathrm{TimedOut}(a) \Rightarrow \neg\mathrm{assume}(\mathrm{Succeeded}) \wedge \neg\mathrm{assume}(\mathrm{Failed})$ | TimedOut forces re-observation | Failure |
| N11 | Losing Loom does not invalidate Cell state | Cell observations are independent of Loom liveness | Cell owns observation and local Event Log | Failure |
| N12 | Canon compilation is deterministic | $\mathrm{compile}(C) = \mathrm{compile}(C') \text{ when } \mathrm{canon}(C)=\mathrm{canon}(C')$ | Canonical IR encoding; content-derived `CanonID` | Property (determinism) |

## What the Predicates Mean

Three of the predicates changed after the 2026-09-28 research review ([evaluation](../research/2026-09-28-typed-core/README.md)). The changes narrow each claim to what a mechanism can establish.

**N1 compares a projection.** Literal whole-machine equality is false for any observer. Reading a file updates its access time, and the audit subsystem records the read. $\pi$ projects the machine onto the properties Canon can express for the resources it names. The observe-only capability rules out calls to mutate methods; it cannot prove that an adapter has no side effects. That residue is tested, not typed: property tests on the mock, and integration tests that diff $\pi$ on Linux.

**N5 holds at the moment of the effect.** The predicate is stated at execution. The check in [fencing-and-idempotency.md](fencing-and-idempotency.md#fencing) happens before dispatch, and between that check and the operating-system effect a newer generation can be accepted. The N5 theorem there names the atomicity assumption it needs. Closing the gap is ADR candidate `recovery-authority`.

**N9 is about admission, not the world.** The earlier predicate, $\forall f: \mathrm{Unavailable}(f) \le k_f$, promised something no controller can keep. Two nodes in one rack can lose power together, and no admission decision prevented it. Spec §58 says budgets are never *intentionally* exceeded, and the predicate now says the same: an Action is admitted into failure domain $f$ only if the nodes already unavailable in the snapshot the decision used, plus the nodes reserved by admitted disruptive Actions, plus this one, fit within $k_f$. A node is counted once even if it is both unavailable and reserved. When the snapshot is stale or the headroom is gone, nothing new is admitted. Involuntary failures beyond the budget are reported, not denied.

## Architectural Leverage

The cheapest way to enforce an invariant is to make its violation fail to compile. These proposals need an ADR:

- **N1.** Split the Substrate port into an *observe* capability and a *mutate* capability. Trace gets only the first, so the compiler rules out mutation.
- **N6.** `Succeeded` can only be constructed from a `Verification` value.
- **N7.** The Store port has no update or delete operations.
- **N8.** Secret bytes are a type whose `Serialize`, `Debug`, and `Display` implementations, if any, reveal nothing.
- **N12.** A private constructor is not a boundary if a derived decoder can fill the fields. Decode into an untrusted data-transfer object, then convert fallibly into the validated type (`serde`'s `try_from`). Constructors, decoders, and migrations share one validator.

A type establishes one thing: the transition was gated. It says nothing about whether the gate's evidence was true. `Succeeded` built only from a `Verification` value shows that verification ran; what makes the verification mean anything is the verifier, the observation's provenance, the identity of the target, and the freshness of the observation. Those are tested, and the tests are the grounding plan's `assessment-algebra` and `bounded-convergence`.
