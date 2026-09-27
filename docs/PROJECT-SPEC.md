# Nomos Project Specification

- Status: Draft v0.1
- Project: Nomos
- Organization: Moiric
- Primary implementation language: Rust
- Initial platform: Linux, with Debian as the reference distribution
- Purpose: Declarative host-state convergence, distributed execution, and fleet coordination

---

## 1. Definition

Nomos (Ancient Greek: νόμος, law, custom, established order) is a host-state convergence and distributed control system.

Nomos describes how machines should be, observes how they are, computes the difference, constructs a safe execution plan, applies the required changes, verifies their effects, and records the resulting transitions in an immutable append-only Event Log.

Its fundamental control relationship is:

```
DesiredState --compare--> ObservedState
Variance = DesiredState - ObservedState
Plan     = compile(Variance)
```

applied until `ObservedState ≈ DesiredState`.

Nomos is not intended to be "Salt rewritten in Rust." It uses Salt as one source of lessons, while deliberately incorporating ideas from reconciliation systems, workflow engines, schedulers, operating systems, databases, formal methods, security systems, and distributed-systems research.

The objective is a smaller and more principled model.

---

## 2. Core Architecture

```
                      NOMOS
              ordering and protocol
                       │
       ┌───────────────┴───────────────┐
       │                               │
      LOOM                            CELL
 coordinator / compiler          node executor
       │                               │
       │ compiles                      │ operates through
       ▼                               ▼
      WARP                         SUBSTRATE
 dependency graph              operating-system layer
```

### Nomos (`nomos`)

The overall project, protocol, specification, schemas, semantics, and shared model. Nomos defines:

- Canon semantics
- resource identity
- reconciliation semantics
- protocol semantics
- Event schemas
- Action lifecycle
- compatibility/versioning rules
- safety invariants

Nomos itself is not a daemon.

### Loom (`nomos-loom`)

The fleet coordinator. Loom:

- accepts Canon
- tracks Cells
- receives Traits
- resolves targets
- evaluates Variance
- invokes Warp compilation
- produces Plans
- dispatches Actions
- enforces concurrency constraints
- ingests Events
- maintains fleet materialized state
- manages execution leases
- coordinates multi-node convergence

Loom is an authority, not merely a message broker. Version 0 deliberately supports a single authoritative Loom. High availability and distributed consensus are deferred until there is evidence that multiple authoritative replicas are necessary.

### Cell (`nomos-cell`)

The node-resident executor. A Cell:

- discovers local Traits
- observes Substrate resources
- receives Plans
- validates authorization and freshness
- executes Actions
- verifies postconditions
- performs standalone convergence
- records Events locally
- survives temporary Loom disconnection
- forwards buffered Events after reconnection

A Cell is intentionally autonomous enough to operate locally without Loom:

```
nomos-cell trace --canon local.yaml
nomos-cell enforce --canon local.yaml
```

Loom adds fleet coordination rather than creating a dependency for basic reconciliation.

### Warp (`nomos-warp`)

Warp is the dependency and execution-graph engine. It takes declarative resources and Variance and produces a Directed Acyclic Graph (DAG) of executable Actions. Responsibilities:

- dependency resolution
- graph construction
- cycle detection
- topological ordering
- conditional activation
- conflict detection
- concurrency frontier calculation
- failure propagation
- graph validation

`petgraph` is an appropriate initial Rust implementation. Its topological sort detects cycles and runs in O(|V|+|E|).

### Substrate (`nomos-substrate`)

Substrate is the operating-system boundary. It converts strongly typed Nomos operations into concrete Linux interactions involving files, directories, users/groups, packages, systemd units, sysctl, processes, cgroup v2, `/proc`, `/sys`, networking interfaces, permissions and ownership.

Substrate should prefer native APIs over shell execution:

```
Nomos → SystemdResource → zbus → systemd D-Bus API → PID 1
```

rather than:

```
Nomos → "systemctl restart nginx" → shell → systemctl → D-Bus → PID 1
```

systemd exposes units, jobs, state and operations through its D-Bus object model, making that interface a considerably stronger substrate contract than parsing CLI output.

---

## 3. Public Vocabulary

