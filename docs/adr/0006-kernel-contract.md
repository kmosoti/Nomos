# ADR 0006: The Kernel Is a Pure Transition Function and Owns Reconciliation Semantics

- **Status.** Accepted, with the items under *Not Decided Here* open
- **Date.** 2026-09-28, amended 2026-09-28
- **Provenance.** The ownership table and the one-engine rule come from the review of `main` at `013b9d0` that the project owner supplied, and the owner set the executable kernel contract as the next milestone. The owner did not state these decisions separately; if they should be revisited before milestone 1 PR 4, the status is Proposed.

## Context

Spec §9 put `observe`, `diff`, `plan`, `apply`, and `verify` on one driver trait that each backend implemented. Read literally, the mock backend and the Linux backend would each decide what satisfaction, Variance, and a valid Plan mean. Tests run against the mock; production runs against Linux. The mock would implement whatever interpretation makes the tests pass, and nothing would notice that Linux implements another.

The research snapshot's `pure-kernel` recommendation asked for a deterministic transition kernel with explicit effects. The review of `main` at `013b9d0` made the ownership argument above and asked for one transition function that runs in production and in a deterministic simulator. The project owner set the next milestone as an executable kernel contract.

## Decision

### 1. Ownership by Layer

| Layer | Owns | Must not own |
| --- | --- | --- |
| Core (`nomos-core`) | Conditions, evidence interpretation, Assessments, Obligations, transition rules | Filesystem access, clock reads, network access |
| Canon (`nomos-canon`) | Construction, validation, normalization, Canonical IR encoding and decoding | Host mutation, undeclared host inspection |
| Warp (`nomos-warp`) | Graph validation, activation, readiness, conflict-aware selection | Starting tasks, calling the operating system |
| Application (`nomos-app`) | Running use cases, recording intent, invoking ports, feeding results back into the kernel | Linux- or Vault-specific detail |
| Ports | Typed requests, Observations, effect receipts, durability contracts | Transport or OS-library types leaking through the interface |
| Adapters | Obtaining evidence and performing concrete operations | A second implementation of assessment or planning |

Resource-specific pure logic exists. Assessing a file Condition against a file Observation, or deciding what counts as settlement evidence for a systemd job, is resource-specific and lives in core, shared by the mock and Linux paths. An adapter returns an Observation or an effect receipt and has no channel through which to return an interpretation.

### 2. The Kernel Is One Function

$$
\mathrm{step}(\mathit{KernelSnapshot},\ \mathit{Input}) = \mathit{Decision}
$$

A `KernelSnapshot` is the control state: the accepted Canonical IR, the Assessments so far, the Action lifecycle states, reservations, Obligations, and the effects not yet Settled. An `Input` is one thing that happened: an Observation arrived, an effect receipt arrived, a deadline passed, an authority presented a Plan, a recovery replayed the log. A `Decision` is the next snapshot, the Events to record, and the effect requests to issue. Clocks, deadlines, identifiers, capabilities, and policy enter as data in the snapshot or the input. The function performs no I/O and reads no ambient state.

### 3. One Engine, Two Drivers

The application layer drives `step` in production, interpreting effect requests through ports and feeding receipts back. The deterministic simulator drives the same `step` with scripted inputs: delayed receipts, duplicates, crashes modeled as dropped receipts, and superseding authority. There is no test build of the engine. Replaying a recorded input sequence must reproduce the decisions exactly. That is a requirement on the kernel, tested in milestone `05-transition-kernel`. It is not N12, which covers Canon compilation, and it is not a numbered invariant until those tests exist.

### 4. Ports Carry Data, Not Behavior

A port trait exposes typed requests and typed results. `nomos-substrate` exposes `observe` and `apply` as spec §9 now states. No port method returns an Assessment, a Plan, or a verdict.

### 5. Where Cross-Layer Tests Live

The workspace is virtual, so a test must belong to a package. Conformance suites that wire the application to adapters live at a composition boundary, `crates/bin/nomos-cell/tests/` first, or in a dedicated conformance crate if an ADR adds one. The root `tests/` directory holds fixtures as data that those targets load; Cargo does not discover Rust files placed there.

### Not Decided Here

These are open, and milestones `03-assessment-kernel` to `05-transition-kernel` settle them. None may be treated as decided until its milestone records the choice:

