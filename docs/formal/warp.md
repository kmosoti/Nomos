# Warp

## Graph Model

$$
G = (V, E), \qquad E = E_{req} \cup E_{after} \cup E_{chg}
$$

$G$ is a directed acyclic graph (DAG). $V$ is the set of Actions produced from Assessments and Obligations, plus the satisfaction anchors defined below. An edge $(u, v)$ means "$u$ constrains $v$".

| Edge | Canon | Working definition: $v$ may start when… | $v$ activated when… |
| --- | --- | --- | --- |
| $E_{req}$ | `requires` | $\sigma(u) = \mathrm{Succeeded}$ | Always |
| $E_{after}$ | `after` | $u$ is terminal (any outcome) | Always |
| $E_{chg}$ | `on_change` | $u$ is terminal | Some `on_change` source of $v$ succeeded and produced a verified change |

**Failure propagation.** If $\sigma(u) \in \{\mathrm{Failed}, \mathrm{TimedOut}, \mathrm{Cancelled}, \mathrm{Rejected}\}$ and $(u, v) \in E_{req}$, then $v$ never becomes ready. This propagates transitively along $E_{req}$.

**Well-formedness.** The whole edge set $E$ must be acyclic. A cycle in any edge kind is a compilation error.

## Satisfaction Anchors

$V$ is produced from Assessments, so a resource whose Assessment is Satisfied produces no Action. Read literally, that leaves `cell-config requires config-directory` with a dangling edge whenever the directory already exists, and the file Action either never starts or starts without its prerequisite being represented at all.

*Working definition.* For every resource that a `requires`, `after`, or `on_change` edge refers to, Warp adds a vertex. If the resource has a Variance or an Obligation, the vertex is its Action. If its Assessment is Satisfied, the vertex is a *satisfaction anchor*: no operation, $\sigma = \mathrm{Succeeded}$ on entry, $\mathrm{changed} = \mathrm{false}$. An anchor is terminal on entry, so an `after` edge from it is met immediately, a `requires` edge from it is met immediately, and an `on_change` edge from it never activates. Anchors mutate nothing, so N3 holds with them present. An anchor for a resource whose Assessment is Indeterminate is not `Succeeded`. It blocks its dependents, because a prerequisite that might be missing is not a prerequisite that is met.

## Cycle Detection and Topological Order

Kahn's algorithm gives both results in one pass.

```text
kahn(G):
    indeg[v] := |{u : (u,v) ∈ E}|   for all v
    Q := { v : indeg[v] = 0 }        # ordered by stable Action ID
    order := []
    while Q ≠ ∅:
        u := pop_min(Q)
        order.push(u)
        for (u,v) ∈ E:
            indeg[v] -= 1
            if indeg[v] = 0: Q.insert(v)
    if |order| < |V|: return Cycle(V \ order)
    return order
```

**Complexity.** $O(|V| + |E|)$ with a FIFO queue. With the ordered queue used for determinism, it is $O(|V|\log|V| + |E|)$.

**Claim 1 (validity).** For every $(u, v) \in E$, $u$ precedes $v$ in `order`.

*Proof.* $v$ enters $Q$ only when $\mathrm{indeg}[v] = 0$. That happens only after every predecessor, including $u$, has been popped and appended. $\square$

**Claim 2 (completeness).** $|order| = |V| \iff G$ is acyclic.

*Proof.* (⇐) Every non-empty DAG has a vertex with in-degree zero. Removing the popped vertices leaves a DAG, so $Q$ is empty only once every vertex is output. (⇒) Suppose a cycle $c_1 \to \dots \to c_m \to c_1$ exists, and let $c_i$ be the first cycle vertex output. Its cycle predecessor must have been output earlier, which is a contradiction. So no cycle vertex is ever output and $|order| < |V|$. $\square$

