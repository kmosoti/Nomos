# ADR 0001: Headscale as the Preliminary Mesh Adapter

- **Status.** Accepted
- **Date.** 2026-09-27

## Context

Cells open outbound connections to Loom (spec §30). That has to work through NAT, host firewalls, and cloud and enterprise networks, and it needs verifiable node identity (spec §32). Writing a new overlay network is not on the roadmap.

## Decision

Add a `nomos-mesh` port covering node enrollment, node identity, reachability, and revocation. Implement it first as `nomos-mesh-headscale`, backed by a self-hosted Headscale control server (Tailscale-compatible).

What is the primitive underneath? WireGuard tunnels between hosts, plus a coordination server that hands out WireGuard public keys, addresses, and access rules. Headscale is that coordination server. It never carries Loom ↔ Cell traffic; the tunnels do. The port is shaped around the primitive (enroll, identify, reach, revoke), not around Headscale's API, which keeps the adapter replaceable.

## Consequences

- Cell enrollment maps to Headscale pre-auth keys. Revocation maps to node expiry or deletion.
- Mesh identity is transport-level only. Nomos keeps its own authorization, fencing, and protocol-level authentication (spec §33, §36).
- A SPIFFE-style or plain mutual TLS (mTLS) adapter can replace Headscale later without touching the application layer.
- **Failure behavior.** If Headscale disappears, new enrollment and revocation stop. Established tunnels should keep working, because Tailscale-compatible clients keep their last network map. That last claim is from Tailscale's documented client behavior, not from our own testing yet. The failure demonstration (spec §57) should confirm it.
- **Open question.** Tailnet membership proves a host is on the network, not that it may act. Spec §62 tracks how mesh identity binds to Cell authorization.
