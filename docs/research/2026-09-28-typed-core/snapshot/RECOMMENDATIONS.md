# Recommendation Index

Snapshot: 2026-09-28 UTC. These are proposed research recommendations, not accepted ADRs or verified implementations. Importance and salience use ordinal 0–4 judgments. Evidence strength remains separate. See README.md for the rubric.

| Recommendation key | Importance | Salience | Disposition | Phase |
| --- | ---: | ---: | --- | --- |
| [agent-proof](#agent-proof) | 4 | 4 | `adopt_direction` | `phase0` |
| [algebraic-model](#algebraic-model) | 4 | 4 | `adopt_direction` | `phase0` |
| [budget](#budget) | 4 | 4 | `adopt_direction` | `before_fleet_mutation` |
| [canonical-profile](#canonical-profile) | 4 | 4 | `trial` | `phase0` |
| [composition](#composition) | 4 | 4 | `research_required` | `phase0` |
| [durable-refresh](#durable-refresh) | 4 | 4 | `research_required` | `phase0` |
| [effect-recovery](#effect-recovery) | 4 | 4 | `adopt_direction` | `phase0` |
| [evidence-assessment](#evidence-assessment) | 4 | 4 | `adopt_direction` | `phase0` |
| [fencing](#fencing) | 4 | 4 | `research_required` | `before_remote_execution` |
| [formal](#formal) | 4 | 4 | `adopt_direction` | `phase0` |
| [log-boundary](#log-boundary) | 4 | 4 | `adopt_direction` | `phase0` |
| [progress](#progress) | 4 | 4 | `adopt_direction` | `phase0` |
| [pure-kernel](#pure-kernel) | 4 | 4 | `adopt_direction` | `phase0` |
| [scheduler](#scheduler) | 4 | 4 | `adopt_direction` | `phase0` |
| [typed-canon](#typed-canon) | 4 | 4 | `adopt_direction` | `phase0` |
| [validated-boundary](#validated-boundary) | 4 | 4 | `adopt_direction` | `phase0` |
| [warp-semantics](#warp-semantics) | 4 | 4 | `adopt_direction` | `phase0` |
| [bounded-bindings](#bounded-bindings) | 3 | 4 | `adopt_direction` | `phase0` |
| [layer-enforcement](#layer-enforcement) | 3 | 4 | `adopt_direction` | `phase0` |
| [cipher](#cipher) | 4 | 3 | `adopt_direction` | `before_secret_support` |
| [compatibility](#compatibility) | 4 | 3 | `adopt_direction` | `phase0` |
| [identity-recovery](#identity-recovery) | 4 | 3 | `research_required` | `before_remote_execution` |
| [log-retention](#log-retention) | 4 | 3 | `research_required` | `before_durable_execution` |
| [substrate](#substrate) | 4 | 3 | `adopt_direction` | `before_linux_mutation` |
| [incremental](#incremental) | 3 | 2 | `defer_until_measured` | `after_phase0` |
| [plan-witness](#plan-witness) | 3 | 2 | `research_required` | `after_phase0` |

## agent-proof

**Protect invariants from verification gaming by coding agents**

Require separate review for specification or assumption changes. Reject new assume/admit/axiom or external-body escape hatches unless reviewed as explicit trust boundaries. Record actual verifier output and use negative-control mutants.

**Priority rationale:** Agent-driven development can weaken the specification it is meant to satisfy; protect review gates from the first formal change.

**Constraints:** Regex checks alone are not a proof that a specification was preserved. Semantic weakening and disabled test paths require review.

**Acceptance criteria:** A patch that weakens a postcondition or replaces proof with an assumption fails the gate.

**Proposed experiments:** `agent-proof-gate`.

**Proposed locations:** `AGENTS.md`, `docs/formal/verification-matrix.md`.

**Provenance:** [VeruSAGE v2](https://arxiv.org/html/2512.18436v2).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## algebraic-model

**Use sum types to remove contradictions, not just to rename tags**

Represent FileCondition as Absent or Present(FileRequirements), Assessment as typed alternatives, and Events as an envelope containing one tagged payload enum. Keep Option only for deliberately unconstrained fields.

**Priority rationale:** Defines legal domain values and prevents broad invalid-state classes; shapes all Phase 0 APIs.

**Constraints:** Prefer closed resource variants for v0. Do not export opaque map-of-values extensions into the trusted kernel.

**Acceptance criteria:** Compile-fail tests reject invalid variant combinations and mutation of private invariants. Round-trip tests preserve explicit enum discriminants.

**Proposed experiments:** `typed-validation`.

**Proposed locations:** `crates/core/nomos-core`, `docs/CANON.md`.

**Provenance:** [Rust Reference: non_exhaustive](https://doc.rust-lang.org/reference/attributes/type_system.html); [Rust Reference: enumerations](https://doc.rust-lang.org/reference/items/enumerations.html); [Serde: container attributes](https://serde.rs/container-attrs.html).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## budget

**Replace impossible world-wide safety with admission safety**

Define N9 over admitted disruptive reservations using a fresh observation snapshot. Count pending, running, and unresolved reservations, and count unavailable nodes once. Stop new disruption when evidence is stale or headroom is exhausted.

**Priority rationale:** The current invariant promises more than admission control can enforce; repair the claim now before fleet mutation.

**Constraints:** Independent hardware faults may exceed the threshold; report rather than deny that reality. Budget checks and reservation commit need atomicity under concurrent requests.

**Acceptance criteria:** Concurrent admissions cannot oversubscribe a budget. Spontaneous failure pauses further disruptive admission.

**Proposed experiments:** `scheduler-admission`.

**Proposed locations:** `docs/formal/invariants.md`, `docs/formal/warp.md`.

**Provenance:** [Kubernetes: disruptions](https://kubernetes.io/docs/concepts/workloads/pods/disruptions/); [Nomos: docs/formal/invariants.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/invariants.md).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## canonical-profile

**Specify semantic normalization separately from encoding**

Trial a restricted deterministic CBOR profile against JCS. Define ordering, defaults, integer ranges, duplicate keys, Unicode/path-byte policy, tags, and forbidden floats before selecting bytes to hash.

**Priority rationale:** Identity and replay depend on stable semantics; settle the accepted data model before artifact interchange.

**Constraints:** No Rust memory-layout serialization. Do not hash protobuf transport bytes as CanonID. Hash domain/version and normalized content; keep provenance outside semantic identity.

**Acceptance criteria:** Permutation and cross-build golden vectors agree for declared semantic equivalences. Semantically different Conditions do not collapse under normalization in the test corpus.

**Proposed experiments:** `canonical-encoding`.

**Proposed locations:** `docs/CANON.md`, `docs/formal/canonicalization.md`.

**Provenance:** [RFC 8949: CBOR](https://www.rfc-editor.org/rfc/rfc8949.html); [RFC 8785: JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785.html); [Protocol Buffers: serialization is not canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## composition

**Model resource ownership and cross-controller interference**

Give each driver or controller explicit read/write footprint, reliance assumptions, guarantees, and progress dependencies. Trial Anvil/Welder-style composition on two conflicting controllers before attempting whole-host proofs.

**Priority rationale:** Independent convergence is insufficient when controllers interfere; a small Phase 0 counterexample prevents a misleading foundation.

**Constraints:** Host resource semantics differ from Kubernetes objects; transfer the proof discipline, not its model unchanged. Partial configuration ownership requires an explicit conflict policy.

**Acceptance criteria:** Two controllers that oscillate in composition are rejected or detected even when each converges in isolation.

**Proposed experiments:** `controller-composition`.

**Proposed locations:** `docs/formal/reconciliation.md`, `docs/formal/ownership.md`.

**Provenance:** [Anvil: Verifying Liveness of Cluster Management Controllers](https://www.usenix.org/conference/osdi24/presentation/sun-xudong); [Anvil and Welder project documentation](https://github.com/anvil-verifier/anvil); [Nomos: docs/formal/reconciliation.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/reconciliation.md); [systemd D-Bus API manual](https://man7.org/linux/man-pages/man5/org.freedesktop.systemd1.5.html); [Welder: Compositional Liveness Verification of Cluster Control Planes](https://cathy-cai.page/pubs/welder26.pdf).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## durable-refresh

**Make refresh obligations durable or observable**

Prefer a condition that the service has applied a specific configuration revision when the application can attest it. Otherwise record a durable refresh obligation before changing config and settle it through an explicit recovery policy.

**Priority rationale:** A crash can lose a required service refresh while files appear converged; its semantics affect Phase 0 planning.

**Constraints:** A running service is not evidence of a particular loaded configuration. systemd invocation identity alone may not reveal application reload semantics. Exactly-once refresh is not promised for opaque services.

**Acceptance criteria:** Crash after file replacement but before restart does not lose the outstanding refresh obligation.

**Proposed experiments:** `refresh-recovery`.

**Proposed locations:** `docs/formal/warp.md`, `docs/formal/fencing-and-idempotency.md`.

**Provenance:** [Nomos: docs/formal/fencing-and-idempotency.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/fencing-and-idempotency.md); [Nomos: docs/formal/warp.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/warp.md); [systemd D-Bus API manual](https://man7.org/linux/man-pages/man5/org.freedesktop.systemd1.5.html); [Temporal Activity definition](https://docs.temporal.io/activity-definition).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## effect-recovery

**Declare effect-specific recovery contracts**

Classify Actions by inspectability and repeatability. Use a stable key within one execution, a new run identity for later drift repair, and an explicit Indeterminate outcome for lost completion. Require operator recovery for effects that cannot be safely resolved.

**Priority rationale:** Retry and new-run identity determine duplicate effects and suppressed repairs; define before execution.

**Constraints:** Keep desired semantic Action digest separate from execution key. Do not retry an external job while it may still be active. Postcondition satisfaction does not always establish who caused it.

**Acceptance criteria:** Duplicate delivery in one run never creates a second live operation. A later run can repair the same resource after new drift.

**Proposed experiments:** `effect-recovery`.

**Proposed locations:** `docs/formal/fencing-and-idempotency.md`, `crates/core/nomos-core`.

**Provenance:** [Nomos: docs/formal/fencing-and-idempotency.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/fencing-and-idempotency.md); [Nomos: docs/formal/warp.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/warp.md); [systemd D-Bus API manual](https://man7.org/linux/man-pages/man5/org.freedesktop.systemd1.5.html); [Temporal Activity definition](https://docs.temporal.io/activity-definition).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## evidence-assessment

**Use per-condition evidence and three-way assessment**

Keep Known(Present), Known(Absent), and Indeterminate distinct. Bind assessment to condition revision, resource identity, observation source, collection window, and freshness policy. Preserve mixed results across a Canon.

**Priority rationale:** Unknown evidence must not become a mutation command; this is the core Phase 0 decision algebra.

**Constraints:** Start v0 with a conjunction of resource Conditions; defer arbitrary NOT and OR mutation planning. Conflicting observations become an explicit reason for Indeterminate unless a documented authority resolves them.

**Acceptance criteria:** Permission errors never become Absent or Variance. Satisfied requires sufficient relevant evidence, not an empty error list.

**Proposed experiments:** `assessment-algebra`.

**Proposed locations:** `docs/formal/reconciliation.md`, `crates/core/nomos-core`.

**Provenance:** [Nomos: docs/formal/invariants.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/invariants.md); [Rust Reference: enumerations](https://doc.rust-lang.org/reference/items/enumerations.html); [User-approved Nomos direction](conversation:2026-09-28T00:46:03Z).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## fencing

**Put fencing and authorization at the actual effect boundary**

Serialize admission and reservation at Cell, validate authenticated authority before persisting a fence, and reject lower accepted fences at effect admission. Define quiescence or safe draining before conflicting supersession.

**Priority rationale:** Essential for remote authority safety; agree on its effect boundary now even though networking is later.

**Constraints:** Local acceptance cannot detect a newer fence the Cell has never received. An already issued OS job is not revoked by updating a counter. Loom and standalone Cell requests share one local arbitration path.

**Acceptance criteria:** Pause old work after precheck, accept a newer plan, then resume old work: no conflicting unreserved mutation occurs.

**Proposed experiments:** `fence-interleavings`.

**Proposed locations:** `docs/formal/fencing-and-idempotency.md`, `docs/PROTOCOL.md`.

**Provenance:** [Nomos: docs/formal/fencing-and-idempotency.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/fencing-and-idempotency.md); [Temporal Activity definition](https://docs.temporal.io/activity-definition); [The Update Framework specification](https://theupdateframework.github.io/specification/latest/).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## formal

**Connect formal claims to executable, scoped evidence**

Each invariant needs assumptions, counterexamples, Rust mapping, and executable checks. Start with property tests and bounded protocol models; trial Kani or Verus on a small pure kernel instead of adopting every verifier.

**Priority rationale:** Connects asserted invariants to checkable evidence; the first kernel needs this discipline immediately.

**Constraints:** TLC finite-model checks are not a proof of arbitrary implementations. Record toolchain, solver, bounds, axioms, and unverified adapters. Never populate a verification matrix with passed marks for planned work.

**Acceptance criteria:** Known-bad transitions fail a negative control. Replay fixtures compare implementation decisions to the modeled protocol.

**Proposed experiments:** `kernel-conformance`.

**Proposed locations:** `docs/formal/verification-matrix.md`, `formal/tla`.

**Provenance:** [Anvil: Verifying Liveness of Cluster Management Controllers](https://www.usenix.org/conference/osdi24/presentation/sun-xudong); [Anvil and Welder project documentation](https://github.com/anvil-verifier/anvil); [Nomos: docs/formal/invariants.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/invariants.md); [Rust Reference: enumerations](https://doc.rust-lang.org/reference/items/enumerations.html); [Welder: Compositional Liveness Verification of Cluster Control Planes](https://cathy-cai.page/pubs/welder26.pdf); [Lamport: TLA+ overview](https://lamport.azurewebsites.net/tla/tla.html); [Kani proof and unwind attributes](https://model-checking.github.io/kani/reference/attributes.html); [Proptest book](https://proptest-rs.github.io/proptest/proptest/index.html); [Rust loom documentation](https://docs.rs/loom/latest/loom/); [Verus guide](https://verus-lang.github.io/verus/guide/).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## log-boundary

**Separate control replay, durable outbox, and live truth**

Append decision Events and dispatch intents transactionally before effects. Rebuild control projections from Events, but re-observe the host after recovery. A local database transaction cannot atomically include arbitrary Linux mutations.

**Priority rationale:** Recovery semantics depend on distinguishing recorded decisions from host truth; affects the initial transition model.

**Constraints:** A crash can leave intended or started work without a terminal Event. A durable acknowledgment means persisted under the stated storage contract, not merely buffered in memory.

**Acceptance criteria:** Kill before and after intent commit, effect, completion append, and acknowledgment; recover without false success.

**Proposed experiments:** `event-crash-replay`.

**Proposed locations:** `docs/formal/event-log.md`, `crates/ports/nomos-store`.

**Provenance:** [Nomos: docs/formal/fencing-and-idempotency.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/fencing-and-idempotency.md); [Nomos: docs/formal/event-log.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/event-log.md); [Temporal Activity definition](https://docs.temporal.io/activity-definition).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## progress

**Fix bounded reconciliation and specify conditional liveness**

Reassess after the final permitted execution. Track relevant fingerprints together with outstanding work and provider progress. Treat stagnation and oscillation as evidence-based diagnostics, not conclusions from one repeated sample.

**Priority rationale:** The draft can report nonconvergence after the last successful mutation; correct the core loop before testing claims.

**Constraints:** Bound caller waiting separately from bounding effects. Convergence assumes stable satisfiable intent, adequate permissions, fair opportunities, and bounded interference.

**Acceptance criteria:** The last permitted successful change returns Converged. Slow asynchronous completion is not prematurely classified as oscillation.

**Proposed experiments:** `bounded-convergence`.

**Proposed locations:** `docs/formal/reconciliation.md`.

**Provenance:** [Nomos: docs/formal/reconciliation.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/reconciliation.md); [systemd D-Bus API manual](https://man7.org/linux/man-pages/man5/org.freedesktop.systemd1.5.html); [Temporal Activity definition](https://docs.temporal.io/activity-definition).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## pure-kernel

**Implement a deterministic transition kernel with explicit effects**

Keep core transitions pure: prior control model plus explicit input produces a next model and typed effect requests. The application executes effects and returns observations. Inject clocks, deadlines, IDs, and policy as data.

**Priority rationale:** Determinism enables replay and model comparisons across all subsystems; implement before external adapters.

**Constraints:** A pure kernel may contain state-machine data; removing the overloaded public State noun does not ban state machines. Do not introduce a runtime dependency into the kernel.

**Acceptance criteria:** Recorded inputs replay to identical decisions. Core has no ambient I/O or time dependency.

**Proposed experiments:** `kernel-conformance`.

**Proposed locations:** `crates/core/nomos-core`, `crates/core/nomos-warp`, `crates/app/nomos-app`.

**Provenance:** [Anvil: Verifying Liveness of Cluster Management Controllers](https://www.usenix.org/conference/osdi24/presentation/sun-xudong); [Anvil and Welder project documentation](https://github.com/anvil-verifier/anvil); [Cargo metadata command](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html); [Nomos: docs/adr/0000-foundations.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/adr/0000-foundations.md); [Welder: Compositional Liveness Verification of Cluster Control Planes](https://cathy-cai.page/pubs/welder26.pdf).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## scheduler

**Use conservative resource-footprint admission**

Choose a deterministic feasible subset from activated-ready Actions using conflict reservations and bounded concurrency. Hold reservations through verification and unresolved effects; include pending dispatch, not only Running.

**Priority rationale:** Shared resources and unresolved effects constrain safe concurrency; reservations belong in the initial model.

**Constraints:** Do not materialize the conflict graph unless useful; indexed resource keys suffice. Read/write footprints must cover implicit package/service effects. Maximality requires downward-closed constraints; fairness is separate from deterministic selection.

**Acceptance criteria:** Every selected set respects modeled footprints and reservations. Repeated arrivals cannot starve a task under the documented fairness policy.

**Proposed experiments:** `scheduler-admission`.

**Proposed locations:** `docs/formal/warp.md`, `crates/core/nomos-warp`.

**Provenance:** [Kubernetes: disruptions](https://kubernetes.io/docs/concepts/workloads/pods/disruptions/); [Nomos: docs/formal/invariants.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/invariants.md); [Nomos: docs/formal/warp.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/warp.md).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## typed-canon

**Adopt a typed Rust authoring crate and inert artifact**

Compile and run Canon generation in an isolated, bounded build job without host-management credentials. Loom and Cell accept only inert validated artifacts, not Rust source or native plugins.

**Priority rationale:** Foundational execution boundary; decide before implementing the Rust authoring surface.

**Constraints:** Rust authoring is ordinary code execution, not a purity guarantee. Record toolchain, lockfile, declared inputs, generator version, and artifact digest separately.

**Acceptance criteria:** Two isolated builds with the same declared inputs produce identical canonical artifacts. Denied ambient network/time/environment inputs are tested rather than merely documented.

**Proposed experiments:** `build-hermeticity`.

**Proposed locations:** `docs/CANON.md`, `crates/core/nomos-canon`.

**Provenance:** [Cargo: build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html); [SLSA provenance v1.1](https://slsa.dev/spec/v1.1/provenance); [W3C PROV-O](https://www.w3.org/TR/prov-o/).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## validated-boundary

**Make validation impossible to bypass through supported APIs**

Decode untrusted DTOs, then perform fallible conversion into private-field domain types. Constructors, builders, decoding, and migrations must use the same validation rules.

**Priority rationale:** Invalid decoded intent can cross every later safety boundary; Phase 0 types must enforce this first.

**Constraints:** A signed artifact still requires domain validation. Validation errors must not contain secrets.

**Acceptance criteria:** Malformed ranges, duplicate resource keys, invalid references, unknown executable variants, and oversized inputs fail before planning.

**Proposed experiments:** `typed-validation`.

**Proposed locations:** `docs/CANON.md`, `crates/core/nomos-core`, `crates/core/nomos-canon`.

**Provenance:** [Nomos: docs/formal/invariants.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/invariants.md); [Rust Reference: enumerations](https://doc.rust-lang.org/reference/items/enumerations.html); [Serde: container attributes](https://serde.rs/container-attrs.html).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## warp-semantics

**Separate prerequisite satisfaction, activation, and conflicts**

Retain requirement-satisfaction anchors for no-op resources. Resolve each dependency as Waiting, Satisfied, Disabled, or Blocked. A changed gate requires verified relevant change, not mere terminal status.

**Priority rationale:** The existing readiness formula admits unactivated work and lacks no-op anchors; fix before graph execution.

**Constraints:** Specify all/any semantics for multiple change sources. Distinguish Disabled from failed or indeterminate. Kahn plus stable ties is a baseline; SCC diagnostics need a separately extracted cycle witness.

**Acceptance criteria:** Unchanged config does not activate refresh. Satisfied user prerequisites allow dependent directory creation without a user mutation. Failed partial writes do not trigger dependent service restart.

**Proposed experiments:** `warp-truth-table`.

**Proposed locations:** `docs/formal/warp.md`, `crates/core/nomos-warp`.

**Provenance:** [petgraph algorithm documentation](https://docs.rs/petgraph/latest/petgraph/algo/); [Nomos: docs/formal/reconciliation.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/reconciliation.md); [Nomos: docs/formal/warp.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/warp.md).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## bounded-bindings

**Represent dynamic bindings as finite typed expressions**

Canon may contain a small typed expression AST for arithmetic, clamps, explicit Trait references, and parameters. Resolve against a frozen binding snapshot without embedded Rust closures or a second mandatory authoring language.

**Priority rationale:** Useful capability rather than a safety theorem; decide its limited scope now to prevent hidden ambient inputs.

**Constraints:** Missing input and overflow are explicit outcomes. Resource-sensitive policy checks trusted provenance, not just Trait value.

**Acceptance criteria:** Evaluation is bounded by AST limits and deterministic for fixed bindings. Changing one declared Trait changes the resolved-condition identity predictably.

**Proposed experiments:** `assessment-algebra`.

**Proposed locations:** `docs/CANON.md`, `crates/core/nomos-canon`.

**Provenance:** [Cargo: build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html); [SLSA provenance v1.1](https://slsa.dev/spec/v1.1/provenance); [User-approved Nomos direction](conversation:2026-09-28T00:46:03Z).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## layer-enforcement

**Test the dependency policy rather than assuming Cargo enforces it**

Add a cargo-metadata policy check for workspace edges, including feature and target variants that can introduce adapters into the core. Make exceptions explicit ADR decisions.

**Priority rationale:** Strong architectural leverage but not a runtime safety guarantee; a small Phase 0 check prevents dependency drift.

**Constraints:** No toolchain change is part of this research artifact. Avoid one-port-only dogma where a reviewed adapter composition genuinely needs multiple contracts.

**Acceptance criteria:** An intentionally inserted core-to-adapter dependency is rejected by the architecture check.

**Proposed experiments:** `layer-policy`.

**Proposed locations:** `docs/adr/0000-foundations.md`, `AGENTS.md`.

**Provenance:** [Cargo metadata command](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html); [Nomos: docs/adr/0000-foundations.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/adr/0000-foundations.md).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## cipher

**Keep secret references and secret sinks distinct**

Canon contains typed references, never secret bytes. Resolve at use through an explicit provider; prohibit plaintext in Events, Plans, diagnostics, and build artifacts. Version references or use safe provider metadata for rotation decisions.

**Priority rationale:** Secret leakage is high consequence; concrete sink controls are needed when secret-backed resources arrive.

**Constraints:** A configured protected file can be an intentional secret sink; the no-persistence rule should refer to forbidden sinks. Do not use an unsalted digest of a low-entropy secret as public identity.

**Acceptance criteria:** Sentinel secrets and their common encodings do not occur in operational outputs across failure paths.

**Proposed experiments:** `secret-nondisclosure`.

**Proposed locations:** `docs/security-model.md`, `crates/ports/nomos-cipher`.

**Provenance:** [secrecy crate documentation](https://docs.rs/secrecy/latest/secrecy/).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## compatibility

**Version inert artifacts independently from Rust APIs**

Define separate schema versions and resource capability versions. Reject unknown executable semantics before mutation; permit opaque preservation only on explicitly non-executing archival paths.

**Priority rationale:** Unknown executable variants can change safety semantics; reserve explicit version boundaries early.

**Constraints:** Do not use in-memory enum layout as the wire format. Migrations produce new content identity and preserve original provenance.

**Acceptance criteria:** Compatibility fixtures exercise old/new readers, unknown variants, migration, and capability mismatch.

**Proposed experiments:** `compatibility-matrix`.

**Proposed locations:** `docs/CANON.md`, `docs/PROTOCOL.md`.

**Provenance:** [RFC 8949: CBOR](https://www.rfc-editor.org/rfc/rfc8949.html); [RFC 8785: JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785.html); [Protocol Buffers: serialization is not canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/); [Rust Reference: non_exhaustive](https://doc.rust-lang.org/reference/attributes/type_system.html); [Serde: container attributes](https://serde.rs/container-attrs.html).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## identity-recovery

**Define node, authority, and log recovery identities**

Separate logical NodeID, enrollment credential, writer incarnation, durable sequence, authority epoch, and resource key. Recovery from rollback requires authorized re-enrollment or an external monotonic anchor; a new field alone is insufficient.

**Priority rationale:** High-consequence rollback and clone safety; implementation can follow the local core but cannot be postponed past enrollment.

**Constraints:** Random incarnation values help uniqueness, not ordering or authenticated authority. A VM memory snapshot can clone RNG and process state. Do not lexicographically order arbitrary UUIDs as authority epochs.

**Acceptance criteria:** Document and test crash, reinstall, disk rollback, full VM clone, and old-authority replay separately.

**Proposed experiments:** `identity-rollback`.

**Proposed locations:** `docs/PROTOCOL.md`, `docs/formal/event-log.md`.

**Provenance:** [Linux openat2 manual](https://man7.org/linux/man-pages/man2/openat2.2.html); [Nomos: docs/formal/fencing-and-idempotency.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/fencing-and-idempotency.md); [The Update Framework specification](https://theupdateframework.github.io/specification/latest/); [SPIFFE concepts](https://spiffe.io/docs/latest/spiffe-about/spiffe-concepts/).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## log-retention

**Specify logical append-only history and storage-pressure behavior**

Make events immutable and append-only at the logical-history layer. Keep projections and acknowledgment cursors separately mutable. Never silently drop unacknowledged control events; define backpressure and explicit archive/retention policy.

**Priority rationale:** Essential for long-lived durability and disk-full safety; detailed storage policy follows the Phase 0 kernel.

**Constraints:** Strict physical retention grows without bound; no compaction or deletion is approved by this recommendation. Hash chains need externally retained checkpoints to expose rewritten prefixes.

**Acceptance criteria:** Same event coordinate with different payload is an integrity fault. Out-of-order uploads acknowledge only a durably contiguous prefix. Disk-full fails mutation admission safely.

**Proposed experiments:** `event-crash-replay`.

**Proposed locations:** `docs/formal/event-log.md`.

**Provenance:** [Nomos: docs/formal/fencing-and-idempotency.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/fencing-and-idempotency.md); [Nomos: docs/formal/event-log.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/event-log.md); [The Update Framework specification](https://theupdateframework.github.io/specification/latest/).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## substrate

**Narrow the privileged boundary and qualify Trace purity**

Give Trace only observation capabilities and define no mutation of managed-resource properties, excluding documented incidental effects. Use runtime-safe resource resolution, typed authorization, and a minimal privileged helper.

**Priority rationale:** Privileged path safety is foundational; OS implementation is later than pure-kernel semantics.

**Constraints:** Typed paths do not eliminate filesystem races. No shell interpolation does not prohibit a controlled argv invocation of an unavoidable package-manager CLI. Do not invent equivalent D-Bus APIs for every Linux subsystem.

**Acceptance criteria:** Mutation-capability calls fail compile-time access tests for Trace. Symlink and alias attacks cannot redirect an authorized operation outside its target policy.

**Proposed experiments:** `substrate-contract`.

**Proposed locations:** `docs/formal/invariants.md`, `docs/security-model.md`, `crates/ports/nomos-substrate`.

**Provenance:** [Linux openat2 manual](https://man7.org/linux/man-pages/man2/openat2.2.html); [Nomos: docs/formal/invariants.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/invariants.md).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## incremental

**Trial incremental assessment only against a full baseline**

First implement complete deterministic recomputation. Later trial tracked invalidation with keys including condition, evidence, capability, policy, and driver versions; compare every result with the baseline.

**Priority rationale:** Potential performance benefit; defer until the full recomputation path is correct and profiling shows need.

**Constraints:** Notifications are invalidation hints, not authoritative reality. Measured end-to-end benefit must exceed cache complexity.

**Acceptance criteria:** Incremental and full plans agree under randomized invalidations and dropped notifications.

**Proposed experiments:** `incremental-ablation`.

**Proposed locations:** `docs/adr`, `docs/formal/warp.md`.

**Provenance:** [Salsa incremental computation](https://salsa-rs.github.io/salsa/).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.


## plan-witness

**Explore independently checkable Plan witnesses**

Trial a small Plan checker that validates condition revision, evidence references, prerequisites, declared footprints, and policy bindings independently of the optimizing planner.

**Priority rationale:** Promising independent-checker architecture; needs a baseline planner and measured checker scope before adoption.

**Constraints:** A witness validates the encoded argument, not truth of untrusted observations. Call it proof-carrying only when the certificate language and checker theorem are explicit.

**Acceptance criteria:** A deliberately corrupted dependency or footprint is rejected by the checker. A simple planner and experimental planner both target the same checker.

**Proposed experiments:** `plan-witness`.

**Proposed locations:** `docs/formal/plan-witness.md`, `docs/adr`.

**Provenance:** [RFC 8949: CBOR](https://www.rfc-editor.org/rfc/rfc8949.html); [RFC 8785: JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785.html); [Protocol Buffers: serialization is not canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/); [Nomos: docs/formal/invariants.md](https://github.com/kmosoti/Nomos/blob/d4c11fa221c4cdc056bc85b6df84e127a6709d00/docs/formal/invariants.md); [Rust Reference: enumerations](https://doc.rust-lang.org/reference/items/enumerations.html).

**Evidence status:** Research synthesis, requiring implementation and validation. Sources inform this proposal; they do not prove Nomos implements it.