| Term | Definition |
|---|---|
| Canon | Declarative description of desired state |
| Trait | Typed observation about a Cell or its environment |
| Cipher | Reference to protected secret material |
| Variance | Difference between desired and observed state |
| Trace | Non-mutating evaluation of Variance |
| Enforce | Reconciliation of observed state toward Canon |
| Event | Immutable record of a meaningful transition |
| Event Log | Append-only ordered collection of Events |

Alternative names are intentionally discarded. There is no simultaneous "Pattern/Canon", "Flaw/Variance", "Audit/Trace", or "Weave/Enforce" vocabulary. One concept gets one name.

---

## 4. Traits

A Trait is not necessarily immutable. `architecture = x86_64` is extremely stable; `kernel`, `ip_address`, `package_version`, `memory_available` can change. Therefore:

```
Trait = Value + Provenance + ObservationTime + Stability
```

```rust
struct Trait<T> {
    key: TraitKey,
    value: T,
    source: TraitSource,
    observed_at: Timestamp,
    stability: Stability,
}
```

Stability classes: `Identity`, `Stable`, `Dynamic`, `Ephemeral`.

Targeting should primarily use Identity and Stable Traits unless a Canon explicitly permits otherwise. This prevents fleet membership from unpredictably changing.

---

## 5. Canon

Canon expresses desired state.

```yaml
apiVersion: nomos/v1alpha1
name: telemetry-node

resources:
  - id: nomos-user
    kind: system_user
    spec:
      name: nomos
      shell: /usr/sbin/nologin

  - id: config-directory
    kind: directory
    requires:
      - nomos-user
    spec:
      path: /etc/nomos
      owner: nomos
      group: nomos
      mode: "0750"

  - id: cell-config
    kind: file
    requires:
      - config-directory
    spec:
      path: /etc/nomos/cell.yaml
      owner: nomos
      group: nomos
      mode: "0640"
      content:
        source: artifact
        digest: sha256:...

  - id: cell-service
    kind: systemd_unit
    requires:
      - cell-config
    on_change:
      - cell-config
    spec:
      name: nomos-cell.service
      enabled: true
      state: running
```

Canon should be versioned, typed, deterministic, declarative, statically validated, and independent of execution order where possible.

Canon should not initially contain an embedded general-purpose programming language. Conditional expressions should remain deliberately restricted.

---

## 6. Canon Compilation

Human-readable Canon is not the execution representation.

```
YAML → Parser → Typed AST → Validation → Canonical IR (→ hash) → Warp → Plan DAG
```

The canonical IR must serialize deterministically, allowing:

```
CanonID = H(CanonicalEncoding(Canon))
```

Content-derived identity borrows the useful part of Nix's content-addressing model. Nomos does not need to reproduce the Nix store; the transferable lesson is deterministic identity.

---

## 7. Cipher

Cipher means specifically: *a protected reference whose plaintext value is resolved only at the boundary where it is required.*

```yaml
password:
  cipher: vault://production/database/password
```

Cipher should not become a synonym for all node-specific configuration. Ordinary scoped variables remain ordinary Canon parameters.

Cipher plaintext must never appear in Plan hashes, Event payloads, Trace output, error messages, debug logs, or telemetry.

```rust
trait CipherProvider {
    async fn resolve(&self, reference: &CipherRef) -> Result<SecretBytes>;
}
```

Possible providers: local protected file, environment-backed development provider, HashiCorp Vault, cloud secret stores, external enterprise systems.

Nomos should avoid becoming a secrets database.

---

## 8. Reconciliation Model

