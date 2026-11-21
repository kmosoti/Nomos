//! Pure-IPv6 WireGuard mesh adapter for [`nomos_mesh`] ([ADR 0019](../../../docs/adr/0019-wireguard-mesh-and-quic-protocol.md)).
//!
//! Architecture follows the strict three-tier boundary:
//! - [`boundary`]: fixed-slab packet [`BufPool`], descriptors, zeroized key tables, and bounded ring queues.
//! - [`engine`]: deterministic, zero-allocation Sans-I/O tunnel core (`CoreEngine::poll`).
//! - [`shell`]: async Tokio + `quinn-udp` GSO/GRO/ECN I/O, `ArcSwap<KeyTable>`, and adaptive ChaCha20-Poly1305 AEAD.

pub mod boundary;
pub mod engine;
pub mod shell;

pub use boundary::{
    BufHandle, BufPool, CipherDirection, CipherTask, CoreOutput, DecryptedRx, EcnCodepoint,
    FixedRing, GSO_SUPER_BYTES, KeyTable, KeyTableUpdate, PacketLayout, SLOT_BYTES, SessionKeys,
    SliceWriter,
};
pub use engine::{CoreEngine, InnerEndpoint, InnerIpv6UdpHeader, ReplayWindow, TunnelQuicBudget};
pub use shell::{
    AdaptiveCipher, DEFAULT_RAYON_THRESHOLD, KeyStore, UdpShell, WireGuardMeshAdapter,
};
