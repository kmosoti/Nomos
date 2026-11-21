# ADR 0019: WireGuard Mesh and QUIC Control Protocol Adapters

- **Status.** Accepted
- **Date.** 2026-11-21

## Context

Nomos requires a deterministic, pure-IPv6 control mesh connecting Loom and Cell nodes ([ADR 0001](0001-headscale-mesh-adapter.md)) alongside an authenticated Loom-to-Cell control protocol. Deploying directly on heterogeneous dual-stack hosts without strict packet framing causes IPv6 Path Maximum Transmission Unit (PMTU) blackholing, dynamic heap churn on packet RX/TX paths, and implicit trust in ambient public key infrastructure (PKI) roots.

## Decision

### 1. Expand `nomos-mesh` and Implement `nomos-mesh-wireguard`

`crates/ports/nomos-mesh` defines caller-configured mesh topology, interface settings, peer descriptors, `mtu_from_outer_pmtu`, and `MeshPosture` using `nomos_core` identifiers without embedding node names or addresses.

`crates/adapters/nomos-mesh-wireguard` implements a pure-IPv6 WireGuard tunnel engine structured into three strict internal boundaries:

- **Fixed-slab boundary.** Pre-allocated `BufPool` single-packet and Generic Segmentation Offload (GSO) super-buffers, bounded `RingBuf` queues, `KeyTableUpdate` swaps, and `CipherTask` / `DecryptedRx` / `CoreOutput` descriptors enforce bounded memory and fixed 80-byte tunnel overhead:
  - Bytes `[0..16]`: 16-byte WireGuard transport header.
  - Bytes `[16..64]`: 48-byte inner IPv6 plus UDP header.
  - Bytes `[64..len-16]`: QUIC payload capped at `outer_pmtu - 80`.
  - Bytes `[len-16..len]`: 16-byte Poly1305 authentication tag tailroom.
- **Zero-allocation Sans-I/O engine.** `CoreEngine::poll` takes explicit monotonic time `now_ns: u64` and pre-allocated pool handles, driving handshake progression, 2048-bit anti-replay checks, `TimerWheel` timeouts, inner IPv6 header framing, and QUIC PMTU clamping (`64 + quic_payload_len + 16 <= outer_pmtu`) with zero heap allocations on steady-state RX/TX paths.
- **Async shell.** Owns `tokio`, `ArcSwap<KeyTable>`, adaptive ChaCha20-Poly1305 authenticated encryption with associated data (AEAD) execution (inline for batches below 8 frames, `rayon` worker pool for batches of 8 or more), edge decryption and tag verification dropping tampered packets and recycling handles before `CoreEngine`, and `quinn-udp` GSO, Generic Receive Offload (GRO), and Explicit Congestion Notification (ECN) socket I/O with `accept_ra = 2`.

### 2. Expand `nomos-protocol` and Implement `nomos-protocol-quic`

`crates/ports/nomos-protocol` defines typed Loom-to-Cell remote procedure call (RPC) envelopes (`PlanId`, `Generation`, `Digest`), Request for Comments (RFC) 9221 unreliable telemetry datagrams, `QuicStats`, and the `ControlProtocol` port contract.

`crates/adapters/nomos-protocol-quic` implements `nomos-protocol` over `quinn` 0.11 and `rustls` 0.23 using ephemeral or persisted Ed25519 certificates (`rcgen`), strict SHA-256 certificate fingerprint pinning (`ServerCertVerifier` and `ClientCertVerifier` rejecting unpinned peers), length-prefixed JSON control RPC streams, and pure-IPv6 datagram channels.

### 3. Preserve Hexagonal and Workspace Invariants

- Both adapters implement their single matching port (`nomos-mesh` or `nomos-protocol`) and share no cross-adapter dependencies ([ADR 0000](0000-foundations.md)).
- `nomos-mesh-headscale` remains a valid adapter satisfying `nomos-mesh` (`UnsupportedBackend` until Headscale runtime lands).
- Workspace `#![forbid(unsafe_code)]` holds across all implementation and test files, including `stats_alloc`-backed zero-allocation verification.

## Consequences

- Steady-state tunnel polling is deterministic over `(now_ns, rx_batch)` and executes zero heap allocations.
- MTU sweep tests prove `64 + quic_payload_len + 16 <= outer_pmtu` across standard and jumbo paths.
- Unpinned peer certificates fail TLS verification before any RPC frame or datagram reaches `nomos-protocol`.
