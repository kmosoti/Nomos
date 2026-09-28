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

- **Composition.** Experiment `controller-composition`, after milestone 1 PR 4: two simulated controllers on one property, in shared, single-owner, and disjoint configurations. The shared case must be rejected or detected, and one safe composition demonstrated.
- **Identity.** Experiment `identity-rollback`, before remote execution: crash, disk restore, reinstall, and full clone, each handled separately, with no stale authority accepted.

## Consequences

- Canon admission gains a footprint check, and Conditions gain a footprint field.
- The mesh question, whether overlay identity binds to Cell authorization, is answered by §3: mesh identity is transport identity only, and authority comes from enrollment and the epoch.
- **Revisit trigger.** Reopen if property-level footprints prove too fine to declare by hand, in which case they are derived from the resource kind in core.
