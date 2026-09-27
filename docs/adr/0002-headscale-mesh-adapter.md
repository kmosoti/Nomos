# ADR 0002: Headscale as the preliminary Mesh adapter

- Status: Accepted
- Date: 2026-09-27

## Context

Cells initiate outbound connections to Loom (spec §30). The spec needs this to
work across NAT, host firewalls and cloud or enterprise networks, and it needs
verifiable node identity (spec §32). Nomos should not build its own overlay
network.

## Decision

Introduce a `nomos-mesh` port covering node enrollment, node identity,
reachability and revocation. Implement it first with `nomos-mesh-headscale`,
backed by a self-hosted Headscale control server (Tailscale-compatible).

## Consequences

- Cell enrollment maps onto Headscale pre-auth keys. Revocation maps onto node expiry/deletion.
- Mesh identity is transport-level only. Nomos keeps its own authorization,
  fencing and protocol-level authentication (spec §33, §36).
- A SPIFFE-style or plain mTLS adapter can replace Headscale later without changing the application layer.