The remainder $V \setminus order$ contains every cycle, and more: it also contains every vertex downstream of a cycle. For $A \leftrightarrow B$ and $B \to C$, the remainder is $\{A, B, C\}$, and the singleton component $\{C\}$ has no cycle in it. Strongly connected components partition the remainder, and only a component with more than one vertex, or a single vertex with a self-loop, contains a cycle. One component can contain many cycles, and none of them is *minimal* without an objective to minimize. The diagnostic therefore extracts one explicit witness per cyclic component, a closed walk found by depth-first search from any vertex of the component, and reports every acyclic remainder vertex as blocked by the component upstream of it. The test for the diagnostic is that each witness is a cycle in $E$ and that the $A \leftrightarrow B$, $B \to C$ graph yields exactly one witness.

**Claim 3 (determinism, N12).** Ties are broken by a stable, content-derived Action ID. So `order` is a function of $G$ alone and does not depend on hash seeds or insertion order.

## Execution Frontier

Readiness is two questions, answered separately and both required.

$$
\mathrm{Ready} = \{\, v \in \mathrm{Pending} \mid \mathrm{Startable}(v) \wedge \mathrm{Activated}(v) \,\}
$$

**Startable** asks whether $v$'s prerequisites are met and its ordering predecessors have finished, edge by edge:

$$
\mathrm{Startable}(v) \iff
\underbrace{\forall (u,v) \in E_{req}: \sigma(u) = \mathrm{Succeeded}}_{\text{hard prerequisites satisfied}}
\ \wedge\
\underbrace{\forall (u,v) \in E_{after} \cup E_{chg}: \mathrm{terminal}(u)}_{\text{ordering predecessors settled}}
$$

**Activated** asks whether $v$ has a reason to run, as a property of its `on_change` sources taken together:

$$
\mathrm{Activated}(v) \iff
\mathrm{chg}(v) = \varnothing
\ \vee\
\exists (u,v) \in E_{chg}: \sigma(u) = \mathrm{Succeeded} \wedge \mathrm{changed}(u)
$$

Here $\mathrm{chg}(v)$ is the set of `on_change` edges into $v$, and $\mathrm{changed}(u)$ means the verification of $u$ observed a state change. $\mathrm{changed}$ is defined only for $\sigma(u) = \mathrm{Succeeded}$: a failed partial write is terminal and may have altered bytes, but it did not produce the change the edge waits for.

Consider a service refresh with two `on_change` sources, configuration A and configuration B. A succeeded and changed; B succeeded and did not. The refresh is Activated: one changed source is a reason to run. B's unchanged outcome is not a "disabled edge" that vetoes the group. Activation is decided over the group, and the group is Disabled only when every source is terminal and none changed. The earlier table in this document said the dependent runs when every edge is Satisfied, which contradicted the formula for exactly this case, and the formula is the definition.

$\mathrm{Startable}$ alone was the earlier definition of readiness. It let a service restart run after its configuration Action finished without changing anything, which is the one thing `on_change` exists to prevent (counterexample [`activation-missing`](../research/2026-09-28-typed-core/README.md)). $\mathrm{Ready}$ contains an Action only once every hard dependency has succeeded, which gives **N4**.

**Resolution.** Each `requires` and `after` edge resolves on its own; the `on_change` sources of $v$ resolve as a group. The states are exhaustive.

| Unit | State | Meaning |
| --- | --- | --- |
| `requires` edge | Waiting | $u$ is not terminal |
| `requires` edge | Satisfied | $\sigma(u) = \mathrm{Succeeded}$ |
| `requires` edge | Blocked | $u$ ended other than `Succeeded`, or $u$ is an Indeterminate anchor. Propagates |
| `after` edge | Waiting | $u$ is not terminal |
| `after` edge | Satisfied | $u$ is terminal, any outcome |
| `on_change` group | Waiting | Some source is not terminal |
| `on_change` group | Activated | Some source succeeded and changed |
| `on_change` group | Disabled | Every source is terminal and none changed |

$v$ runs when every `requires` and `after` edge is Satisfied and its `on_change` group is Activated or empty. $v$ is Skipped when no edge is Blocked and the group is Disabled: not failed, terminal without change, so its own dependents see it that way. $v$ is Blocked when any `requires` edge is. Spec §62 lists edge semantics as needing an ADR; [ADR 0009](../adr/0009-warp-activation-semantics.md) proposes this table, and milestone 1 pull request 3 produces the truth table that ADR records.

