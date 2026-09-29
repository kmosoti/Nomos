# Nomos Project Specification

| | |
| --- | --- |
| Status | Draft v0.1 |
| Project | Nomos |
| Organization | Moiric |
| Language | Rust |
| Initial platform | Linux, with Debian as the reference distribution |
| Purpose | Declarative host-state convergence, distributed execution, and fleet coordination |

## 1. Definition

Nomos (Ancient Greek νόμος: law, custom, established order) is a host-state convergence and distributed control system.

Nomos describes how machines should be as Conditions, observes how they are, assesses each Condition against the evidence, builds a safe execution plan for the known mismatches, applies the required changes, verifies their effects, and records every transition in an immutable, append-only Event Log.

Its fundamental control relationship:

$$
\begin{aligned}
\mathit{Assessment}_r &= \mathrm{assess}(\mathit{Condition}_r,\ \mathit{Observation}_r) \in \{\mathrm{Satisfied},\ \mathrm{Variance},\ \mathrm{Indeterminate}\} \\
\mathit{Plan} &= P(\mathit{Canon},\ \{\mathit{Assessment}_r\},\ \mathit{Obligations},\ \mathit{Capabilities},\ \mathit{Policy})
\end{aligned}
$$

applied until the Canon is converged: every Condition Satisfied, no Obligation pending, every relevant effect Settled (§8). An Indeterminate Assessment contributes nothing to the Plan. Unknown evidence does not imply noncompliance.

Nomos is not Salt rewritten in Rust. Salt is one source of lessons. So are reconciliation systems, workflow engines, schedulers, operating systems, databases, formal methods, security systems, and distributed-systems research.

The goal is a smaller, more principled model.

## 2. Core Architecture

```mermaid
flowchart TB
    N["<b>NOMOS</b><br/>ordering and protocol"]
    L["<b>LOOM</b><br/>coordinator / compiler"]
    C["<b>CELL</b><br/>node executor"]
    W["<b>WARP</b><br/>dependency graph<br/><i>Loom compiles through it</i>"]
    S["<b>SUBSTRATE</b><br/>operating-system layer<br/><i>Cell operates through it</i>"]
    N --> L
    N --> C
    L --> W
    C --> S
```

### Nomos (`nomos`)

The project, protocol, specification, schemas, semantics, and shared model. Nomos defines:

- Canon semantics
- resource identity
- reconciliation semantics
- protocol semantics
- Event schemas
- the Action lifecycle
- compatibility and versioning rules
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
- maintains materialized fleet state
- manages execution leases
- coordinates multi-node convergence

Loom is an authority, not a message broker. Version 0 supports exactly one authoritative Loom. High availability and consensus wait until there is evidence that multiple authoritative replicas are needed.

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

A Cell is autonomous enough to work without Loom:

```sh
nomos-cell trace --canon artifacts/local
nomos-cell enforce --canon artifacts/local
```

Loom adds fleet coordination. It is not a dependency for basic reconciliation.

### Warp (`nomos-warp`)

The dependency and execution-graph engine. Warp takes the Canon's resources and relationships, their Assessments, pending Obligations, capabilities, and policy, and produces a directed acyclic graph (DAG) of executable Actions ([formal/reconciliation.md](formal/reconciliation.md#model)). It handles:

- dependency resolution
- graph construction
- cycle detection
- topological ordering
- conditional activation
- conflict detection
- concurrency frontier calculation
- failure propagation
- graph validation

`petgraph` is a suitable first implementation. Its topological sort detects cycles and runs in $O(|V| + |E|)$.

### Substrate (`nomos-substrate`)

The operating-system boundary. Substrate turns strongly typed Nomos operations into concrete Linux interactions with files, directories, users and groups, packages, systemd units, sysctl, processes, cgroup v2, `/proc`, `/sys`, network interfaces, permissions, and ownership.

Substrate prefers native APIs to shell execution. This:

```mermaid
flowchart LR
    A["Nomos"] --> B["SystemdResource"] --> C["zbus"] --> D["systemd D-Bus API"] --> E["PID 1"]
```

not this:

```mermaid
flowchart LR
    A["Nomos"] --> B["systemctl restart nginx"] --> C["shell"] --> D["systemctl"] --> E["D-Bus"] --> F["PID 1"]
```

systemd exposes units, jobs, state, and operations through its D-Bus object model. That is a far stronger contract than parsing output written for humans.

## 3. Public Vocabulary

| Term | Definition |
| --- | --- |
| Canon | Compiled desired intent: Conditions and their relationships, authored in typed Rust and consumed as an inert, normalized artifact (§5–§6) |
| Condition | A proposition about one resource that reality is expected to satisfy |
| Observation | Evidence obtained from Substrate about one resource: what was seen, by which source, when, and whether collection succeeded |
| Assessment | The interpretation of a Condition against an Observation: Satisfied, Variance, or Indeterminate |
| Variance | A known mismatch: sufficient evidence that a Condition does not hold |
| Indeterminate | Insufficient or failed evidence, with its reason. Never a mismatch |
| Obligation | A follow-up effect a completed change requires and no Condition can observe, held durably until discharged |
| Settled | The condition of an effect whose outcome is known and which can cause no further change |
| Action | One bounded attempt to cause or verify a transition of one resource (§18) |
| Plan | An immutable compiled set of Actions, their dependency edges, and their constraints (§16) |
| Trait | Typed observation about a Cell or its environment |
| Cipher | Reference to protected secret material |
| Trace | Non-mutating evaluation of Assessments |
| Enforce | Reconciliation of reality toward Canon: plan from Variance and Obligations, apply, verify, observe again |
| Event | Immutable record of a meaningful transition |
| Event Log | Append-only, ordered collection of Events |

