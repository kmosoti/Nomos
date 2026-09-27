# Nomos

*νόμος: law, custom, established order.*

Every machine in a fleet drifts. Someone edits a config by hand, a package
updates, a service stops and nobody notices, a permission changes. Most tools
respond by running scripts at machines and hoping the exit codes mean what
they seem to mean.

Nomos takes a different stance. You describe how your machines **should** be.
We call that description the **Canon**. Nomos then keeps asking one question:
*how are they actually?*

It observes each host and measures the **Variance** between reality and the
Canon. From that, it works out the smallest safe set of changes. It applies
them, then looks again to confirm the change really happened. Every step is
written down as immutable history.

> Observe reality, compare it with Canon, derive the smallest valid change,
> apply that change under explicit safety constraints, verify reality again,
> and preserve what happened as immutable history.

## The cast

- **Cell** lives on each machine. It observes, reconciles and verifies, and it
  keeps working when it is cut off from everything else.
- **Loom** coordinates the fleet. It decides which machines a change applies
  to, plans the rollout, and makes sure a change never takes down more than
  you allowed.
- **Warp** turns desired changes into a dependency graph of Actions, so things
  happen in the right order and never twice at once.
- **Substrate** is where Nomos meets the operating system. It uses native
  Linux interfaces rather than scraped shell output.

**Trace** shows you what would change without touching anything. **Enforce**
makes it so. Running Enforce a second time does nothing, because there is
nothing left to do.

## What Nomos is not

Nomos is not a container orchestrator, a secrets manager, a telemetry
platform, or a way to broadcast shell commands. It solves one problem well:

*Reliably describe, inspect, change and verify Linux host state, locally and
across a fleet.*

Nomos is part of **Moiric**. FabricO11y observes systems, Nomos controls them,
and Metron bounds what computation may do.

## Status

Nomos is at the skeleton stage. The architecture and crate boundaries are in
place. The implementation starts with Phase 0: deterministic Canon → Variance
→ Plan against a simulated machine.

## Learn more

Everything technical lives in [`docs/`](docs/):

- [Project specification](docs/PROJECT-SPEC.md): the model, semantics and safety invariants
- [Architecture](docs/architecture/): how the system and the code are organised
- [Formal](docs/formal/): the algorithms and the invariants Nomos must never break
- [Decision records](docs/adr/): why it is organised that way