- **The types.** The fields of `KernelSnapshot`, the variants of `Input`, and the shape of `Decision`. §2 names what they must carry, not how.
- **Port signatures.** Whether `observe` and `apply` are asynchronous, how they report transport failure, and how a receipt names the effect it answers.
- **Settlement evidence.** What shows, per resource kind, that an effect can cause no further change ([warp.md](../formal/warp.md#conflict-keys) gives examples, not a contract).
- **Loom.** Whether Loom runs the same `step` over fleet state or a separate kernel with the same discipline. Phase 3.
- **A conformance crate.** Whether cross-layer suites outgrow `nomos-cell/tests/`. That would be a new crate and a new ADR.

### Related Decisions

Ownership of individual resource properties across controllers is [ADR 0008](0008-ownership-and-identity.md). Readiness, activation, and reservations are [ADR 0009](0009-warp-activation-semantics.md). Effect recovery and fencing are [ADR 0010](0010-effect-recovery-and-fencing.md). The Event ordering the kernel emits is [ADR 0012](0012-event-history.md). All four are proposed and plug into `step`.

### Assumptions

- Every effect the kernel needs can be expressed as a typed request and answered with a typed receipt. An effect that needs a callback into the kernel mid-flight is a design smell to be resolved by splitting it, not by widening the port.
- The control state fits in memory per Cell. Fleet state at Loom is Phase 3 and does not change this contract.

### Alternatives

- **Per-backend semantics** (the previous §9). Rejected, for the reason in the context.
- **Shared semantics in a trait default implementation** that adapters may override. Rejected. An override is the same hole with a politer name.
- **A separate simplified engine for tests.** Rejected. It tests the simplified engine.

### Evidence

Research recommendation `pure-kernel` and finding `single-controller` are arguments, not measurements. Milestone `01-foundation-gates` added the first mechanical evidence of §1's layer boundaries: `cargo xtask check-layers` rejects an adapter dependency in core, ports, or app ([ADR 0007](0007-verification-gates.md)). The semantic half of §1, that no adapter can return an interpretation, and §2 to §4 are owed by milestone 1 PR 2 to PR 4.

## Note, 2026-09-28: The Transition Kernel's Working Definitions

Milestone `05-transition-kernel` needs answers to four items under *Not Decided Here*. It takes these as working definitions; the ADR stays Accepted for §1 to §5 and the items stay open until the owner accepts them.

- **Where `step` lives.** The transition rules are in `nomos-core`: `action` holds the Action lifecycle, `plan` the fence, and `effect` the idempotency key, the effect request, the receipt, and settlement. `step` composes those rules with Warp's frontier and selection, and `nomos-core` may not depend on `nomos-warp`. So `step` lives in the `kernel` module of `nomos-app`, which the grounding plan names for this milestone. It performs no I/O, reads no clock, and names no port. The purity checker covers `crates/core/` only, so for `nomos-app` the purity of `step` rests on review and on the replay tests; moving `step` into a core crate would be a new crate and a new ADR, and that choice belongs to the owner.
- **The types.** An `Input` is one of: an authority presents a Plan (`Enforce`), Observations arrive (`Observed`), a receipt arrives (`Receipt`), time advances (`Tick`), or the process restarted and replayed its log (`Recovered`). A `Decision` carries the Events to record, the effect requests to issue, and the next `KernelSnapshot`. The next snapshot is the fold of the Decision's Events over the previous one, and applying an Event is the only way a snapshot changes. Replaying the Event Log from the initial snapshot therefore reproduces the snapshot, which is [event-log.md](../formal/event-log.md)'s control state as a fold, and a crash test compares the two.
- **Port signatures.** Synchronous and typed. The Substrate port splits into an observe capability, which takes resource paths and returns Observations, and a mutate capability, which takes an effect request and returns receipts, each naming its effect by idempotency key. A transport failure during observation is a collection failure, which is Indeterminate; a lost receipt is a deadline that passes.
- **Settlement evidence, for the file family and the service refresh.** A terminal receipt settles an effect: completed, failed, or refused. So does the effect's settle-by instant passing: every effect request carries one, and the Substrate contract is that an effect not finished by then can cause no further change. An acceptance or start receipt settles nothing, and neither does a satisfied re-observation ([warp.md](../formal/warp.md#conflict-keys)).

The kernel's Canon input is the part of a compiled Canon the kernel reads: for each managed resource its Condition, its operation (replace a file, or refresh a service), its conflict keys, and the nodes its Action would disrupt, plus the edges. The artifact form of that input is `06-canon-artifact`'s.

## Consequences

- Spec §9 is amended in the same change. The mock backend is a first-class backend of the same semantics, as spec §9 already claimed and can now mean.
- `nomos-substrate` narrows to evidence and effects. The observe-only capability split for Trace (invariants, Architectural Leverage) applies to this narrower port.
- Recovery is a replay of recorded inputs into `step`, followed by fresh Observations, matching [event-log.md](../formal/event-log.md).
- **Verification.** Milestone `05-transition-kernel`: the transition kernel with a recovery simulator, the first executable TLA+ model of the same transitions, and replayable traces. Milestones `03-assessment-kernel` and `04-warp-kernel` supply the assessment and Warp functions `step` calls.
- **Failure behavior.** A kernel given an input it cannot interpret, for example a receipt for an unknown effect, records the fact as an Event and leaves the snapshot unchanged; it never guesses.
- **Revisit trigger.** Reopen if an effect cannot be expressed without a callback, if the simulator and production drivers diverge in a way the input model cannot capture, or if the owner does not accept the provenance above.