Alternative names are discarded on purpose. There is no "Pattern/Canon", "Flaw/Variance", "Audit/Trace", or "Weave/Enforce", and no "Requirement/Condition", "Check/Assessment", "Unknown/Indeterminate", or "Quiesced/Settled". One concept gets one name. Condition, Observation, Assessment, Indeterminate, Obligation, and Settled were added by [ADR 0005](adr/0005-assessment-vocabulary.md). Action and Plan were already defined in §16 and §18 and are listed so the one-name rule covers them too.

### The Assessment Algebra

A Condition and an Observation produce exactly one Assessment:

```mermaid
flowchart TB
    C["Condition<br/>what reality should satisfy"] --> A{"Assessment"}
    O["Observation<br/>evidence from Substrate"] --> A
    A --> S(["Satisfied"])
    A --> V(["Variance<br/>known mismatch"])
    A --> I(["Indeterminate<br/>insufficient or failed evidence"])
```

Conceptually:

```rust
enum Assessment<V, E> {
    Satisfied,
    Variance(V),
    Indeterminate(E),
}
```

`V` describes the mismatch and `E` the reason the evidence was insufficient. The Rust types are not implemented yet; milestone `03-assessment-kernel` implements them for the first resource family.

A failed observation is never a Variance. A denied read, a stale reading, or two readings that contradict each other assess as Indeterminate. Casting such evidence to Variance would plan a mutation from ignorance; casting it to Satisfied would hide drift behind a permission error. **Unknown evidence does not imply noncompliance** (proposed invariant, §58). Assessments are kept per Condition: an Indeterminate Assessment of one resource does not erase the Variance of another.

## 4. Traits

A Trait is not necessarily immutable. `architecture = x86_64` is very stable. `kernel`, `ip_address`, `package_version`, and `memory_available` are not. So:

$$
\mathit{Trait} = \mathit{Value} + \mathit{Provenance} + \mathit{ObservationTime} + \mathit{Stability}
$$

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

Targeting uses Identity and Stable Traits unless a Canon explicitly allows otherwise. Otherwise fleet membership would change every time someone targets a value that moves every five seconds.

## 5. Canon

Canon expresses desired intent as Conditions on resources and the relationships between them. It is authored in typed Rust: a crate that depends on `nomos-canon`, builds a `Canon` value from typed resource specifications, and emits the inert artifact of §6 ([ADR 0004](adr/0004-rust-typed-canon.md)). The shape below is illustrative. The authoring API, including whether it offers builders, a macro layer, or both, is not designed yet.

```rust
use nomos_canon::prelude::*;

pub fn telemetry_node() -> Canon {
    let nomos_user = SystemUser::named("nomos").shell("/usr/sbin/nologin");

    let config_directory = Directory::at("/etc/nomos")
        .owner(&nomos_user)
        .mode(0o750)
        .requires(&nomos_user);

    let cell_config = File::present("/etc/nomos/cell.conf", Content::artifact(CELL_CONFIG_DIGEST))
        .owner(&nomos_user)
        .mode(0o640)
        .requires(&config_directory);

    let cell_service = SystemdUnit::named("nomos-cell.service")
        .enabled(true)
        .state(UnitState::Running)
        .requires(&cell_config)
        .on_change(&cell_config);

    Canon::named("telemetry-node").resources([nomos_user, config_directory, cell_config, cell_service])
}
```

Canon is versioned, typed, deterministic, declarative, statically validated, and independent of execution order where possible. The resource specifications are sum types: a file is `Absent` or `Present` with its requirements, and the absent-with-contents contradiction has no representation.

Rust is the authoring surface, not the runtime. The author's crate runs once, in an isolated build job, and what it emits is inert data. Loom and Cell never compile or run Rust from a Canon; they accept only the artifact. No programming language runs on a managed host, which is the property the earlier YAML design existed to protect. Configuration systems have repeatedly shown humanity's talent for turning templating engines into badly documented programming languages. Nomos declines a second time. How a Condition may depend on a Trait is an open decision (§62), and so is how Conditions compose beyond a set. Whatever answers are chosen must survive the same rule: what reaches Loom and Cell is data, never a closure or a template.

## 6. Canon Compilation

The author's Rust is not what gets executed. It is compiled, once, into an inert artifact, and the artifact is what Loom and Cell consume.

```mermaid
flowchart TB
    SRC[/"typed Rust Canon source"/] --> DC["validated domain construction"]
    DC --> SV["semantic validation"]
    SV --> NM["normalization"]
    NM --> IR["inert Canonical IR"]
    IR --> ART[("Canon artifact")]
    ART --> LC["Loom · Cell<br/>validated decode"]
    LC --> W["Warp"] --> D(["Plan DAG"])
    IR --> H(["CanonID"])
```

Every stage above the artifact runs in the author's build job. Every stage below it runs in Nomos, and none of them runs Rust from the Canon.

The build job is ordinary code execution and is treated as such: no network, no host-management credentials, declared inputs only, and a provenance record (toolchain, lockfile digest, `nomos-canon` version, input digests) written beside the IR and excluded from `CanonID`. Two builds with the same declared inputs produce the same IR bytes. That is tested, not assumed. The consumer decodes the IR into untrusted data-transfer objects and converts them, fallibly, into the validated domain types through the same validator the authoring API uses. An IR that fails that conversion produces no Plan.

The IR is encoded as deterministic Concise Binary Object Representation (CBOR), in files with the extension `.cbor` ([ADR 0011](adr/0011-canon-artifact-encoding.md), [canon-ir.md](formal/canon-ir.md)). A container around the artifact, the content hash profile behind $H$ below, and any artifact signing scheme are open (§62).

The canonical intermediate representation (IR) serializes deterministically, which gives:

$$
\mathit{CanonID} = H(\mathit{CanonicalEncoding}(\mathit{Canon}))
$$

Content-derived identity borrows the useful part of Nix: identical canonical content produces identical identity. Nomos does not need the Nix store. It needs deterministic identity.

## 7. Cipher

A Cipher is a protected reference whose plaintext is resolved only at the boundary where it is needed.

```rust
.password(Cipher::reference("vault://production/database/password"))
```