## Conflict Keys

Each Action carries a set of exclusive keys $K(a)$, for example `package-manager:dpkg`, `file:/etc/hosts`, or `systemd:nginx.service`. Independence in the graph does not imply independence in operation:

$$
a \ne b \wedge \mathrm{Reserved}(a) \wedge \mathrm{Reserved}(b) \Rightarrow K(a) \cap K(b) = \varnothing
$$

$\mathrm{Reserved}$ is wider than $\mathrm{Running}$. An Action holds its keys from dispatch until its effect is Settled: through acceptance, execution, and verification, and past a timeout for as long as the outcome stays unknown (N10). An Action that timed out may still be mutating; releasing its keys would let a conflicting Action run beside it.

**Settlement is not satisfaction.** Keys are released when the effect is Settled, its outcome known and no further change possible. Re-observing the Condition does not settle anything: the file holding the desired bytes does not show that an earlier writer has finished, and an active service does not show that an earlier service-manager job has completed or cannot still change the unit. What counts as settlement evidence is resource-specific and belongs to core ([ADR 0006](../adr/0006-kernel-contract.md)): a completed job result for a systemd unit, a completed rename and directory fsync for a file, a platform deadline past which an opaque effect can have no further consequence. Until an effect is Settled its reservation holds, a retry of it is not admitted, and a superseding Plan cannot claim its keys. This is the same obligation fencing has at the effect boundary ([fencing](fencing-and-idempotency.md#fencing)); a lock around the call that starts an effect does not cover the effect.

## Runnable Set

$$
\mathrm{Runnable} = \mathrm{Ready} \cap \mathrm{PolicyAllowed} \cap \mathrm{CapacityAvailable}
$$

Selection is deterministic and greedy:

```text
select(Ready, Reserved, p):
    held := ⋃ K(r) for r ∈ Reserved
    R := []
    for v in Ready ordered by topological index:
        if |Reserved| + |R| ≥ p:            break
        if K(v) ∩ held ≠ ∅:                  continue
        if violates_budget(v, Reserved ∪ R): continue   # max_unavailable, failure domains (N9)
        R.push(v); held := held ∪ K(v)
    return R
```

**Claim (mutual exclusion).** The conflict-key predicate holds after each selection.

*Proof.* By induction. It holds for $\mathrm{Reserved}$ before selection. Each chosen $v$ is disjoint from `held`, which contains the keys of every reserved Action and every Action already chosen. $\square$

The claim holds for one selection over one snapshot. Two selections evaluated concurrently against the same snapshot can each be correct and together over-admit, so reservation updates are serialized: one admission path per Cell, and one per failure domain at Loom.

The greedy pass optimizes nothing. It is safe and deterministic, which is the job. The set it picks is maximal only relative to this snapshot and to constraints that are downward closed, and it says nothing about fairness: a task that keeps losing the tie to new arrivals is a scheduling policy question, not a safety one. Priority schemes such as critical path or fair queuing are later research (spec §22).

## Known Gaps

Assigned in the [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md).

- **Obligations across runs.** Activation is computed from the outcomes of this run. A crash after the configuration file is replaced and before the dependent restart runs leaves the next run seeing a Satisfied file and no pending activation. The Obligation to refresh is recorded durably before the file is replaced, or the service exposes the configuration revision it loaded and that revision is a Condition. Milestone 1 pull request 4, [ADR 0009](../adr/0009-warp-activation-semantics.md), proposed. The same gap is stated from the idempotency side in [fencing-and-idempotency.md](fencing-and-idempotency.md#idempotency).
- **Implicit footprints.** A package install can restart a service and rewrite a configuration file. Conflict keys declared by the Action author cover what the author knew about. The footprint of `package` and `systemd_unit` Actions needs to be derived in core from the resource kind, not typed by hand. Experiment `scheduler-admission`.
