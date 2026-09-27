# ADR 0001: Hexagonal architecture

- Status: Accepted
- Date: 2026-09-27

## Context

Nomos must run the same reconciliation logic against real Linux, a
deterministic mock, and future backends. It must also integrate external
systems (secret stores, network control planes, storage engines, transports)
without pulling those products into its core model (spec §6 Phase 6, §53).

## Decision

Structure the workspace as ports and adapters:

- the domain (`crates/core/`) contains only Nomos semantics;
- each external dependency is reached through a port crate (`crates/ports/`);
- each concrete technology is an adapter crate implementing exactly one port;
- binaries are the composition roots that wire adapters to ports.

The dependency rule is written down in `docs/ARCHITECTURE.md`.

## Consequences

- Trace/Enforce and the property tests (idempotence, Trace purity) can run
  against `nomos-substrate-mock` with no OS access.
- Replacing Vault, Headscale or the storage engine is local to one adapter crate.
- More crates. We accept the extra crates in exchange for boundaries the compiler enforces.
