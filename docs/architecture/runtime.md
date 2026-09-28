# Runtime

## Canon compilation

Human-readable Canon is never executed directly (spec §6).

```mermaid
flowchart LR
    Y[/"Canon YAML"/] --> P["Parser"] --> AST["Typed AST"] --> V["Validation"] --> IR["Canonical IR"]
    IR --> H["H(CanonicalEncoding)"] --> CID(["CanonID"])
    IR --> W["Warp"] --> DAG(["Plan DAG"])
```

The canonical IR serializes deterministically, so identical Canon always has
the same `CanonID` (invariant N12, see [formal/invariants.md](../formal/invariants.md)).

## Trace and Enforce share one pipeline

Trace is Enforce with the execution stage removed. It is not a separate
"dry-run" implementation (spec §37).

```mermaid
flowchart LR
    C[/"Canon"/] --> Parse --> Observe --> Diff["Variance"] --> Compile["Warp → Plan"]
    Compile --> Report(["Trace: Variance report<br/>no mutation"])
    Compile --> Execute["Enforce: execute Actions"] --> Verify --> Observe2{"Observe again"}
    Observe2 --> Converged(["Variance = ∅<br/>converged"])
    Observe2 --> Again["Variance ≠ ∅, within bound<br/>compile again"] --> Compile
    Observe2 --> NC(["bound exceeded or oscillation<br/>non-convergence"])
```

## Action lifecycle

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
|---|---|
| `Dispatched → Rejected` | stale generation or unauthorized |
| `Verifying → Succeeded` | postcondition observed |
| `Verifying → Failed` | postcondition not observed |
| `TimedOut → end` | outcome unknown; state is re-observed |

`TimedOut` does not mean the Action failed. The remote side may have completed
it. The outcome is recorded as unknown and settled by re-observing
(invariant N10).

## Cell internals

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

Planned privilege separation (spec §33, Phase 5):

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

### Disconnection and recovery

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

## Optimistic planning

```mermaid
flowchart LR
    E["evaluate<br/>@ generation g"] --> P["Plan<br/>expected_generation = g"] --> V{"generation<br/>still g?"}
    V --> X["still g<br/>execute"]
    V --> R["changed<br/>reject stale Plan"] --> O["observe again"] --> E
```
