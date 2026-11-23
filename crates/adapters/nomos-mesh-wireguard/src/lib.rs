//! Pure-IPv6 WireGuard mesh adapter for [`nomos_mesh`] ([ADR 0019](../../../docs/adr/0019-wireguard-mesh-and-quic-protocol.md)).
//!
//! Architecture follows the strict three-tier boundary:
//! - [`boundary`]: fixed-slab jumbo packet [`BufPool`], descriptors, zeroized cached-AEAD key tables, and bounded ring queues.
//! - [`engine`]: deterministic, zero-allocation Sans-I/O tunnel core (`CoreEngine::poll`) and Noise_IKpsk2 handshake state machine.
//! - [`shell`]: async Tokio + `IPV6_V6ONLY` `quinn-udp` GSO/GRO/ECN I/O, `ArcSwap<KeyTable>`, and adaptive ChaCha20-Poly1305 AEAD.

pub mod boundary;
pub mod engine;
pub mod shell;

pub use boundary::{
    BufHandle, BufPool, CipherDirection, CipherTask, CoreOutput, DecryptedRx, EcnCodepoint,
    FixedRing, GSO_SUPER_BYTES, KeyTable, KeyTableUpdate, MAX_BATCH_SPLIT, PacketLayout,
    SLOT_BYTES, SessionKeys, SliceWriter,
};
pub use engine::{
    COOKIE_REPLY_LEN, CookieGenerator, CoreEngine, DEFAULT_REKEY_AFTER_BYTES, INITIATION_LEN,
    InitiatorHandshake, InnerDestinationPolicy, InnerEndpoint, InnerIpv6UdpHeader,
    MSG_TYPE_COOKIE_REPLY, MSG_TYPE_INITIATION, MSG_TYPE_RESPONSE, REJECT_AFTER_TIME_NS,
    REKEY_AFTER_TIME_NS, RESPONSE_LEN, ReplayWindow, ResponderOutcome, StaticKeypair, TimerAction,
    TimerEntry, TimerWheel, TunnelQuicBudget, consume_cookie_reply, initiate_handshake,
    ipv6_in_prefix, respond_handshake,
};
pub use shell::{
    AdaptiveCipher, CryptoDispatchMode, DEFAULT_RAYON_THRESHOLD, KeyStore,
    PARALLEL_CRYPTO_MIN_BYTES, PARALLEL_CRYPTO_THRESHOLD, UdpShell, WireGuardMeshAdapter,
    dispatch_mode_for_batch,
};