Cipher is not a synonym for node-specific configuration. Ordinary scoped variables stay ordinary Canon parameters. Secrets need different guarantees.

Cipher plaintext never appears in Plan hashes, Event payloads, Trace output, error messages, debug logs, or telemetry.

```rust
trait CipherProvider {
    async fn resolve(&self, reference: &CipherRef) -> Result<SecretBytes>;
}
```

Candidate providers: a local protected file, an environment-backed development provider, HashiCorp Vault, cloud secret stores, and enterprise systems.

Nomos does not become a secrets database.

## 8. Reconciliation Model

The central abstraction is a level-triggered reconciliation loop. Kubernetes controllers work this way. Nomos adopts the principle without the Kubernetes object model.

For the Condition $C_r$ on resource $r$:

```text
O_r = observe(r)
A_r = assess(C_r, O_r)            # Satisfied | Variance | Indeterminate
match A_r:
  Satisfied     → nothing to plan for r
  Variance      → plan, apply, verify
  Indeterminate → report the reason; never plan a mutation from it
```

An Indeterminate Assessment of one resource does not hide a Variance of another. The report keeps every Assessment.

Convergence needs more than every Assessment Satisfied. A change can leave an Obligation, a follow-up effect no Condition can observe, and an effect that was started may not yet be Settled. Enforce is done only when every Assessment is Satisfied, every Obligation is discharged, and every relevant effect is Settled ([formal/reconciliation.md](formal/reconciliation.md)).

What actually establishes success? A successful Action means *the intended postcondition was observed*. It does not mean *a command exited with status zero*. Everything else builds on this distinction.

## 9. Substrate Resource Contract

A backend obtains evidence and performs operations. It does not interpret either ([ADR 0006](adr/0006-kernel-contract.md)). Every managed resource has a port shaped roughly like this:

```rust
trait ResourceBackend {
    type Condition;    // defined in core
    type Observation;  // defined in core

    async fn observe(&self, condition: &Self::Condition) -> Self::Observation;
    async fn apply(&self, request: &EffectRequest) -> EffectReceipt;
}
```

Assessment, planning, and verification are core functions, shared by every backend: `assess(condition, observations)` yields Satisfied, Variance, or Indeterminate; `plan` turns Assessments and Obligations into Actions; `verify` is `assess` on a fresh Observation. The mock and the Linux backend therefore cannot disagree about what satisfaction means, because neither of them decides it.

Each backend is a black box behind this contract:

```mermaid
flowchart TB
    RC["Resource contract"] --> L["Linux backend"]
    RC --> M["Mock backend"]
    RC --> B["Future BSD backend"]
```

The mock backend is not scaffolding. It is a first-class backend for deterministic reconciliation tests, and it runs the same assessment and planning code as Linux.

## 10. Initial Substrate Resources

Start with `file`, `directory`, `system_user`, `package`, `systemd_unit`, and `sysctl`.

Later: `mount`, `network`, `cgroup`, `kernel_module`, `timer`, `socket`, and container workloads.

Do not start by reproducing Salt's execution-module surface. Prove the resource contract first.

## 11. Atomic Filesystem Operations

`open → truncate → write` can destroy a valid configuration if it fails halfway. File convergence does this instead:

```mermaid
flowchart LR
    A["write temporary file"] --> B["fsync"] --> C["set metadata"] --> D["atomic rename"] --> E["fsync parent directory"] --> F["verify"]
```

Replacement is a transaction wherever Linux allows it. Observed content is compared by hash before anything reads or ships whole files.

## 12. systemd Integration

systemd is the authoritative service manager on the initial platform. Substrate talks to it over D-Bus (`org.freedesktop.systemd1`) for `GetUnit`, `StartUnit`, `StopUnit`, `RestartUnit`, `ReloadUnit`, unit properties, and job state. It does not scrape `systemctl`.

## 13. cgroup v2

Nomos is cgroup-v2-native. A bounded Action can run as:

```mermaid
flowchart LR
    A["Action"] --> S["transient systemd scope"] --> G["delegated cgroup"]
    G --> CPU["CPU limit"]
    G --> MEM["memory limit"]
    G --> IO["I/O limit"]
    G --> PID["process limit"]
```

Nomos cooperates with systemd through `Delegate=`. It does not fight PID 1 for the cgroup hierarchy. PID 1 tends to win those fights.

## 14. Warp Graph Model

Warp builds $G = (V, E)$, where $V$ is the set of Actions and $E$ the dependency constraints. The initial edge kinds:

- **`requires`.** B may execute only after A succeeds.
- **`after`.** B is ordered after A. B does not exist because A changed.
- **`on_change`.** B becomes actionable only if A produced a state change.

`watch` stays out of v0 unless it expresses something these three cannot.

## 15. Warp Algorithms

Warp needs surprisingly little machinery.

- **Cycle detection.** `A requires B`, `B requires C`, `C requires A` fails compilation. Use strongly connected components or the graph library's cycle reporting.
- **Topological ordering.** $O(|V| + |E|)$ for a valid DAG.
- **Execution frontier.** $\mathit{Ready} = \{ v \mid \mathit{dependencies}(v) = \mathit{Succeeded} \}$. Everything in Ready may run concurrently, subject to policy.
- **Conflict keys.** Actions can claim exclusive keys such as `package-manager:dpkg`, `file:/etc/hosts`, or `systemd:nginx.service`. Two Actions sharing a key never run at the same time. Independence in the graph is not independence in the operating system.

Details and proofs: [formal/warp.md](formal/warp.md).

## 16. Plan

A Plan is an internal compiled artifact, not another branded term.

```mermaid
classDiagram
    class Plan {
        ID
        Canon ID
        target snapshot
        Actions
        dependency edges
        constraints
        concurrency policy
        creation generation
        expiry / lease information
    }
```

Plan identity derives from canonical content where practical. Plans are immutable after acceptance. Changing a Plan produces a new Plan.

## 17. Optimistic Planning

