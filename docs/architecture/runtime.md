# Runtime

## Canon Compilation

The author's Rust is never executed on a host (spec §5–§6, [ADR 0004](../adr/0004-rust-typed-canon.md)). An authoring crate runs once in an isolated build job and emits the Canonical IR. Loom and Cell accept only the IR.

```mermaid
flowchart LR
    R[/"Rust authoring crate"/] --> B["isolated build job"] --> IR["Canonical IR"]
    IR --> V["validated decode"] --> H["H(CanonicalEncoding)"] --> CID(["CanonID"])
    V --> W["Warp"] --> DAG(["Plan DAG"])
```

The canonical IR serializes deterministically, so identical Canon always gets the same `CanonID`. That is invariant N12; see [formal/invariants.md](../formal/invariants.md). Reproducibility of the build job that produces the IR is a separate obligation, tested by the `build-hermeticity` grounding experiment.

## Trace and Enforce Share One Pipeline

Trace is Enforce with the execution stage removed. There is no separate dry-run implementation to drift out of sync (spec §37).

```mermaid
flowchart LR
    C[/"Canonical IR"/] --> Decode --> Observe --> Diff["Variance"] --> Compile["Warp → Plan"]
    Compile --> Report(["Trace: Variance report<br/>no mutation"])
    Compile --> Execute["Enforce: execute Actions"] --> Verify --> Observe2{"Observe again"}
    Observe2 --> Converged(["Variance = ∅<br/>converged"])
    Observe2 --> Again["Variance ≠ ∅, within bound<br/>compile again"] --> Compile
    Observe2 --> NC(["bound exceeded or oscillation<br/>non-convergence"])
```

## Action Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Prepared
    Prepared --> Dispatched
    Dispatched --> Accepted
    Dispatched --> Rejected
    Accepted --> Running
    Running --> Verifying
    Verifying --> Succeeded
    Verifying --> Failed
    Running --> Failed
    Running --> TimedOut
    Prepared --> Cancelled
    Dispatched --> Cancelled
    Accepted --> Cancelled
    TimedOut --> [*]
    Succeeded --> [*]
    Failed --> [*]
    Rejected --> [*]
    Cancelled --> [*]
```

| Transition | Condition |
| --- | --- |
| `Dispatched → Rejected` | Stale generation, or unauthorized |
| `Verifying → Succeeded` | Postcondition observed |
| `Verifying → Failed` | Postcondition not observed |
| `TimedOut → end` | Outcome unknown; state is observed again |

`TimedOut` does not mean the Action failed. The remote side may have finished the work after the connection dropped. The outcome is recorded as unknown and settled by observing again (invariant N10).

## Cell Internals

```mermaid
flowchart TB
    net(("Loom stream")) --> proto["protocol"]
    proto --> val["validator<br/>auth · freshness · fencing"]
    val --> exec["executor"]
    exec --> sub["Substrate"]
    obs["observer"] --> traits["Traits"]
    obs --> sub
    exec --> spool[("event spool")]
    spool --> proto
```

Privilege separation is planned for Phase 5 (spec §33):

```mermaid
flowchart TB
    net(("network")) --> cell["unprivileged Cell<br/>parsing · protocol · networking"]
    cell --> ipc(["typed local IPC<br/>Unix socket + peer credentials"]) --> priv["privileged executor"]
    priv --> sub["Substrate"]
```

## Loom ↔ Cell

```mermaid
sequenceDiagram
    autonumber
    participant C as Cell
    participant M as Mesh (Headscale)
    participant L as Loom

    C->>M: enroll (pre-auth key)
    C->>L: open outbound stream, register
    C->>L: Traits, capacity (max_parallel, queue_capacity)
    loop heartbeat
        C-->>L: heartbeat
    end
    L->>L: target (predicate over Traits), compile Plan @ generation g
    L->>C: Plan assignment (generation g, lease, idempotency keys)
    alt g < accepted_generation
        C-->>L: Rejected (stale)
    else fresh
        C->>C: execute, verify, append Events
        C-->>L: Action status + Events
        L-->>C: acknowledge → Cell advances checkpoint
    end
```

### Disconnection and Recovery

```mermaid
sequenceDiagram
    participant C as Cell
    participant L as Loom
    Note over C,L: connection lost
    C->>C: observe · local Trace · local Canon · spool Events
    Note over C: no new fleet authority accepted<br/>expired leases are not executed
    Note over C,L: connection restored
    C->>L: batched spool upload
    L-->>C: acknowledgement
    C->>C: checkpoint advances
```

## Optimistic Planning

Loom plans against a generation and checks it before executing, instead of locking the fleet while it thinks (spec §17).

```mermaid
flowchart LR
    E["evaluate<br/>@ generation g"] --> P["Plan<br/>expected_generation = g"] --> V{"generation<br/>still g?"}
    V --> X["still g<br/>execute"]
    V --> R["changed<br/>reject stale Plan"] --> O["observe again"] --> E
```
