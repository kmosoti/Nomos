//! Mesh port: node identity, pure-IPv6 tunnel parameters, and peer reachability.

use core::fmt;
use nomos_core::plan::Generation;
use nomos_core::resource::Digest;
use serde::{Deserialize, Serialize};

/// WireGuard transport header size (`[0..16]`).
pub const WG_HEADER_LEN: usize = 16;

/// Inner IPv6 header (40B) plus UDP header (8B) size (`[16..64]`).
pub const INNER_IPV6_UDP_LEN: usize = 48;

/// Offset where encapsulated QUIC payload starts (`[64..len-16]`).
pub const INNER_OFFSET: usize = WG_HEADER_LEN + INNER_IPV6_UDP_LEN;

/// Trailing Poly1305 AEAD authentication tag size (`[len-16..len]`).
pub const AEAD_TAG_LEN: usize = 16;

/// Total tunnel framing overhead across header, inner IPv6+UDP, and AEAD tag.
pub const TUNNEL_OVERHEAD: usize = INNER_OFFSET + AEAD_TAG_LEN;

/// Minimum IPv6 link path MTU per RFC 8200.
pub const MIN_IPV6_PMTU: u16 = 1280;

/// Computes maximum encapsulated QUIC payload length (`outer_pmtu - 80`).
pub const fn max_quic_payload(outer_pmtu: u16) -> u16 {
    outer_pmtu.saturating_sub(TUNNEL_OVERHEAD as u16)
}

/// Validates outer IPv6 PMTU and derives safe WireGuard `wg0` MTU (`outer_pmtu - 80`).
pub fn tunnel_mtu_from_outer_pmtu(outer_pmtu: u16) -> Result<u16, MeshError> {
    if outer_pmtu < MIN_IPV6_PMTU {
        return Err(MeshError::InvalidConfig(format!(
            "outer_pmtu {outer_pmtu} is below IPv6 minimum {MIN_IPV6_PMTU}"
        )));
    }
    Ok(max_quic_payload(outer_pmtu))
}

/// Result alias for mesh port operations.
pub type MeshResult<T> = Result<T, MeshError>;

/// Errors returned by mesh configuration, framing, and transport adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// Invalid interface, peer, address, or MTU parameter.
    InvalidConfig(String),
    /// Cryptographic key, handshake, or AEAD verification failure.
    Crypto(String),
    /// Underlying socket or kernel netlink/driver transport failure.
    Transport(String),
    /// Received packet counter failed sliding-window anti-replay check.
    ReplayDetected(u64),
    /// Fixed-capacity buffer pool or ring queue is exhausted.
    PoolExhausted,
    /// Requested mesh backend technology is not implemented by this adapter.
    UnsupportedBackend(String),
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshError::InvalidConfig(msg) => write!(f, "invalid mesh configuration: {msg}"),
            MeshError::Crypto(msg) => write!(f, "mesh cryptographic error: {msg}"),
            MeshError::Transport(msg) => write!(f, "mesh transport error: {msg}"),
            MeshError::ReplayDetected(ctr) => write!(f, "replayed packet counter {ctr}"),
            MeshError::PoolExhausted => f.write_str("fixed packet buffer pool exhausted"),
            MeshError::UnsupportedBackend(msg) => write!(f, "unsupported mesh backend: {msg}"),
        }
    }
}

impl std::error::Error for MeshError {}

/// Logical cryptographic node identity on the mesh.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshIdentity {
    /// Operator-assigned logical node label.
    pub node_name: String,
    /// Hex-encoded 32-byte static public key.
    pub public_key_hex: String,
    /// Assigned pure-IPv6 tunnel CIDR address (for example `fd00:db8::1/64`).
    pub mesh_ipv6: String,
    /// Canonical identity digest matching key material.
    pub key_digest_hex: String,
}

impl MeshIdentity {
    /// Parses [`Self::key_digest_hex`] as a typed [`nomos_core::resource::Digest`].
    pub fn digest(&self) -> Option<Digest> {
        Digest::from_hex(&self.key_digest_hex).ok()
    }
}

