# Warp

## Graph Model

$$
G = (V, E), \qquad E = E_{req} \cup E_{after} \cup E_{chg}
$$

$G$ is a directed acyclic graph (DAG). $V$ is the set of Actions produced from Variance, plus the satisfaction anchors defined below. An edge $(u, v)$ means "$u$ constrains $v$".

| Edge | Canon | Working definition: $v$ may start when… | $v$ activated when… |
| --- | --- | --- | --- |
| $E_{req}$ | `requires` | $\sigma(u) = \mathrm{Succeeded}$ | Always |
| $E_{after}$ | `after` | $u$ is terminal (any outcome) | Always |
| $E_{chg}$ | `on_change` | $u$ is terminal | $u$ succeeded and produced a verified state change |

**Failure propagation.** If $\sigma(u) \in \{\mathrm{Failed}, \mathrm{TimedOut}, \mathrm{Cancelled}, \mathrm{Rejected}\}$ and $(u, v) \in E_{req}$, then $v$ never becomes ready. This propagates transitively along $E_{req}$.

**Well-formedness.** The whole edge set $E$ must be acyclic. A cycle in any edge kind is a compilation error.

## Satisfaction Anchors

$V$ is produced from Variance, so a resource whose Variance is $\varnothing$ produces no Action. Read literally, that leaves `cell-config requires config-directory` with a dangling edge whenever the directory already exists, and the file Action either never starts or starts without its prerequisite being represented at all.

*Working definition.* For every resource that a `requires` or `on_change` edge refers to, Warp adds a vertex. If the resource has Variance, the vertex is its Action. If it does not, the vertex is a *satisfaction anchor*: no operation, $\sigma = \mathrm{Succeeded}$ on entry, $\mathrm{changed} = \mathrm{false}$. Anchors mutate nothing, so N3 holds with them present. An anchor for a resource whose observation supported neither satisfaction nor Variance (see [reconciliation, Known Gaps](reconciliation.md#known-gaps)) is not `Succeeded`. It blocks its dependents, because a prerequisite that might be missing is not a prerequisite that is met.

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

The remainder $V \setminus order$ contains every cycle. Strongly connected components partition it, but a component is not a cycle: one component can contain many, and none of them is *minimal* without an objective to minimize. The diagnostic therefore extracts one explicit witness per component, a closed walk found by depth-first search from any vertex of the component, and reports that. The test for the diagnostic is that the witness is a cycle in $E$, not that it is short.

**Claim 3 (determinism, N12).** Ties are broken by a stable, content-derived Action ID. So `order` is a function of $G$ alone and does not depend on hash seeds or insertion order.

## Execution Frontier

At runtime, a pending vertex is ready when it may start and it is activated:

$$
\mathrm{Ready} = \{\, v \in \mathrm{Pending} \mid \mathrm{Startable}(v) \wedge \mathrm{Activated}(v) \,\}
$$

$$
\mathrm{Startable}(v) \iff
\forall (u,v) \in E_{req}: \sigma(u) = \mathrm{Succeeded}
\ \wedge\
\forall (u,v) \in E_{after} \cup E_{chg}: \mathrm{terminal}(u)
$$

$$
\mathrm{Activated}(v) \iff
\mathrm{chg}(v) = \varnothing
\ \vee\
\exists (u,v) \in E_{chg}: \sigma(u) = \mathrm{Succeeded} \wedge \mathrm{changed}(u)
$$

Here $\mathrm{chg}(v)$ is the set of `on_change` edges into $v$, and $\mathrm{changed}(u)$ means the verification of $u$ observed a state change. $\mathrm{changed}$ is defined only for $\sigma(u) = \mathrm{Succeeded}$: a failed partial write is terminal and may have altered bytes, but it did not produce the change the edge waits for.

$\mathrm{Startable}$ alone was the earlier definition. It let a service restart run after its configuration Action finished without changing anything, which is the one thing `on_change` exists to prevent (counterexample [`activation-missing`](../research/2026-09-28-typed-core/README.md)). $\mathrm{Ready}$ contains an Action only once every hard dependency has succeeded, which gives **N4**.

**Dependency resolution.** Each incoming edge of $v$ resolves to one state, and the states are exhaustive.

| State | Meaning | $v$ |
| --- | --- | --- |
| Waiting | $u$ is not terminal | Stays pending |
| Satisfied | The start condition holds and, for $E_{chg}$, $u$ changed | Runs when every edge is Satisfied |
| Disabled | Every `on_change` source is terminal and none changed | Skipped. Not failed, and its own dependents see it as terminal without change |
| Blocked | A `requires` source did not succeed, or an anchor is not `Succeeded` | Never runs. Failure propagation |

*Working definition* for several `on_change` sources: *any* one changed source activates $v$. The alternative, *all*, has no use case yet. Spec §62 already lists edge semantics as needing an ADR (candidate `warp-gates`); experiment `warp-truth-table` in the [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md) produces the table that ADR records.

## Conflict Keys

Each Action carries a set of exclusive keys $K(a)$, for example `package-manager:dpkg`, `file:/etc/hosts`, or `systemd:nginx.service`. Independence in the graph does not imply independence in operation:

$$
a \ne b \wedge \mathrm{Reserved}(a) \wedge \mathrm{Reserved}(b) \Rightarrow K(a) \cap K(b) = \varnothing
$$

$\mathrm{Reserved}$ is wider than $\mathrm{Running}$. An Action holds its keys from dispatch until its outcome is known: through acceptance, execution, and verification, and past a timeout for as long as the outcome stays unknown (N10). An Action that timed out may still be mutating; releasing its keys would let a conflicting Action run beside it. Keys are released on a terminal outcome that is known, or when re-observation settles an unknown one. *Working definition;* experiment `scheduler-admission`.

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

The greedy pass optimizes nothing. It is safe and deterministic, which is the job. The set it picks is maximal only relative to this snapshot and to constraints that are downward closed, and it says nothing about fairness: a task that keeps losing the tie to new arrivals is a scheduling policy question, not a safety one. Priority schemes such as critical path or fair queuing are later research (spec §22).

## Known Gaps

Identified by the 2026-09-28 research snapshot ([evaluation](../research/2026-09-28-typed-core/README.md)) and assigned in the [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md).

- **Durable change consumption.** Activation is computed from the outcomes of this run. A crash after the configuration file is replaced and before the dependent restart runs leaves the next run seeing no file Variance and no pending activation; the restart is lost. Either the obligation is persisted before the file is replaced, or the service exposes the configuration revision it loaded and that revision is part of its observation. Experiment `refresh-recovery`, ADR candidate `warp-gates`. The same gap is stated from the idempotency side in [fencing-and-idempotency.md](fencing-and-idempotency.md#idempotency).
- **Implicit footprints.** A package install can restart a service and rewrite a configuration file. Conflict keys declared by the Action author cover what the author knew about. The footprint of `package` and `systemd_unit` Actions needs to be derived from the driver, not typed by hand. Experiment `scheduler-admission`.