Nomad separates evaluation, planning, validation, and execution, and rejects plans made stale by concurrent changes. Nomos uses a simpler version.

Loom compiles against a node-state generation, say 417. The Plan carries `expected_generation = 417`. Before execution, the authority checks that the assumption still holds. If reality has moved on, the Plan is rejected, state is observed again, and the Plan is recompiled.

This beats locking the whole fleet while planning.

## 18. Action

An Action is one bounded attempt to cause or verify a state transition.

```mermaid
classDiagram
    class Action {
        ID
        type
        target
        inputs
        preconditions
        operation
        postconditions
        timeout
        retry policy
        conflict keys
        idempotency key
    }
```

Lifecycle: `Prepared → Dispatched → Accepted → Running → Verifying → Succeeded`. The terminal alternatives are `Failed`, `TimedOut`, `Cancelled`, and `Rejected`.

A timeout is not a failure. The remote operation may have finished after the connection vanished. Nomos records the outcome as uncertain and observes again. It never assumes the operation did not happen.

## 19. Idempotency

Networks are imperfect. The baseline is at-least-once delivery plus idempotent execution.

Each Action carries an idempotency key, and the Cell persists the keys it accepts. When Loom retransmits after losing a response:

```mermaid
flowchart LR
    A["Action abc123"] --> C["Cell"] --> Q{"already<br/>completed?"}
    Q --> Y["yes<br/>return stored result"]
    Q --> N["no<br/>execute"]
```

Nomos does not claim magical exactly-once distributed execution. It gives explicit semantics under retries. Details: [formal/fencing-and-idempotency.md](formal/fencing-and-idempotency.md).

## 20. Fencing

Plans carry monotonically increasing generations (fencing tokens). A Cell that has accepted generation 52 rejects generation 51, because $51 < 52$.

This stops stale controllers, delayed packets, and reconnect races from resurrecting obsolete Actions. It matters even more once Loom is highly available.

## 21. Loom Targeting

Fleet selection evaluates predicates over Traits:

```sh
nomos-loom trace \
  --target 'traits.os == "debian" && traits.tier == "edge"' \
  --canon telemetry-stack
```

$$
\mathit{Target} = \{ n \in \mathit{Nodes} \mid \mathit{Predicate}(\mathit{Traits}(n)) \}
$$

For small fleets, scan an in-memory indexed map. At larger scale, common equality predicates can use roaring bitmaps (Trait → value → bitmap of NodeIDs). Add that when measurements demand it, not before.

## 22. Loom Scheduling

Once Warp exposes a frontier, Loom applies operational constraints:

```yaml
execution:
  parallelism: 8
  max_unavailable: 1
  failure_domain:
    trait: rack
    max_unavailable: 1
```

$$
\mathit{Runnable} = \mathit{Ready} \cap \mathit{PolicyAllowed} \cap \mathit{CapacityAvailable}
$$

v0 needs bounded concurrency, dependency readiness, per-node exclusivity, conflict keys, max-unavailable budgets, cancellation, and backpressure.

Weighted fair queuing, work stealing, critical-path prioritization, adaptive concurrency, and topology-aware scheduling are later research.

## 23. Cell Architecture

```mermaid
flowchart TB
    CELL["CELL"] --> P["protocol"]
    CELL --> O["observer"]
    CELL --> ES["event spool"]
    P --> V["validator"] --> X["executor"] --> S["Substrate"]
    O --> T["Traits"]
```

The Cell owns execution semantics. Loom may ask it to converge resource X. The Cell decides whether the local operation actually achieved its postcondition. Operating-system truth stays close to the operating system.

## 24. Cell Offline Behavior

A disconnected Cell may:

- observe state
- run a local Trace
- execute explicitly local Canon
- persist Events
- finish already-authorized, bounded work where policy allows

A disconnected Cell does not accept imaginary new fleet authority. Central Plans carry leases and expiry for exactly this reason.

When connectivity returns:

```mermaid
flowchart LR
    A["Cell Event spool"] --> B["batched upload"] --> C["Loom acknowledgement"] --> D["local checkpoint advances"]
```

## 25. Event

Every meaningful control transition emits an Event.

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

## 26. Event Log

Event Log is the canonical term. Not Journal. Not Ledger.

The Event Log is an append-only sequence of immutable operational Events. It supports recovery, audit, materialized fleet state, debugging, Plan history, and external integration.

Conceptually, $\mathit{State}_t = \mathrm{fold}(\mathit{Event}_0, \ldots, \mathit{Event}_t)$. In practice, implementations keep materialized state so that queries do not replay all of history.

## 27. Event Ordering

Wall-clock timestamps do not establish causality.

Each writer maintains a `writer_sequence`, which orders its own Events strictly. Loom assigns its own ingestion position on receipt. Causality across nodes is explicit, through `causation_id`, `correlation_id`, `plan_id`, and `action_id`. `timestamp(A) < timestamp(B)` does not imply that A caused B.

Hybrid logical clocks are a reasonable later candidate. Vector clocks need a concrete requirement first, because fleet-wide vector metadata scales poorly.

## 28. Event Log Integrity

Append-only is not the same as tamper-proof. v0 guarantees:

- no supported update operation
- no supported in-place mutation
- a monotonic writer sequence
- transactional append

A later integrity mode can add a hash chain, $H_n = H(H_{n-1} \Vert \mathit{Event}_n)$, followed by signed checkpoints or Merkle structures. Only then does "Ledger" become a defensible word.

## 29. Persistence

The storage API stays abstract. What does the engine actually have to do? Append durably, look records up by key, and survive a crash mid-transaction. `redb`, a pure-Rust embedded ACID key-value store built on copy-on-write B-trees, does all three and is a strong v0 candidate. That is a hypothesis for the ablation to test, not a verdict.

Possible layout: `events`, `event_by_id`, `actions`, `action_idempotency`, `traits`, `materialized_nodes`, `plans`, `canons`, `metadata`.

