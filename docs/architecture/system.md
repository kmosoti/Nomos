# System

## Components

Nomos is the project, protocol and shared model. It is not a daemon. It has
four architectural components (spec §2).

```mermaid
flowchart TB
    N["<b>Nomos</b><br/>semantics · protocol · schemas"]
    L["<b>Loom</b><br/>fleet coordinator / compiler"]
    C["<b>Cell</b><br/>node-resident executor"]
    W["<b>Warp</b><br/>dependency &amp; execution graph<br/><i>Loom compiles through it</i>"]
    S["<b>Substrate</b><br/>operating-system boundary<br/><i>Cell operates through it</i>"]

    N --- L
    N --- C
    L --> W
    C --> S
    C --> Stream(["outbound authenticated stream<br/>Cell initiates"]) --> L
```

| Component | Binary / crate | Role |
|---|---|---|
| Loom | `nomos-loom` | Accepts Canon, tracks Cells, resolves targets, plans, dispatches, ingests Events. Single authority in v0. |
| Cell | `nomos-cell` | Discovers Traits, observes, executes, verifies, spools Events. Works without Loom. |
| Warp | `nomos-warp` | Compiles resources and Variance into an Action DAG. |
| Substrate | `nomos-substrate` (+ adapters) | Typed operations against the OS, using native APIs rather than shell. |

## The control loop

```mermaid
flowchart TB
    Canon[/"Canon<br/>desired state"/] --> Loom
    Loom["Loom<br/>targeting · planning"] --> Warp["Warp<br/>Action DAG"]
    Warp --> Cell["Cell<br/>validated execution"]
    Cell --> Substrate["Substrate<br/>Linux"]
    Substrate --> Obs(["Observation"])
    Obs --> Traits(["Traits"])
    Obs --> Var(["Variance"])
    Traits --> Loom
    Var --> Loom

    Cell -.-> Ev[("Event Log<br/>every transition")]
    Loom -.-> Ev
```

A Cell runs the same loop locally, with no Loom involved:

```mermaid
flowchart LR
    Canon[/"local Canon"/] --> Observe --> Diff["Variance"]
    Diff --> Done(["Variance = ∅<br/>converged"])
    Diff --> Plan["Variance ≠ ∅<br/>Warp → Plan"] --> Apply --> Verify --> Observe
```

## Deployment topology (v0)

One Loom and many Cells. Cells connect **outward** to Loom over the Mesh
(Headscale, [ADR 0001](../adr/0001-headscale-mesh-adapter.md)). Secrets
resolve at the point of use through the Cipher port (Vault,
[ADR 0002](../adr/0002-vault-cipher-adapter.md)).

```mermaid
flowchart LR
    subgraph Control["control plane"]
        Loom["nomos-loom<br/>Cells connect outbound over the tailnet"]
        HS["Headscale<br/>Cells enroll"]
        Vault["Vault<br/>Cipher resolved at use"]
    end
    subgraph Hosts["Debian hosts"]
        C1["nomos-cell"]
        C2["nomos-cell"]
        C3["nomos-cell"]
    end
    C1 & C2 & C3 --> HS
    C1 & C2 & C3 --> Loom
    C1 & C2 & C3 --> Vault
    Loom -.-> F["FabricO11y<br/>receives Nomos Events"]
```

## Place in Moiric

```mermaid
flowchart LR
    F["<b>FabricO11y</b><br/>What is happening?"]
    N["<b>Nomos</b><br/>What should be happening,<br/>and how do we safely get there?"]
    M["<b>Metron</b><br/>Within what boundaries<br/>may computation happen?"]
    N --> E(["operational Events"]) --> F
```

They are independent systems with narrow integration surfaces.
