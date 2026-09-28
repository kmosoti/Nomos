# ADR 0001: Headscale as the preliminary Mesh adapter

- **Status.** Accepted
- **Date.** 2026-09-27

## Context

Cells open outbound connections to Loom (spec §30). That has to work through NAT, host firewalls, and cloud and enterprise networks, and it needs verifiable node identity (spec §32). Writing a new overlay network is not on the roadmap.

## Decision

Add a `nomos-mesh` port covering node enrollment, node identity, reachability, and revocation. Implement it first as `nomos-mesh-headscale`, backed by a self-hosted Headscale control server (Tailscale-compatible).

## Consequences

- Cell enrollment maps to Headscale pre-auth keys. Revocation maps to node expiry or deletion.
- Mesh identity is transport-level only. Nomos keeps its own authorization, fencing, and protocol-level authentication (spec §33, §36).
- A SPIFFE-style or plain mTLS adapter can replace Headscale later without touching the application layer.
