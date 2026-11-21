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

Alpha. `0.1.0-alpha.2` is the released masterless Cell (`nomos-cell`): one Debian host, a Canon compiled from Rust, and a timer that converges the host toward it and records what it did on Debian 12 and 13, described in the [release notes](docs/releases/v0.1.0-alpha.2.md).

The `v0.2.0-alpha.1` development line extends the workspace toward coordinated fleets with headless adapters, resource governance, and NixOS modules:

- **Age and vTPM 2.0 secrets (`nomos-cipher-age-vtpm`):** Data-driven `SecretPolicy` and `NodeTarget` synchronization, `systemd-creds` Virtual Trusted Platform Module (vTPM) 2.0 verification, and metadata-only `SecretsPosture`.
- **WireGuard mesh and pinned QUIC (`nomos-mesh-wireguard`, `nomos-protocol-quic`):** Pure-IPv6 zero-allocation tunnel data plane and SHA-256 fingerprint-pinned QUIC Remote Procedure Call (RPC) and datagram transport.
- **Google Cloud substrate and pure governor (`nomos-cloud-gcp`, `nomos-core::governor`):** Pure-IPv6 Google Cloud Platform (GCP) Compute Engine and Spot Batch adapter paired with a `#![no_std]` Pressure Stall Information (PSI) and control group resource governor (`ServiceGovernor`).
- **Headless coordinator (`nomos-loom`):** Pure-IPv6 HTTP and QUIC daemon (`compile`, `trace`, `enforce`, `serve`) with an SQLite Write-Ahead Log (WAL) credential lease broker, `/v1/governor` and `/v1/telemetry` endpoints, optional `--web-assets-dir` serving, and `nomos-loom_*_amd64.deb` packaging.
- **Reusable NixOS modules (`nix/modules/`):** Flake-exported `services.nomos` modules (`base-ipv6`, `wireguard-wg0`, `secrets`, and `deadman-rollback`) using RFC 3849 and RFC 4193 IPv6 addressing defaults.

## Learn More

Everything technical lives in [`docs/`](docs/):

- [Operator guide](docs/operator-guide.md): install the Cell or headless Loom, configure NixOS modules, author a Canon, and run it
- [Project specification](docs/PROJECT-SPEC.md): the model, semantics, and safety invariants
- [Architecture](docs/architecture/): how the system and the code are organized
- [Formal](docs/formal/): the algorithms, and the invariants Nomos must never break
- [Decision records](docs/adr/): why it is organized that way

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md). Report security issues privately, as described in [SECURITY.md](SECURITY.md).

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
