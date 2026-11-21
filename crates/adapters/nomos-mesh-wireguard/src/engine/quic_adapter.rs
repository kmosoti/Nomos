//! QUIC inner-MTU budget adapter bridging `quinn-proto` with the 80-byte tunnel overhead.

use nomos_mesh::{
    AEAD_TAG_LEN, INNER_OFFSET, MeshResult, max_quic_payload, tunnel_mtu_from_outer_pmtu,
};
use quinn_proto::TransportConfig;

/// Enforces `64 + quic_payload_len + 16 <= outer_pmtu` across QUIC payloads and transport configs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TunnelQuicBudget {
    outer_pmtu: u16,
    max_quic_payload: u16,
}

impl TunnelQuicBudget {
    /// Derives a validated QUIC payload budget from `outer_pmtu` (`>= 1280`).
    pub fn from_outer_pmtu(outer_pmtu: u16) -> MeshResult<Self> {
        let max_quic = tunnel_mtu_from_outer_pmtu(outer_pmtu)?;
        Ok(Self {
            outer_pmtu,
            max_quic_payload: max_quic,
        })
    }

    /// Configured outer link PMTU.
    pub const fn outer_pmtu(self) -> u16 {
        self.outer_pmtu
    }

    /// Maximum encapsulated QUIC datagram length (`outer_pmtu - 80`).
    pub const fn max_quic_payload(self) -> u16 {
        self.max_quic_payload
    }

    /// Clamps an inner payload length so `64 + len + 16 <= outer_pmtu` always holds.
    pub fn clamp_payload_len(self, requested_len: usize) -> usize {
        requested_len.min(max_quic_payload(self.outer_pmtu) as usize)
    }

    /// Verifies the invariant `64 + quic_payload_len + 16 <= outer_pmtu`.
    pub const fn fits(self, quic_payload_len: usize) -> bool {
        INNER_OFFSET + quic_payload_len + AEAD_TAG_LEN <= self.outer_pmtu as usize
    }

    /// Configures a `quinn_proto::TransportConfig` strictly within the tunnel budget.
    pub fn apply_to_transport_config(self, cfg: &mut TransportConfig) {
        let mtu = self.max_quic_payload;
        cfg.mtu_discovery_config(None);
        cfg.min_mtu(mtu);
        cfg.initial_mtu(mtu);
    }
}