/// Configured WireGuard peer descriptor supplied by topology configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshPeerSpec {
    /// Logical peer node identifier.
    pub name: String,
    /// Hex-encoded 32-byte peer public key.
    pub public_key_hex: String,
    /// Pure-IPv6 prefixes routed to this peer (for example `fd00:db8::2/128`).
    pub allowed_ips_v6: Vec<String>,
    /// Optional pure-IPv6 UDP endpoint (`[2001:db8::2]:51820`).
    pub endpoint_v6: Option<String>,
    /// Keepalive interval in seconds (`0` disables keepalives).
    pub persistent_keepalive_secs: u16,
}

/// Pure-IPv6 tunnel configuration for `wg0`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelConfig {
    /// Kernel or logical tunnel interface name (for example `wg0`).
    pub interface: String,
    /// Local pure-IPv6 tunnel address.
    pub local_ipv6: String,
    /// UDP listen port.
    pub listen_port: u16,
    /// Outer link PMTU used to derive `outer_pmtu - 80`.
    pub outer_pmtu: u16,
    /// Required IPv6 `accept_ra` sysctl setting (must be `2` when forwarding).
    pub accept_ra: u8,
    /// Configured mesh peers.
    pub peers: Vec<MeshPeerSpec>,
}

impl TunnelConfig {
    /// Validates pure-IPv6 tunnel constraints and returns derived tunnel MTU.
    pub fn validate(&self) -> MeshResult<u16> {
        if self.interface.trim().is_empty() || self.interface.contains(char::is_whitespace) {
            return Err(MeshError::InvalidConfig(
                "empty or whitespace interface".into(),
            ));
        }
        if !self.local_ipv6.contains(':') {
            return Err(MeshError::InvalidConfig(format!(
                "local_ipv6 `{}` must be an IPv6 address",
                self.local_ipv6
            )));
        }
        if self.accept_ra != 2 {
            return Err(MeshError::InvalidConfig(format!(
                "accept_ra must be 2 on IPv6 router/mesh nodes, got {}",
                self.accept_ra
            )));
        }
        for peer in &self.peers {
            if peer.allowed_ips_v6.is_empty()
                || peer.allowed_ips_v6.iter().any(|cidr| !cidr.contains(':'))
            {
                return Err(MeshError::InvalidConfig(format!(
                    "peer `{}` must specify pure-IPv6 allowed_ips_v6",
                    peer.name
                )));
            }
        }
        tunnel_mtu_from_outer_pmtu(self.outer_pmtu)
    }
}

/// Observed packet and handshake counters for one mesh peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerTelemetry {
    /// Logical peer identifier.
    pub name: String,
    /// Monotonic timestamp (nanoseconds) of latest completed handshake.
    pub last_handshake_ns: u64,
    /// Decrypted inbound byte count.
    pub rx_bytes: u64,
    /// Encrypted outbound byte count.
    pub tx_bytes: u64,
    /// Inbound packets accepted after AEAD and anti-replay verification.
    pub rx_packets: u64,
    /// Outbound packets encrypted and queued.
    pub tx_packets: u64,
    /// Inbound tampered packets dropped at AEAD edge before tunnel core.
    pub tampered_drops: u64,
}

/// Overall mesh posture reported by [`MeshPort`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshPosture {
    /// Tunnel interface name.
    pub interface: String,
    /// Configured outer link PMTU.
    pub outer_pmtu: u16,
    /// Derived tunnel MTU (`outer_pmtu - 80`).
    pub tunnel_mtu: u16,
    /// Applied control-plane generation sequence.
    pub generation: u64,
    /// Count of peers with established session keys.
    pub active_peers: usize,
    /// Per-peer telemetry snapshots.
    pub peers: Vec<PeerTelemetry>,
}

impl MeshPosture {
    /// Typed generation sequence from [`nomos_core::plan::Generation`].
    pub fn typed_generation(&self) -> Generation {
        Generation(self.generation)
    }
}

/// Port contract for mesh enrollment, MTU validation, and telemetry posture.
pub trait MeshPort {
    /// Returns the local node's mesh identity descriptor.
    fn identity(&self) -> MeshResult<MeshIdentity>;

    /// Validates `config`, applies/prepares tunnel parameters, and returns derived MTU.
    fn configure_tunnel(&self, config: &TunnelConfig) -> MeshResult<u16>;

    /// Collects current metadata-only peer and tunnel posture.
    fn posture(&self, generation: Generation) -> MeshResult<MeshPosture>;
}