The central abstraction is a level-triggered reconciliation loop (as in Kubernetes controllers, without adopting Kubernetes's object model).

For resource `r`:

```
O_r = observe(r)
V_r = diff(D_r, O_r)
if V_r = ∅: nothing happens
else:
  A_r = plan(V_r)
  apply(A_r)
  verify(D_r, O'_r)
```

A successful Action means *the intended postcondition was observed* — not merely *a command returned exit code zero*. This distinction is foundational.

---

## 9. Substrate Resource Contract

```rust
trait ResourceDriver {
    type Spec;
    type Observation;
    type Variance;

    async fn observe(&self, spec: &Self::Spec) -> Result<Self::Observation>;
    fn diff(&self, desired: &Self::Spec, observed: &Self::Observation) -> Result<Self::Variance>;
    fn plan(&self, variance: &Self::Variance) -> Result<Vec<Action>>;
    async fn apply(&self, action: &Action) -> Result<ActionEffect>;
    async fn verify(&self, desired: &Self::Spec) -> Result<Verification>;
}
```

```
              Resource contract
                     │
       ┌─────────────┼──────────────┐
       ▼             ▼              ▼
 Linux backend    Mock backend   Future BSD backend
```

The mock implementation is not optional scaffolding. It is a first-class backend enabling deterministic testing of reconciliation.

---

## 10. Initial Substrate Resources

Initial: `file`, `directory`, `system_user`, `package`, `systemd_unit`, `sysctl`.

Later: `mount`, `network`, `cgroup`, `kernel_module`, `timer`, `socket`, container/workload.

Do not begin by reproducing Salt's execution-module surface. Prove the resource contract first.

---

## 11. Atomic Filesystem Operations

Avoid `open → truncate → write`. Preferred:

```
write temporary file → fsync → set metadata → atomic rename → fsync parent directory → verify
```

Observed content should ordinarily be compared using hashes.

---

## 12. systemd Integration

systemd is the authoritative service manager on the initial platform. Substrate communicates over D-Bus (`org.freedesktop.systemd1`) for `GetUnit`, `StartUnit`, `StopUnit`, `RestartUnit`, `ReloadUnit`, unit properties and job state — rather than scraping `systemctl`.

---

## 13. cgroup v2

Nomos should be cgroup-v2-native. Bounded Actions may run as:

```
Action → transient systemd scope → delegated cgroup (CPU / memory / I/O / process limits)
```

Nomos cooperates with systemd (`Delegate=`) rather than fighting PID 1 for ownership of the cgroup hierarchy.

---

## 14. Warp Graph Model

`G = (V, E)` where V = Actions and E = dependency constraints. Initial edge semantics:

- `requires` — B may execute only after A succeeds.
- `after` — ordering constraint without implying B exists because A changed.
- `on_change` — B becomes actionable only if A produced a state change.

`watch` is not part of v0 unless it provides semantics these primitives cannot express.

---

## 15. Warp Algorithms

- **Cycle detection** — cyclic `requires` must fail compilation (SCC or graph-library cycle reporting).
- **Topological ordering** — O(V+E).
- **Execution frontier** — `Ready = { v | dependencies(v) = Succeeded }`; concurrency then restricted by policy.
- **Conflict keys** — e.g. `package-manager:dpkg`, `file:/etc/hosts`, `systemd:nginx.service`. Nodes sharing an exclusive key cannot execute simultaneously; graph independence is not operational independence.

---

## 16. Plan

An internal compiled artifact:

```
Plan
├── ID
├── Canon ID
├── target snapshot
├── Actions
├── dependency edges
├── constraints
├── concurrency policy
├── creation generation
└── expiry / lease information
```

Plan identity derives from canonical content where practical. Plans are immutable after acceptance; changing a Plan produces a new Plan.

---

## 17. Optimistic Planning

Following Nomad: evaluation → plan → validation → execution. Loom compiles against a node-state generation (e.g. 417); the Plan carries `expected_generation = 417`. Before applying, the authority verifies assumptions still hold; otherwise reject stale plan → observe again → recompile.

---

## 18. Action

```
Action
├── ID
├── type
├── target
├── inputs
├── preconditions
├── operation
├── postconditions
├── timeout
├── retry policy
├── conflict keys
└── idempotency key
```

Lifecycle: `Prepared → Dispatched → Accepted → Running → Verifying → Succeeded`, with terminal alternatives `Failed`, `TimedOut`, `Cancelled`, `Rejected`.

A timeout is not equivalent to failure. An uncertain outcome must be represented as such and state re-observed.

---

## 19. Idempotency

At-least-once delivery plus idempotent execution/deduplication. Each Action carries an idempotency key; the Cell persists accepted keys and returns stored results for retransmissions. Nomos does not claim exactly-once distributed execution.

---

## 20. Fencing

Plans receive monotonically increasing generations/fencing tokens. If a Cell has accepted generation 52, generation 51 is rejected.

---

## 21. Loom Targeting

```
nomos-loom trace --target 'traits.os == "debian" && traits.tier == "edge"' --canon telemetry-stack
```

`Target = { n ∈ Nodes | Predicate(Traits(n)) }`. Start with an in-memory indexed scan; consider roaring bitmaps (`Trait → value → bitmap<NodeID>`) only when evidence demands.

---

## 22. Loom Scheduling

```yaml
execution:
  parallelism: 8
  max_unavailable: 1
  failure_domain:
    trait: rack
    max_unavailable: 1
```

`Runnable = Ready ∩ PolicyAllowed ∩ CapacityAvailable`.

v0 needs: bounded concurrency, dependency readiness, per-node exclusivity, conflict keys, max-unavailable budgets, cancellation, backpressure. Weighted fair queuing, work stealing, critical-path prioritization, adaptive concurrency and topology-aware scheduling are later research.

---

## 23. Cell Architecture

```
                CELL
                 │
     ┌───────────┼────────────┐
     ▼           ▼            ▼
 protocol     observer     event spool
     │           │
     ▼           ▼
 validator     Traits
     │
     ▼
 executor
     │
     ▼
 Substrate
```

The Cell owns execution semantics and establishes whether the local operation achieved its postcondition.

---

## 24. Cell Offline Behavior

A disconnected Cell may observe state, perform local Trace, execute explicitly local Canon, persist Events, and complete already-authorized bounded work where policy permits. It must not accept imaginary new fleet authority; central Plans carry leases/expiry. On reconnection: spool → batched upload → Loom acknowledgement → local checkpoint advances.

---

## 25. Event

```rust
struct Event {
    id: EventId,
    writer: WriterId,
    writer_sequence: u64,

    node_id: Option<NodeId>,
    canon_id: Option<CanonId>,
    plan_id: Option<PlanId>,
    action_id: Option<ActionId>,

    kind: EventKind,

    occurred_at: Timestamp,
    causation_id: Option<EventId>,
    correlation_id: Option<CorrelationId>,

    payload: EventPayload,
}
```

Events are immutable after append.

---

## 26. Event Log

The canonical term (not Journal, not Ledger): *an append-only sequence of immutable operational Events.* It supports recovery, audit, materialized fleet state, debugging, Plan history and external integration. Conceptually `State_t = fold(Event_0, …, Event_t)`; implementations may maintain materialized state.

---

## 27. Event Ordering

Wall-clock timestamps do not establish causality. Each writer maintains `writer_sequence`; Loom assigns its own ingestion position. Cross-node causality is explicit via `causation_id`, `correlation_id`, `plan_id`, `action_id`. Hybrid Logical Clocks are a later candidate; vector clocks are not adopted without a concrete requirement.

---

## 28. Event Log Integrity

v0 guarantees: no update operation, no in-place mutation, monotonic writer sequence, transactional append. A later integrity mode may add a hash chain `H_n = H(H_{n-1} || Event_n)`, signed checkpoints or Merkle structures — only then would "Ledger" be defensible.

---

## 29. Persistence

The storage API remains abstract. `redb` is a strong v0 candidate. Possible layout: `events`, `event_by_id`, `actions`, `action_idempotency`, `traits`, `materialized_nodes`, `plans`, `canons`, `metadata`.

An ablation compares redb, SQLite and an LMDB-family option on append throughput, crash recovery, fsync behavior, query ergonomics, binary size, dependency burden, corruption recovery and operational transparency before freezing the choice.

---

## 30. Control Protocol

Protocol and transport are separate abstractions. Initial: Protocol Buffers schema, tonic-compatible service model, HTTP/2 + TLS.

The Cell initiates its Loom connection (works across NAT, firewalls, cloud and enterprise networks). The stream carries registration, Traits, heartbeats, Plan assignments, Action status, Events and acknowledgements. QUIC or another transport can be added without replacing the domain schema.

---

## 31. Backpressure

Each Cell advertises capacity (e.g. `max_parallel_actions = 4`, `queue_capacity = 32`). Loom maintains bounded queues; producers slow down rather than memory growing without bound. Bounded channels and explicit admission control are standard. Tokio cancellation is cooperative, reinforcing explicit Action lifecycle and timeout semantics.

---

## 32. Identity

Nomos uses `NodeID`, `CanonID`, `PlanID`, `ActionID`, `EventID`. Cell identity should eventually be cryptographically verifiable. Borrow SPIFFE principles — workload identity, trust domains, short-lived certificates, mutual authentication, automatic rotation — without requiring SPIRE.

---

## 33. Security Boundary

The Cell is effectively a remote root-management system. It should be privilege-separated:

```
        network
           │
           ▼
   unprivileged Cell
           │
     typed local IPC
           │
           ▼
  privileged executor
           │
           ▼
       Substrate
```

The privileged side accepts typed operations only, over a root-owned Unix-domain socket with peer-credential validation.

---

## 34. Shell Escape Hatch

Arbitrary shell execution is not a core primitive. If introduced it is explicitly opaque (`kind: exec` with `command`, `precondition`, `postcondition`) and policy can prohibit it entirely.

---

## 35. Linux Hardening

Use systemd hardening where compatible: `NoNewPrivileges`, `ProtectSystem`, `ProtectHome`, `PrivateTmp`, `RestrictSUIDSGID`, capability bounding, seccomp, cgroup limits and resource controls. Aggressive for Loom; tailored for Cell and the privileged helper.

---

## 36. Polkit

Polkit may serve local human → CLI → privileged local operation, but remote Loom-to-Cell authorization belongs in Nomos's authenticated protocol and policy model.

---

## 37. Trace

Trace guarantees non-mutation and uses exactly the same parse/observe/diff/compile pipeline as Enforce; only execution is omitted.

```
FILE /etc/example.conf
  content:
    observed: sha256:abc
    desired:  sha256:def
    action: replace

SYSTEMD example.service
  enabled:
    observed: false
    desired:  true
  state:
    observed: stopped
    desired:  running
```

---

## 38. Enforce

```
Canon → Observe → Variance → Warp → Plan → Actions → Verify → Observe again
```

Terminates when `Variance = ∅` or a defined failure condition is reached. Reconciliation loops are bounded; oscillation is detected as non-convergence.

---

## 39. Fixed-Point Property

If `Enforce(Canon, S) = S'` with `Variance(Canon, S') = ∅`, then `Enforce(Canon, S') = S'` with no mutating Actions. This is a primary property-based test.

---

## 40. Failure Semantics

Process crashes, host crashes, network partitions, message duplication, loss and delay, partial execution, Loom restart, Cell restart, clock skew and stale Plans are normal operating conditions. Each outcome must be representable.

---

## 41. Availability Model

v0: one Loom, many Cells. Losing Loom stops new fleet coordination but does not corrupt Cell state; Cells remain locally inspectable.

---

## 42. Future Loom High Availability

If required, multiple Loom replicas use consensus (e.g. OpenRaft) for Canon registrations, Plan generations, ownership/leases and critical control metadata — not for Trait samples, debug logs, metrics or telemetry.

---

## 43. CAP Position

Consistency semantics are chosen per subsystem. Authoritative Loom state prefers refusing conflicting control decisions over split authority. Cells retain local availability for observation, Trace, cached state and explicitly authorized local convergence.

---

## 44. Event Log vs Telemetry

Nomos Events are control-plane records, not an observability pipeline. High-volume telemetry belongs in FabricO11y. Nomos exposes an integration adapter so FabricO11y can ingest Action started/completed, Canon changed, Variance detected, Cell disconnected, Plan rejected, convergence failed.

---

## 45. Testing Architecture

```
Unit → Property → Graph → Concurrency → Crash → Integration → Distributed failure
```

---

## 46. Property-Based Testing

- **Idempotence:** `enforce(enforce(S)) = enforce(S)`
- **Trace purity:** `SubstrateBefore = SubstrateAfter` for every Trace
- **Graph validity:** every dependency precedes its dependent Action
- **Determinism:** identical Canon, Traits and ObservedState produce identical logical Plans
- **Secret non-disclosure:** no Cipher plaintext in serialized Events
- **Event monotonicity:** `sequence_{n+1} > sequence_n` per writer
- **Stale fencing:** an Action below the latest accepted generation never executes

---

## 47. Concurrency Testing

Use the Rust `loom` crate for deterministic interleaving exploration, aliased to avoid the naming collision:

```toml
loom-model = { package = "loom", version = "..." }
```

---

## 48. Formal Specification

Maintain a small TLA+ model of the control protocol (not Linux) in `formal/tla/`.

State variables: `plans`, `actions`, `nodes`, `accepted_generation`, `action_state`, `event_sequence`, `leases`.

Invariants:

- **Dependency safety:** `Running(a) ⇒ ∀ d ∈ Requires(a): Succeeded(d)`
- **Fencing safety:** `Execute(a) ⇒ Generation(a) ≥ AcceptedGeneration(node)`
- **Verified success:** no `Succeeded` without successful verification
- **Single active authority:** a node never simultaneously accepts conflicting Plan generations
- **Bounded disruption:** `Unavailable(FailureDomain) ≤ k`
- **Event monotonicity:** each writer sequence strictly increases

---

## 49. Crash Testing

Force termination at every persistent transition: after accepting an Action, during an operation, after the operation before Event append, after Event append before acknowledgement, Loom after dispatch, Loom after receiving a result before persistence. Recovery must be deterministic.

---

## 50. Workspace

See `docs/ARCHITECTURE.md` for the hexagonal mapping of the workspace. Crates beyond the four components (`nomos-canon`, `nomos-store`, `nomos-protocol`, port and adapter crates) are implementation boundaries, not user-facing concepts.

---

## 51. CLI

Loom:

```
nomos-loom compile --canon base-system.yaml
nomos-loom trace   --target 'traits.os == "debian"'   --canon hardened-host.yaml
nomos-loom enforce --target 'traits.tier == "edge"'   --canon telemetry-stack.yaml
```

Cell:

```
nomos-cell traits
nomos-cell trace   --canon /etc/nomos/local.yaml
nomos-cell enforce --canon /etc/nomos/local.yaml
nomos-cell events
```

---

## 52. Design Lineage

| System / concept | Lesson Nomos takes |
|---|---|
| Salt | broad host-state management, remote execution, targeting |
| Kubernetes | level-triggered reconciliation and independently understandable controllers |
| Nomad | desired/emergent-state evaluation, optimistic planning, validation before commitment |
| Nix | deterministic declarative inputs and content-derived identity |
| Temporal | durable execution state and explicit recovery after process/network failure |
| systemd | typed native service control, cgroup ownership, Linux-native resource lifecycle |
| SPIFFE | workload identity and short-lived cryptographic credentials |
| Raft | consensus only for genuinely replicated authoritative state |
| Event sourcing | immutable transitions plus derived materialized state |
| TLA+ | verify control-protocol safety properties independently of implementation |
| Rust loom | systematically explore local concurrency interleavings |

Nomos takes these properties selectively and does not absorb their architectures.

---

## 53. Explicit Non-Goals for v0

Not a Kubernetes replacement, container orchestrator, secrets manager, telemetry database, service mesh, package ecosystem, distributed filesystem, general workflow engine, shell-command broadcasting tool, multi-region consensus platform, new Linux init system, or universal cross-platform abstraction.

Reference problem: *reliably describe, inspect, change and verify Linux host state, locally and across a fleet.*

---

## 54. Initial Research Ablations

- **Persistence:** redb vs SQLite vs LMDB-family
- **Canon syntax:** strict YAML vs TOML (same typed IR)
- **Transport:** validate tonic/HTTP2 before investigating QUIC
- **File observation:** metadata + hash vs full read vs notification-assisted caching
- **Fleet targeting:** predicate scans vs bitmap indexing at large synthetic fleet sizes
- **Reconciliation scheduling:** sequential vs bounded DAG parallelism vs conflict-aware DAG parallelism

---

## 55. Development Phases

- **Phase 0 — Semantics.** No networking. `nomos-core`, `nomos-canon`, `nomos-substrate-mock`, `nomos-warp`. Prove Canon → Observation → Variance → Plan deterministically.
- **Phase 1 — Masterless Cell.** Linux Substrate on Debian: file, directory, system_user, systemd_unit, sysctl, package. Commands: traits, trace, enforce. A Debian machine converges locally from any supported starting state.
- **Phase 2 — Event Log and Recovery.** Every Action transition durable; kill the Cell at every lifecycle boundary; prove deterministic recovery.
- **Phase 3 — Loom.** One coordinator: registration, Traits, targeting, Plan dispatch, Action results, Event ingestion. No consensus.
- **Phase 4 — Fleet Safety.** Bounded parallelism, max-unavailable, failure domains, fencing, leases, backpressure, stale-plan rejection.
- **Phase 5 — Privilege Separation.** Local IPC, peer authentication, typed privileged API, systemd hardening, seccomp.
- **Phase 6 — External Integrations.** Cipher providers, FabricO11y, CI/CD, change management, identity providers — as adapters, not core model.
- **Phase 7 — High Availability.** Only if required: multiple Loom replicas, leader election, replicated authoritative state.

---

## 56. First End-to-End Demonstration

Use Nomos to deploy FabricO11y Cell/Agent software onto Debian hosts (system user, directories, binary artifact, configuration, systemd service, resource limits, enablement, health):

```
fresh Debian VM → nomos trace → Variance shown → nomos enforce → FabricO11y installed
→ service running → postcondition verified → Event Log complete → nomos enforce again → 0 mutating Actions
```

Then break the host (delete config, stop service, change permissions, alter sysctl): Trace must identify the Variance and Enforce must restore Canon.

---

## 57. Failure Demonstration

During convergence: kill Cell, restart Cell, disconnect network, duplicate Plan delivery, delay old Action, restart Loom.

Expected: no duplicate destructive mutation, no stale Action execution, no false success, no corrupted Event Log, eventual convergence after recovery.

---

## 58. Core Safety Invariants

- **N1** — Trace does not mutate Substrate.
- **N2** — Successful Enforce converges supported resources toward Canon.
- **N3** — Re-enforcing converged Canon performs no required mutation.
- **N4** — No Action executes before its hard dependencies succeed.
- **N5** — A stale fenced Plan cannot execute.
- **N6** — "Succeeded" requires verified postconditions.
- **N7** — Event records are never mutated after append.
- **N8** — Cipher plaintext never enters persistent operational records.
- **N9** — Failure-budget constraints are never intentionally exceeded.
- **N10** — Unknown execution outcome is represented as unknown, not silently converted to success or failure.
- **N11** — Losing Loom does not invalidate already-observed Cell state.
- **N12** — Canon compilation is deterministic for identical inputs.

These invariants are more important than individual implementation technologies.

---

## 59. Architectural Thesis

`Canon + Traits + Observation` produce `Variance`; Variance produces `Warp`; Warp produces `Actions`; Actions operate through `Substrate`; every meaningful transition produces `Events`.

```
                   CANON
             desired state
                   │
                   ▼
                 LOOM
          targeting / planning
                   │
                   ▼
                 WARP
              Action DAG
                   │
                   ▼
                 CELL
          validated execution
                   │
                   ▼
              SUBSTRATE
               Linux
                   │
                   ▼
              OBSERVATION
                   │
             ┌─────┴─────┐
             ▼           ▼
          TRAITS       VARIANCE
             │           │
             └─────┬─────┘
                   ▼
                 LOOM

Every transition → EVENT → EVENT LOG
```

> Observe reality, compare it with Canon, derive the smallest valid change, apply that change under explicit safety constraints, verify reality again, and preserve what happened as immutable history.

---

## 60. Relationship to Moiric

```
MOIRIC
├── FabricO11y — Observe distributed systems.        (What is happening?)
├── Nomos      — Reconcile and control them.          (What should be happening, and how do we safely get there?)
└── Metron     — Bound and govern computation.        (Within what boundaries may computation happen?)
```

They remain independently useful systems with narrow integration surfaces.

---

## 61. Definition of v0 Success

On real Debian hosts, Nomos v0 can:

1. Parse and deterministically compile Canon.
2. Discover and expose typed Traits.
3. Observe supported Linux resources without mutation.
4. Compute precise Variance.
5. Compile valid Action DAGs and reject cycles.
6. Produce a trustworthy Trace.
7. Enforce Canon locally through Cell.
8. Verify every successful state transition.
9. Produce no mutating Actions after convergence.
10. Persist an append-only Event Log.
11. Recover coherently after Cell termination.
12. Connect Cells to a single Loom.
13. Select fleets using Trait predicates.
14. Dispatch and deduplicate Actions reliably.
15. Reject stale Plans using fencing.
16. Enforce concurrency and disruption budgets.
17. Recover from temporary network disconnection.
18. Keep Cipher plaintext out of persistent records.
19. Export Nomos operational Events to FabricO11y.
20. Demonstrate the important protocol invariants through model/property/failure testing.

Nomos should earn its complexity one invariant at a time.
