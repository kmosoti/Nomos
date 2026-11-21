//! Async shell owning Tokio UDP/ECN/GSO I/O, `ArcSwap<KeyTable>`, and adaptive ChaCha20-Poly1305.

pub mod crypto;
pub mod udp_io;

pub use crypto::{AdaptiveCipher, DEFAULT_RAYON_THRESHOLD};
pub use udp_io::{KeyStore, UdpShell, WireGuardMeshAdapter};
