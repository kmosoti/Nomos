# ADR 0008: Resource Ownership, Controller Composition, and Identity

- **Status.** Proposed
- **Date.** 2026-09-28
- **Candidate.** `evidence-model` in the 2026-09-28 research snapshot (recommendations `composition`, `identity-recovery`). Its third recommendation, `evidence-assessment`, is accepted in [ADR 0005](0005-assessment-vocabulary.md).

## Context

The reconciliation model has one controller. Two controllers that each converge alone can oscillate together: Nomos writes a sysctl, a tuning daemon writes it back, and each is correct by its own lights (finding `single-controller`). Anvil and Welder handle this for Kubernetes controllers by stating what each controller owns, what it guarantees, and what it relies on others not to do. Host resources are not Kubernetes objects, so the discipline transfers and the object model does not.

Identity has the same shape of problem. A path is not an authorization (finding `identity-path`), and a counter restored from a disk snapshot looks exactly like a counter that was never rolled back (finding `epoch-not-oracle`). Research open questions `resource-identity`, `rollback`, and `mesh` all end here.

## Decision

Proposed. The formal documents follow it as a working definition until it is accepted.

### 1. Every Managed Property Has One Owner

Each Condition declares a footprint: the resource properties it reads and the ones it writes, at property granularity, for example `sysctl:vm.swappiness` or `file:/etc/example.conf#content`. Within one Cell, a property is written by at most one Condition. A Canon whose Conditions overlap on a written property is rejected at admission, unless it names an explicit conflict policy for that property.

### 2. Foreign Writers Are Assumptions, Stated

A Condition may declare what it relies on others not to change. A property with a known foreign writer is either excluded from the footprint or marked contested, and a contested property that changes against the Condition is reported as interference, not retried until the bound. The oscillation diagnostic of [reconciliation](../formal/reconciliation.md#oscillation) is the detector for writers nobody declared.

### 3. Identities Are Separate Values

| Identity | Meaning | Source of truth |
| --- | --- | --- |
| Resource key | The logical resource a Condition names | Canon |
| OS identity | What the kernel or service manager calls it: path plus device and inode, unit name, user ID | Substrate, resolved at use |
| NodeID | The Cell's logical identity | Enrollment |
| Enrollment credential | What proves the NodeID | The identity provider |
| Writer incarnation | One run of one Event writer | Minted at start |
| Authority epoch | Which authority may issue Plans | Loom, under an external anchor |

A path string is a resource key, never an authorization. Resolution to an OS identity happens at use, under the boundary rules of [ADR 0013](0013-trust-boundaries.md).

### 4. Rollback Needs Evidence From Outside

Recovery after a crash reuses the persisted incarnation and sequence. Recovery after a disk restore, a reinstall, or a virtual-machine clone cannot be distinguished from a crash using only restored state, so it requires authorized re-enrollment or a monotonic anchor held outside the Cell. Random incarnation values give uniqueness, not order.

### Alternatives

- **Last writer wins.** Rejected: it converts a policy conflict into an oscillation.
- **Resource-level ownership.** Rejected as too coarse: two Conditions can legitimately own different properties of one file's metadata.
- **Lexicographic epochs from UUIDs.** Rejected: ordering random values says nothing about authority.

### Acceptance Criteria

- **Composition.** Experiment `controller-composition`, after milestone `05-transition-kernel`: two simulated controllers on one property, in shared, single-owner, and disjoint configurations. The shared case must be rejected or detected, and one safe composition demonstrated.
- **Identity.** Experiment `identity-rollback`, before remote execution: crash, disk restore, reinstall, and full clone, each handled separately, with no stale authority accepted.

## Note, 2026-09-29: Working Definitions for Controller Composition

Experiment `controller-composition` needs definitions that §1 and §2 leave open. It takes these, written before the code. The ADR stays Proposed.

- **Controller.** Anything that writes host properties toward its own intent. In the experiment, a controller is one transition kernel with its own Canon, Event Log, and fence, and every controller writes the same mock host.
- **Property.** Text of the form `<kind>:<name>`, such as `file:/etc/example.conf#content` or `sysctl:vm.swappiness`, compared as text. A property names a resource key, not an OS identity (§3), so two keys that reach one inode or one service are two properties to the check.
- **Footprint.** Three sets of properties per controller. The *guarantee* is what it writes: it writes no property outside the set. The *reliance* is what it relies on no other controller writing. The *reads* are what it observes and tolerates changing under another controller's guarantee.
- **A write is relied on.** A controller that converges alone on a property converges because it is that property's only writer, so its reliance always includes its guarantee. The check leans on this definition, and semantic mutant `SM-COMPOSE-001` removes it.
- **The composition check.** Footprints compose when their controllers are distinct and, for every pair of distinct controllers $i$ and $j$, $W_j \cap R_i = \varnothing$, where $W$ is a guarantee and $R$ a reliance. Otherwise the check names every interference, in an order that does not depend on the order of the footprints: a *shared write* when both controllers write the property, a *violated reliance* when one relies on a property only the other writes, and a *duplicate controller* when two footprints carry one name.
- **Configurations.** *Disjoint*: no property appears in two footprints. *Single-owner*: every property has at most one writer, and any other controller at most reads it. *Shared*: some property has two writers.
- **Not modeled.** §1's explicit conflict policy and §2's contested properties. No composition is accepted on a policy, and the check knows no contested category. The check is a pure function in the `footprint` module of `nomos-core`; wiring it into Canon admission, and deriving footprints from resource kinds, wait for acceptance.
- **The invariant it serves.** N2: a successful Enforce leaves the host Converged toward its Canon. A run's Converged is sound at its last Observation, and under a shared write the next controller's run makes it false again. The check is the working precondition under which N2 keeps its meaning when more than one controller writes a host.

## Note, 2026-09-29: Evidence From `controller-composition`

The composition criterion ran, on the mock host and a two-kernel harness ([record](../research/2026-09-28-typed-core/results/controller-composition.md)):

- **The counterexample is detected.** The check rejects two controllers that write one file with a shared write, before anything executes. Run without the check, the pair oscillates on the mock: each sweep rewrites the file twice, and a projection over both Canons repeats at the second sweep. Semantic mutant `SM-COMPOSE-001`, which drops the guarantee from the reliance, is caught by that test.
- **Two safe compositions are demonstrated.** Disjoint and single-owner pairs are accepted and reach a joint fixed point, in either order and at 25 start offsets.
- **§2's detector misses a writer that takes turns.** Every run of the oscillating pair ends Converged, and across 25 offsets no run ends NonConvergent. The per-run oscillation diagnostic cannot see a writer that acts between runs; seeing one needs fingerprint history across runs of the same Canon, which nothing implements.
- **The check is as good as its footprints.** A write left out of a footprint passes the check, and only an audit of executions against guarantees sees it. A declared reliance rejects a pair that converges without the check.

The ADR stays Proposed. Nothing ran on a real host, no sysctl was modeled, and the check is not wired into Canon admission. The identity criterion (`identity-rollback`) has not run. Acceptance is the owner's.

## Consequences

- Canon admission gains a footprint check, and Conditions gain a footprint field.
- The mesh question, whether overlay identity binds to Cell authorization, is answered by §3: mesh identity is transport identity only, and authority comes from enrollment and the epoch.
- **Revisit trigger.** Reopen if property-level footprints prove too fine to declare by hand, in which case they are derived from the resource kind in core.