Before the choice is frozen, an ablation compares redb, SQLite, and an LMDB-family option on append throughput, crash recovery, fsync behavior, query ergonomics, binary size, dependency burden, corruption recovery, and operational transparency.

## 30. Control Protocol

Protocol and transport are separate abstractions. The starting point is a Protocol Buffers schema, a tonic-compatible service model, and HTTP/2 with TLS.

The Cell initiates the connection to Loom. That works through NAT, host firewalls, cloud networks, and enterprise networks without exposing a management port on every host.

The stream carries registration, Traits, heartbeats, Plan assignments, Action status, Events, and acknowledgements. QUIC or another transport can be added later without touching the domain schema.

## 31. Backpressure

Loom never produces work at a higher rate than Cells can safely consume it.

Each Cell advertises its capacity, for example `max_parallel_actions = 4` and `queue_capacity = 32`. Loom keeps bounded queues. When capacity runs out, the producer slows down. Memory usage does not approach infinity.

Bounded channels and explicit admission control are standard everywhere. Tokio cancellation is cooperative, and dropping a future does not reverse side effects. That is one more reason for an explicit Action lifecycle and explicit timeouts.

## 32. Identity

Nomos uses `NodeID`, `CanonID`, `PlanID`, `ActionID`, and `EventID`. Identity is fundamental, but it does not need another themed noun.

Cell identity will be cryptographically verifiable. Nomos borrows SPIFFE's principles (workload identity, trust domains, short-lived certificates, mutual authentication, automatic rotation) without requiring a SPIRE deployment. SPIFFE compatibility can come later as an integration.

## 33. Security Boundary

A Cell can change `/etc`, packages, services, users, and kernel parameters. That makes it a remote root-management system. Pretending otherwise only gives root access nicer branding.

The Cell is therefore privilege-separated:

```mermaid
flowchart TB
    N(("network")) --> U["unprivileged Cell"]
    U --> IPC(["typed local IPC"]) --> P["privileged executor"]
    P --> S["Substrate"]
```

The privileged side accepts typed operations only, never serialized shell commands. The two halves talk over local inter-process communication (IPC): a root-owned Unix-domain socket, with the kernel's peer credentials (`SO_PEERCRED`) identifying the caller. Parsing, networking, and protocol complexity stay out of the most privileged process.

## 34. Shell Escape Hatch

Arbitrary shell execution is not a reconciliation primitive. If it is ever added, it is explicitly opaque:

```rust
Exec::opaque(command)
    .precondition(before)
    .postcondition(after)
```

Policy can forbid it entirely. Arbitrary commands make idempotence, expected mutation, authorization scope, rollback, affected resources, verification, and permissions much harder to reason about. Typed operations come first.

## 35. Linux Hardening

Nomos services use systemd hardening where their responsibilities allow: `NoNewPrivileges`, `ProtectSystem`, `ProtectHome`, `PrivateTmp`, `RestrictSUIDSGID`, capability bounding, seccomp, cgroup limits, and resource controls.

Loom can be hardened aggressively. The Cell's privilege boundaries must match the mutations it actually performs, and the privileged helper needs the most carefully tailored policy of all.

## 36. Polkit

Polkit is not the foundation of network authorization. It may help a local human run a privileged operation through the CLI. Loom-to-Cell authorization belongs to Nomos's authenticated protocol and policy model.

## 37. Trace

Trace guarantees non-mutation.

```sh
nomos-cell trace --canon artifacts/system
```

```text
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

FILE /etc/example.d/secret.conf
  INDETERMINATE: read denied (EACCES)
  action: none
```

The last entry is an Indeterminate Assessment. Trace reports it with its reason and plans nothing for it.

Trace uses exactly the same decode, observe, assess, and compile pipeline as Enforce. Only execution is left out. A dry run that secretly takes a different code path is a lie with good manners.

## 38. Enforce

```mermaid
flowchart LR
    C[/"Canon artifact"/] --> O["Observe"] --> AS["Assess"] --> W["Warp"] --> P["Plan"] --> A["Actions"] --> VF["Verify"] --> O2["Observe again"]
```

Enforce ends `Converged` when every Condition is Satisfied, no Obligation is pending, and every relevant effect is Settled (§8). It ends `Indeterminate` when nothing is left to plan but some evidence is insufficient, and `NonConvergent` or `Failed` otherwise ([formal/reconciliation.md](formal/reconciliation.md)). Empty Variance alone is not convergence.

Reconciliation loops are bounded. If A changes X and B changes it back, over and over, Nomos detects the non-convergence. It does not cheerfully consume electricity forever.

## 39. Fixed-Point Property

