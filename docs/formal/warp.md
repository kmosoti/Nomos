# Warp

## Graph model

$$
G = (V, E), \qquad E = E_{req} \cup E_{after} \cup E_{chg}
$$

$V$ is the set of Actions produced from Variance. An edge $(u, v)$ means
"$u$ constrains $v$".

| Edge | Canon | Working definition: $v$ may start when… | $v$ activated when… |
|---|---|---|---|
| $E_{req}$ | `requires` | $\sigma(u) = \mathrm{Succeeded}$ | always |
| $E_{after}$ | `after` | $u$ is terminal (any outcome) | always |
| $E_{chg}$ | `on_change` | $u$ is terminal | $u$ produced a state change |

**Failure propagation.** If $\sigma(u) \in \{\mathrm{Failed}, \mathrm{TimedOut},
\mathrm{Cancelled}, \mathrm{Rejected}\}$ and $(u, v) \in E_{req}$, then $v$
never becomes ready. This propagates transitively along $E_{req}$.

**Well-formedness.** The whole edge set $E$ must be acyclic. A cycle in any
edge kind is a compilation error.

## Cycle detection and topological order

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

**Complexity.** $O(|V| + |E|)$ with a FIFO queue. With the ordered queue used
for determinism, it is $O(|V|\log|V| + |E|)$.

**Claim 1 (validity).** For every $(u, v) \in E$, $u$ precedes $v$ in `order`.

*Proof.* $v$ enters $Q$ only when $\mathrm{indeg}[v] = 0$. That happens only
after every predecessor, including $u$, has been popped and appended. $\square$

**Claim 2 (completeness).** $|order| = |V| \iff G$ is acyclic.

*Proof.* (⇐) Every non-empty DAG has a vertex with in-degree zero. Removing
the popped vertices leaves a DAG, so $Q$ is empty only once every vertex is
output. (⇒) Suppose a cycle $c_1 \to \dots \to c_m \to c_1$ exists, and let
$c_i$ be the first cycle vertex output. Its cycle predecessor must have been
output earlier, which is a contradiction. So no cycle vertex is ever output
and $|order| < |V|$. $\square$

The remainder $V \setminus order$ contains every cycle. Strongly connected
components of that remainder give the minimal cycles to report to the user.

**Claim 3 (determinism, N12).** Ties are broken by a stable, content-derived
Action ID. So `order` is a function of $G$ alone and does not depend on hash
seeds or insertion order.

## Execution frontier

At runtime:

$$
\mathrm{Ready} = \{\, v \in \mathrm{Pending} \mid
\forall (u,v) \in E_{req}: \sigma(u) = \mathrm{Succeeded}
\ \wedge\
\forall (u,v) \in E_{after} \cup E_{chg}: \mathrm{terminal}(u) \,\}
$$

$\mathrm{Ready}$ contains an action only once every hard dependency has
succeeded, which gives **N4**.

## Conflict keys

Each Action carries a set of exclusive keys $K(a)$, for example
`package-manager:dpkg`, `file:/etc/hosts` or `systemd:nginx.service`.
Independence in the graph does not imply independence in operation:

$$
a \ne b \wedge \mathrm{Running}(a) \wedge \mathrm{Running}(b) \Rightarrow K(a) \cap K(b) = \varnothing
$$

## Runnable set

$$
\mathrm{Runnable} = \mathrm{Ready} \cap \mathrm{PolicyAllowed} \cap \mathrm{CapacityAvailable}
$$

Selection is deterministic and greedy:

```text
select(Ready, Running, p):
    held := ⋃ K(r) for r ∈ Running
    R := []
    for v in Ready ordered by topological index:
        if |Running| + |R| ≥ p:             break
        if K(v) ∩ held ≠ ∅:                  continue
        if violates_budget(v, Running ∪ R):  continue   # max_unavailable, failure domains (N9)
        R.push(v); held := held ∪ K(v)
    return R
```

**Claim (mutual exclusion).** The conflict-key predicate holds after each
selection.

*Proof.* By induction. It holds for $\mathrm{Running}$ before selection.
Each chosen $v$ is disjoint from `held`, which contains the keys of every
running Action and every Action already chosen. $\square$

The greedy pass maximises nothing. It is only safe and deterministic.
Priority schemes such as critical path or fair queuing are later research
(spec §22).
