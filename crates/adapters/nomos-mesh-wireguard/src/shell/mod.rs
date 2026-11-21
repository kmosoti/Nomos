//! Async shell owning Tokio UDP/ECN/GSO I/O, `ArcSwap<KeyTable>`, and adaptive ChaCha20-Poly1305.

pub mod adapter;
pub mod crypto;
pub mod udp_io;

pub use adapter::WireGuardMeshAdapter;
pub use crypto::{
    AdaptiveCipher, CryptoDispatchMode, DEFAULT_RAYON_THRESHOLD, PARALLEL_CRYPTO_MIN_BYTES,
    PARALLEL_CRYPTO_THRESHOLD, dispatch_mode_for_batch,
};
pub use udp_io::{KeyStore, UdpShell};