If $\mathrm{Enforce}(\mathit{Canon}, S) = S'$ and the Canon is converged on the host $S'$, then $\mathrm{Enforce}(\mathit{Canon}, S') = S'$.

The second application performs no mutating Actions. This is one of Nomos's most important property-based tests.

## 40. Failure Semantics

Nomos treats all of these as normal operating conditions, not exotic theory:

- process and host crashes
- network partitions
- message duplication, loss, and delay
- partial execution
- Loom and Cell restarts
- clock skew
- stale Plans

Every outcome must be representable.

## 41. Availability Model

Version 0 runs one Loom and many Cells.

What fails if the coordinator disappears? New fleet decisions. Nothing a Cell has already observed or been authorized to do. Cell state is not corrupted, and Cells stay locally inspectable. That is intentional. Do not add consensus merely because Nomos involves more than one computer.

## 42. Future Loom High Availability

Is that actually consensus, or coordination wearing a consensus hat? Single-writer metadata with failover is a narrower problem than replicated state-machine consensus. Consensus becomes necessary once multiple replicas must agree on an ordered sequence of state changes despite failures. If real requirements get there, Raft fits Canon registrations, Plan generations, ownership and leases, and critical control metadata. OpenRaft is a current Rust implementation.

Consensus does not replicate Trait samples, debug logs, metrics, or high-volume telemetry just because a consensus engine is available. Not everything deserves a quorum.

## 43. Consistency Under Partition (CAP)

Nomos does not solve the consistency, availability, and partition-tolerance trade-off. It chooses consistency semantics per subsystem.

During a partition, authoritative Loom state prefers refusing conflicting control decisions to letting multiple authorities mutate the same fleet. That part leans toward consistency.

Cells keep local availability for observation, Trace, cached state, and explicitly authorized local convergence. The system does not need one universal CAP choice.

## 44. Event Log vs. Telemetry

Nomos Events are control-plane records, not an observability pipeline. CPU samples, log lines, packets, and metrics do not belong in the authoritative Event Log.

```mermaid
flowchart LR
    N["Nomos Event Log<br/>operational truth"]
    F["FabricO11y<br/>high-volume telemetry"]
    N --> A(["integration adapter"]) --> F
```

The adapter exports Events such as Action started, Action completed, Canon changed, Variance detected, Cell disconnected, Plan rejected, and convergence failed. FabricO11y observes systems. Nomos controls them. Their storage architectures stay independent.

## 45. Testing Architecture

Nomos is built for aggressive testing from day one.

```mermaid
flowchart LR
    U["Unit"] --> P["Property"] --> G["Graph"] --> C["Concurrency"] --> CR["Crash"] --> I["Integration"] --> D["Distributed failure"]
```

## 46. Property-Based Testing

- **Idempotence.** $\mathrm{enforce}(\mathrm{enforce}(S)) = \mathrm{enforce}(S)$.
- **Trace purity.** $\mathit{SubstrateBefore} = \mathit{SubstrateAfter}$ for every Trace.
- **Graph validity.** Every dependency precedes its dependent Action.
- **Determinism.** An identical Canon artifact, Traits, and Observations produce identical logical Plans.
- **Unknown evidence.** No Indeterminate Assessment becomes a Variance, and no Action is planned from one.
- **Secret non-disclosure.** No Cipher plaintext appears in serialized Events.
- **Event monotonicity.** For every writer, $\mathit{sequence}_{n+1} > \mathit{sequence}_n$.
- **Stale fencing.** An Action with a generation below the latest accepted generation never executes.

## 47. Concurrency Testing

There is a wonderfully inconvenient naming collision here. Rust already has a crate called `loom`. It systematically explores thread interleavings to expose concurrency bugs, instead of hoping random tests eventually hit them.

That is exactly what Nomos needs. The component keeps the name `nomos-loom`, and the test dependency is aliased:

```toml
loom-model = { package = "loom", version = "..." }
```

Queues, leases, and state transitions then get deterministic concurrency tests. The universe has apparently decided to participate in the naming scheme.

## 48. Formal Specification

Nomos keeps a small TLA+ model next to the implementation in `formal/tla/`. It models the control protocol, not Linux.

State variables: `plans`, `actions`, `nodes`, `accepted_generation`, `action_state`, `event_sequence`, `leases`.

Initial invariants:

- **Dependency safety.** $\mathrm{Running}(a) \Rightarrow \forall d \in \mathrm{Requires}(a) : \mathrm{Succeeded}(d)$.
- **Fencing safety.** $\mathrm{Execute}(a) \Rightarrow \mathrm{Generation}(a) \ge \mathrm{AcceptedGeneration}(\mathit{node})$.
- **Verified success.** No Action becomes `Succeeded` without successful verification.
- **Single active authority.** A node never accepts conflicting Plan generations at the same time.
- **Bounded disruption.** For policy $k$, $\mathrm{Unavailable}(\mathit{FailureDomain}) \le k$.
- **Event monotonicity.** Each writer sequence strictly increases.

TLA+ finds illegal interleavings before they become Rust integration tests. Written statements and proof sketches live in [formal/](formal/).

## 49. Crash Testing

Every persistent transition gets killed on purpose:

- the Cell after accepting an Action
- the Cell during an operation
- the Cell after an operation, before the Event append
- the Cell after the Event append, before acknowledgement
- Loom after dispatch
- Loom after receiving a result, before persistence

Recovery must be deterministic. This is how the Event Log earns its existence instead of becoming architectural decoration.

## 50. Workspace

The workspace follows a hexagonal layout. See [architecture/hexagon.md](architecture/hexagon.md) and architecture decision record (ADR) [0000](adr/0000-foundations.md).

Crates beyond the four components (`nomos-canon`, `nomos-store`, `nomos-protocol`, and the port and adapter crates) are implementation boundaries, not user-facing concepts.

## 51. CLI

Loom:

```sh
nomos-loom compile --canon artifacts/base-system
nomos-loom trace   --target 'traits.os == "debian"' --canon artifacts/hardened-host
nomos-loom enforce --target 'traits.tier == "edge"' --canon artifacts/telemetry-stack
```

| Command | Effect |
| --- | --- |
| `compile` | Validate a Canon artifact and report its `CanonID` |
| `trace` | Distributed, non-mutating evaluation |
| `enforce` | Coordinate fleet convergence |

Cell:

```sh
nomos-cell traits
nomos-cell trace   --canon /etc/nomos/artifacts/local
nomos-cell enforce --canon /etc/nomos/artifacts/local
nomos-cell events
```

| Command | Effect |
| --- | --- |
| `traits` | Show local Traits |
| `trace` | Evaluate local Variance |
| `enforce` | Standalone convergence |
| `events` | Inspect the local Event Log |

`--canon` names a Canon artifact, a `.cbor` file ([ADR 0011](adr/0011-canon-artifact-encoding.md)). The paths above predate the choice and carry no extension.

## 52. Design Lineage

Nomos inherits principles, not products.

| System or concept | Lesson Nomos takes |
| --- | --- |
| Salt | Broad host-state management, remote execution, targeting |
| Kubernetes | Level-triggered reconciliation and independently understandable controllers |
| Nomad | Desired vs. emergent state, optimistic planning, validation before commitment |
| Nix | Deterministic declarative inputs and content-derived identity |
| Temporal | Durable execution state and explicit recovery after failures |
| systemd | Typed native service control, cgroup ownership, Linux-native resource lifecycle |
| SPIFFE | Workload identity and short-lived cryptographic credentials |
| Raft | Consensus only for genuinely replicated authoritative state |
| Event sourcing | Immutable transitions plus derived materialized state |
| TLA+ | Control-protocol safety verified independently of the implementation |
| Rust `loom` | Systematic exploration of concurrency interleavings |

Nomos takes these properties selectively. It does not absorb their architectures.

## 53. Explicit Non-Goals for v0

Nomos v0 is not:

- a Kubernetes replacement
- a container orchestrator
- a secrets manager
- a telemetry database
- a service mesh
- a package ecosystem
- a distributed filesystem
- a general workflow engine
- a shell-command broadcasting tool
- a multi-region consensus platform
- a new Linux init system
- a universal cross-platform abstraction

The reference problem is simpler: *reliably describe, inspect, change, and verify Linux host state, locally and across a fleet.* That problem is hard enough without trying to colonize computing.

## 54. Initial Research Ablations

Uncertain choices are settled by experiment before they are frozen.

- **Persistence.** redb vs. SQLite vs. an LMDB-family store.
- **Canonical IR encoding.** A restricted deterministic profile of Concise Binary Object Representation (CBOR) vs. the JSON Canonicalization Scheme (JCS), with the same typed model either way. Settled: both passed every semantic test, and [ADR 0011](adr/0011-canon-artifact-encoding.md) chose deterministic CBOR as the simpler.
- **Transport.** Validate tonic over HTTP/2 against requirements before looking at QUIC.
- **File observation.** Metadata plus hash vs. full read vs. notification-assisted caching.
- **Fleet targeting.** Predicate scans vs. bitmap indexes, at large synthetic fleet sizes only.
- **Reconciliation scheduling.** Sequential vs. bounded DAG parallelism vs. conflict-aware DAG parallelism.

Do not pick sophisticated algorithms because their papers have attractive diagrams.

## 55. Development Phases

- **Phase 0: Semantics.** No networking. Build `nomos-core`, `nomos-canon`, `nomos-substrate-mock`, and `nomos-warp`. Prove that Canon → Observation → Variance → Plan is deterministic.
- **Phase 1: Masterless Cell.** Add the Linux Substrate, Debian first, with `file`, `directory`, `system_user`, `systemd_unit`, `sysctl`, and `package`, and the commands `traits`, `trace`, and `enforce`. Done when a Debian machine converges locally from any supported starting state.
- **Phase 2: Event Log and recovery.** Make every Action transition durable. Kill the Cell at every lifecycle boundary and prove deterministic recovery.
- **Phase 3: Loom.** Introduce one coordinator with Cell registration, Traits, targeting, Plan dispatch, Action results, and Event ingestion. No consensus.
- **Phase 4: Fleet safety.** Bounded parallelism, max-unavailable, failure domains, fencing, leases, backpressure, and stale-Plan rejection. This is where Nomos becomes genuinely useful for production maintenance.
- **Phase 5: Privilege separation.** Split network-facing Cell logic from a minimal privileged executor, with local IPC, peer authentication, a typed privileged API, systemd hardening, and seccomp where practical.
- **Phase 6: External integrations.** Cipher providers, FabricO11y, CI/CD, change management, and identity providers, all as adapters outside the core model.
- **Phase 7: High availability.** Only if real operating goals require it: multiple Loom replicas, leader election, and replicated authoritative state. Evaluate OpenRaft and alternatives then.

Consensus is an extension of the architecture, not a prerequisite for compiling a file manifest.

## 56. First End-to-End Demonstration

The first serious demonstration manages a sibling project: Nomos deploys the FabricO11y agent onto Debian hosts. The Canon covers a system user, directories, the binary artifact, configuration, the systemd service, resource limits, service enablement, and service health.

```mermaid
flowchart TB
    A["fresh Debian VM"] --> B["nomos trace"] --> C["Variance shown"] --> D["nomos enforce"]
    D --> E["FabricO11y installed"] --> F["systemd service running"] --> G["postcondition verified"]
    G --> H["Event Log complete"] --> I["nomos enforce again"] --> J(["0 mutating Actions"])
```

Then break the host on purpose: delete the config, stop the service, change permissions, and alter a sysctl. Trace must identify the exact Variance. Enforce must restore the Canon.

## 57. Failure Demonstration

The second demonstration is deliberately hostile to Nomos itself. During convergence:

- kill the Cell
- restart the Cell
- disconnect the network
- deliver a Plan twice
- delay an old Action
- restart Loom

Expected outcome:

- no duplicate destructive mutation
- no stale Action execution
- no false success
- no corrupted Event Log
- eventual convergence after recovery

This proves more than a benchmark of 40,000 meaningless no-op commands per second.

## 58. Core Safety Invariants

The constitutional layer.

- **N1.** Trace does not mutate Substrate.
- **N2.** Successful Enforce converges supported resources toward Canon.
- **N3.** Re-enforcing converged Canon performs no required mutation.
- **N4.** No Action executes before its hard dependencies succeed.
- **N5.** A stale, fenced Plan cannot execute.
- **N6.** `Succeeded` requires verified postconditions.
- **N7.** Events are never mutated after append.
- **N8.** Cipher plaintext never enters persistent operational records.
- **N9.** Failure-budget constraints are never intentionally exceeded.
- **N10.** An unknown execution outcome stays unknown. It is never silently converted to success or failure.
- **N11.** Losing Loom does not invalidate Observations the Cell has already collected.
- **N12.** Canon compilation is deterministic for identical inputs.

- **N13.** Unknown evidence does not imply noncompliance. An Indeterminate Assessment is never counted as a Variance, and every planned Action is caused by a Variance or an Obligation, never by an Indeterminate Assessment.

N13 was proposed by [ADR 0005](adr/0005-assessment-vocabulary.md) and numbered when milestone `03-assessment-kernel` landed the tests that check its Assessment clause; the Obligation clause is checked when a Plan exists. N1–N12 kept their numbers.

These invariants matter more than any implementation technology. Formal statements: [formal/invariants.md](formal/invariants.md).

## 59. Architectural Thesis

Nomos is five transformations. Conditions from the Canon and Observations from Substrate produce Assessments. Variances and Obligations produce a Warp graph. Warp produces Actions. Actions operate through Substrate. Every meaningful transition produces Events.

```mermaid
flowchart TB
    CANON[/"<b>CANON</b><br/>compiled desired intent"/] --> LOOM["<b>LOOM</b><br/>targeting / planning"]
    LOOM --> WARP["<b>WARP</b><br/>Action DAG"]
    WARP --> CELL["<b>CELL</b><br/>validated execution"]
    CELL --> SUB["<b>SUBSTRATE</b><br/>Linux"]
    SUB --> OBS(["OBSERVATION"])
    OBS --> TR(["TRAITS"])
    OBS --> AS(["ASSESSMENT<br/>Satisfied · Variance · Indeterminate"])
    TR --> LOOM
    AS --> LOOM
    X["every transition"] --> EV["EVENT"] --> EL[("EVENT LOG")]
```

The governing idea fits in one sentence:

> Observe reality, compare it with Canon, derive a deterministic, valid set of required changes, apply those changes under explicit safety constraints, verify reality again, and preserve what happened as immutable history.

Everything else is machinery in service of that loop.

## 60. Relationship to Moiric

```mermaid
flowchart TB
    M["<b>MOIRIC</b>"]
    M --> F["<b>FabricO11y</b><br/>Observe distributed systems.<br/><i>What is happening?</i>"]
    M --> N["<b>Nomos</b><br/>Reconcile and control them.<br/><i>What should be happening,<br/>and how do we safely get there?</i>"]
    M --> T["<b>Metron</b><br/>Bound and govern computation.<br/><i>Within what boundaries<br/>may computation happen?</i>"]
```

The three stay independently useful, with narrow integration surfaces. They do not slowly merge into one enormous platform. That separation is architecture, not branding.

## 61. Definition of v0 Success

Nomos v0 succeeds when it demonstrates all of the following on real Debian hosts:

1. Build Canon from Rust to a Canonical IR and compile it deterministically.
2. Discover and expose typed Traits.
3. Observe supported Linux resources without mutation.
4. Compute precise Variance.
5. Compile valid Action DAGs and reject cycles.
6. Produce a trustworthy Trace.
7. Enforce Canon locally through the Cell.
8. Verify every successful state transition.
9. Produce no mutating Actions after convergence.
10. Persist an append-only Event Log.
11. Recover coherently after Cell termination.
12. Connect Cells to a single Loom.
13. Select fleets with Trait predicates.
14. Dispatch and deduplicate Actions reliably.
15. Reject stale Plans with fencing.
16. Enforce concurrency and disruption budgets.
17. Recover from temporary network disconnection.
18. Keep Cipher plaintext out of persistent records.
19. Export operational Events to FabricO11y.
20. Demonstrate the key protocol invariants through model, property, and failure testing.

Anything beyond this is subsequent architecture. Nomos earns its complexity one invariant at a time.

## 62. Open Questions

The unresolved parts, stated so they can be argued with:

- **Edge semantics.** Does `after` wait for any terminal outcome, and does `on_change` imply ordering? Closed: [ADR 0009](adr/0009-warp-activation-semantics.md), accepted 2026-09-28, decides both as [formal/warp.md](formal/warp.md) states them.
- **Idempotency key retention.** The Cell persists accepted keys. For how long? Unbounded retention is a slow disk leak. Bounded retention reopens the duplicate window for very late retransmissions.
- **Leases vs. clock skew.** Plan leases expire in time, and clocks drift. Does expiry use Loom's clock, the Cell's clock, or a monotonic budget measured from receipt?
- **Secrets during Trace.** Some observations may need a Cipher, for example comparing the hash of a rendered file that contains a password. Does Trace resolve secrets, or does it assess that Condition as Indeterminate? [ADR 0013](adr/0013-trust-boundaries.md) proposes the second, unless a provider can compare without exposing plaintext.
- **Mesh identity vs. Nomos identity.** Headscale authenticates nodes on the network. Nomos authorizes Cells to act. How are the two bound, so a compromised tailnet key does not become fleet authority?
- **Persistence.** redb, SQLite, or an LMDB-family store. The ablation in §54 decides.

Canon ([ADR 0004](adr/0004-rust-typed-canon.md) decides the authoring surface and the inert-artifact boundary, and [ADR 0011](adr/0011-canon-artifact-encoding.md) proposes identity and versioning rules; these remain open):

- **Artifact encoding.** Closed: [ADR 0011](adr/0011-canon-artifact-encoding.md), accepted 2026-09-29, chooses deterministic CBOR (§54).
- **Artifact container.** The extension is `.cbor` and the artifact is the bare encoded value; whether signing needs a container around it is open with signing.
- **Content hash profile.** The algorithm and domain separation behind $H$ in §6. [canon-ir.md](formal/canon-ir.md) uses SHA-256, from the Secure Hash Algorithm (SHA) 2 family, over a profile tag, the schema version, and the canonical bytes as a working definition.
- **Artifact signing.** Whether artifacts are signed, by whom, and what a Cell checks.
- **Extensibility model.** Whether resource kinds are a closed set or can be extended, and how an unknown kind is carried without being executed. Today an unknown kind is refused for execution and readable only by archival inspection ([canon-ir.md](formal/canon-ir.md)).
- **Condition composition.** Whether Conditions compose beyond a set, for example by disjunction or negation, and what a repair means for a composite.
- **Trait-dependent expressions.** How a Condition may depend on a Trait value, and how that dependency is bounded.
- **Authoring API form.** Builders, a macro layer, or both.
