# Invariants

The constitutional layer (spec §58). Each invariant appears as a predicate, next to the mechanism that establishes it and the tests that check it.

| # | Statement | Predicate | Established by | Tested by |
| --- | --- | --- | --- | --- |
| N1 | Trace does not mutate Substrate | $\pi(S_{before}(\mathrm{Trace})) = \pi(S_{after}(\mathrm{Trace}))$ | Trace holds only the observe capability of the Substrate port | Property (mock), integration |
| N2 | Successful Enforce converges toward Canon | $\mathrm{Enforce}(C,S)=S' \wedge \mathrm{ok} \Rightarrow \mathrm{Converged}(C,S')$ | Verify after apply; re-observe loop ([reconciliation](reconciliation.md)) | Property, integration |
| N3 | Re-enforcing converged Canon mutates nothing | $\mathrm{Converged}(C,S) \Rightarrow \mathrm{mut}(\mathrm{Enforce}(C,S))=\varnothing$ | Plan is a pure function of Assessments and Obligations; $\mathrm{Converged}$ includes discharged Obligations and Settled effects ([reconciliation](reconciliation.md#convergence)) | Property (fixed point) |
| N4 | No Action runs before its hard dependencies succeed | $\mathrm{Running}(a) \Rightarrow \forall d \in \mathrm{Req}(a): \sigma(d)=\mathrm{Succeeded}$ | Execution frontier ([warp](warp.md)) | Graph, TLA+ `PlanExecution` |
| N5 | A stale fenced Plan cannot execute | $\mathrm{Execute}(a) \Rightarrow g(a) \ge g^{acc}_n$ | Generation check before execution ([fencing](fencing-and-idempotency.md)) | TLA+ `Fencing`, failure |
| N6 | Succeeded requires verified postconditions | $\sigma(a)=\mathrm{Succeeded} \Rightarrow \mathrm{Verified}(a)$ | Only `Verifying → Succeeded` exists | TLA+ `ActionLifecycle` |
| N7 | Events are never mutated after append | $e \in L_t \Rightarrow \forall t' > t: L_{t'}[\mathrm{pos}(e)] = e$ | Store port exposes append only | Property, crash |
| N8 | Cipher plaintext never persists | $\forall e \in L: \mathrm{plaintext}(c) \not\sqsubseteq \mathrm{ser}(e)$ | Cipher values are opaque secret types that cannot be serialized | Property (secret non-disclosure) |
| N9 | Failure budgets are never intentionally exceeded | $\mathrm{admit}(a, f) \Rightarrow \mathrm{Fresh}(\mathit{snap}) \wedge \lvert U_f \cup R_f \cup D_f(a) \rvert \le k_f$ | Serialized admission in Loom scheduling against a fresh observation snapshot ([warp](warp.md#runnable-set)) | TLA+, failure |
| N10 | Unknown outcomes stay unknown | $\mathrm{TimedOut}(a) \Rightarrow \neg\mathrm{assume}(\mathrm{Succeeded}) \wedge \neg\mathrm{assume}(\mathrm{Failed})$ | TimedOut forces re-observation | Failure |
| N11 | Losing Loom does not invalidate collected Observations | Cell Observations are independent of Loom liveness | Cell owns observation and local Event Log | Failure |
| N12 | Canon compilation is deterministic | $\mathrm{compile}(C) = \mathrm{compile}(C') \text{ when } \mathrm{canon}(C)=\mathrm{canon}(C')$ | Canonical IR encoding; content-derived `CanonID`. The authoring build that produces the IR is outside N12 and tested separately ([ADR 0004](../adr/0004-rust-typed-canon.md)) | Property (determinism) |
| N13, proposed | Unknown evidence does not imply noncompliance | $A_r = \mathrm{Indeterminate}(\rho) \Rightarrow A_r \notin \mathit{Variances} \wedge \forall a \in P: \mathrm{cause}(a) \in \mathit{Variances} \cup \mathit{Obligations}$ | `assess` returns Indeterminate for failed or insufficient evidence; $P$ takes no input from Indeterminate Assessments ([reconciliation](reconciliation.md#model)) | Not run. Milestone `03-assessment-kernel` truth tables and property tests |

## What the Predicates Mean

**N13 is proposed, not established.** It is numbered N13 so that N1–N12 keep their numbers, and it becomes an invariant when the tests in its row exist. The Obligation clause matters: a service whose loaded configuration cannot be observed has an Indeterminate Assessment and may still owe a refresh. The refresh is caused by the Obligation, not by the Indeterminate evidence.

Three of the predicates changed after the 2026-09-28 research review ([evaluation](../research/2026-09-28-typed-core/README.md)). The changes narrow each claim to what a mechanism can establish.

**N1 compares a projection.** Literal whole-machine equality is false for any observer. Reading a file updates its access time, and the audit subsystem records the read. $\pi$ projects the machine onto the properties Canon can express for the resources it names. The observe-only capability rules out calls to mutate methods; it cannot prove that an adapter has no side effects. That residue is tested, not typed: property tests on the mock, and integration tests that diff $\pi$ on Linux.

**N5 holds at the moment of the effect.** The predicate is stated at execution. The check in [fencing-and-idempotency.md](fencing-and-idempotency.md#fencing) happens before dispatch, and between that check and the operating-system effect a newer generation can be accepted. The N5 theorem there names the atomicity assumption it needs. Closing the gap is [ADR 0010](../adr/0010-effect-recovery-and-fencing.md), proposed.

**N3 is stated over convergence, not empty Variance.** A host whose files match but whose service still owes a refresh has a pending Obligation. It is not converged, and discharging the Obligation is a required mutation. Stating N3 over empty Variance would forbid that recovery.

**N9 is about admission, not the world.** The earlier predicate, $\forall f: \mathrm{Unavailable}(f) \le k_f$, promised something no controller can keep. Two nodes in one rack can lose power together, and no admission decision prevented it. Spec §58 says budgets are never *intentionally* exceeded, and the predicate now says the same, over sets of node identities: $U_f$ is the nodes of failure domain $f$ unavailable in the snapshot the decision used, $R_f$ the nodes reserved by admitted disruptive Actions, and $D_f(a)$ the nodes Action $a$ would disrupt in $f$. A node is counted once even if it is in more than one set. $\mathrm{Fresh}(\mathit{snap})$ is the freshness policy on that snapshot; when it fails, or the headroom is gone, nothing new is admitted. Admissions are serialized per domain: a correct predicate evaluated concurrently by two admissions against one snapshot can still over-admit. Involuntary failures beyond the budget are reported, not denied.

## Three Questions per Invariant

Each invariant can be checked in three senses, and the [verification matrix](verification-matrix.md) keeps a column for each. **Specification correctness**: the predicate above is consistent with the algorithm documents and with the other invariants; a TLA+ model or a counterexample model speaks to this. **Implementation conformance**: the Rust function the *Established by* column names satisfies the predicate on the inputs a test, a property generator, a semantic mutant, or a bounded harness supplied. **Environment and adapter correspondence**: the Observation an adapter returned, or the effect it performed, is what the predicate assumed; only a conformance suite on a real host speaks to this. A row in the table above is checked in the third sense only when its *Tested by* column names an integration test, and today none does.

## Architectural Leverage

The cheapest way to enforce an invariant is to make its violation fail to compile. These proposals need an ADR:

- **N1.** Split the Substrate port into an *observe* capability and a *mutate* capability. Trace gets only the first, so the compiler rules out mutation.
- **N6.** `Succeeded` can only be constructed from a `Verification` value.
- **N7.** The Store port has no update or delete operations.
- **N8.** Secret bytes are a type whose `Serialize`, `Debug`, and `Display` implementations, if any, reveal nothing.
- **N12.** A private constructor is not a boundary if a derived decoder can fill the fields. Decode into an untrusted data-transfer object, then convert fallibly into the validated type (`serde`'s `try_from`). Constructors, decoders, and migrations share one validator.

A type establishes one thing: the transition was gated. It says nothing about whether the gate's evidence was true. `Succeeded` built only from a `Verification` value shows that verification ran; what makes the verification mean anything is the verifier, the observation's provenance, the identity of the target, and the freshness of the observation. Those are tested, and the tests are the grounding plan's `assessment-algebra` and `bounded-convergence`.
