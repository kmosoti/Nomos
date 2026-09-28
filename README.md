# Nomos

*νόμος: law, custom, established order.*

Every machine in a fleet drifts. Someone edits a config by hand. A package updates. A service stops and nobody notices. A permission changes. Most tools respond by firing scripts at machines and trusting that exit code zero means what it seems to mean.

Nomos asks a different question. You describe how your machines **should** be. That description is the **Canon**. Nomos then keeps asking: *how are they actually?*

It observes each host and measures the **Variance** between reality and the Canon. It works out a deterministic set of required changes that is valid under explicit safety constraints, applies them, and looks again to confirm that reality moved. Every step is written down as immutable history.

What does "done" mean here? The intended postcondition was observed on the machine. An exit code is a rumor.

> Observe reality, compare it with Canon, derive a deterministic, valid set of required changes, apply those changes under explicit safety constraints, verify reality again, and preserve what happened as immutable history.

## The Cast

- **Cell** lives on each machine. It observes, reconciles, and verifies, and it keeps working when cut off from everything else.
- **Loom** coordinates the fleet. It decides which machines a change applies to, plans the rollout, and never takes down more than you allowed.
- **Warp** turns desired changes into a dependency graph of Actions, so things happen in the right order and never collide.
- **Substrate** is where Nomos meets the operating system. It speaks native Linux interfaces, not scraped shell output.

**Trace** shows what would change without touching anything. **Enforce** makes it so. Run Enforce twice and the second run does nothing, because there is nothing left to do. That is the point.

## What Nomos Is Not

Nomos is not a container orchestrator, a secrets manager, a telemetry platform, or a way to broadcast shell commands. It solves one problem, and solves it properly:

*Reliably describe, inspect, change, and verify Linux host state, locally and across a fleet.*

That problem is hard enough without trying to colonize computing.

Nomos is part of **Moiric**. FabricO11y observes systems, Nomos controls them, and Metron bounds what computation may do.

## Status

Skeleton. The architecture and crate boundaries are in place, and the code is still empty. Phase 0 comes next: a deterministic Canon → Variance → Plan pipeline against a simulated machine.

## Learn More

Everything technical lives in [`docs/`](docs/):

- [Project specification](docs/PROJECT-SPEC.md): the model, semantics, and safety invariants
- [Architecture](docs/architecture/): how the system and the code are organized
- [Formal](docs/formal/): the algorithms, and the invariants Nomos must never break
- [Decision records](docs/adr/): why it is organized that way

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md). Report security issues privately, as described in [SECURITY.md](SECURITY.md).

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
