//! Boundary packet descriptors and 80-byte WireGuard tunnel framing layout.

use nomos_mesh::{AEAD_TAG_LEN, INNER_OFFSET, MeshError, MeshResult, WG_HEADER_LEN};

use crate::boundary::BufHandle;

/// Explicit Congestion Notification (ECN) codepoint carried across the tunnel boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EcnCodepoint {
    /// Non-ECN capable packet (`0b00`).
    #[default]
    None,
    /// ECN-Capable Transport ECT(0) (`0b10`).
    Ect0,
    /// ECN-Capable Transport ECT(1) (`0b01`).
    Ect1,
    /// Congestion Experienced (`0b11`).
    Ce,
}

impl EcnCodepoint {
    /// Converts from `quinn_udp::EcnCodepoint`.
    pub const fn from_udp(ecn: Option<quinn_udp::EcnCodepoint>) -> Self {
        match ecn {
            None => Self::None,
            Some(quinn_udp::EcnCodepoint::Ect0) => Self::Ect0,
            Some(quinn_udp::EcnCodepoint::Ect1) => Self::Ect1,
            Some(quinn_udp::EcnCodepoint::Ce) => Self::Ce,
        }
    }

    /// Converts into `quinn_udp::EcnCodepoint`.
    pub const fn to_udp(self) -> Option<quinn_udp::EcnCodepoint> {
        match self {
            Self::None => None,
            Self::Ect0 => Some(quinn_udp::EcnCodepoint::Ect0),
            Self::Ect1 => Some(quinn_udp::EcnCodepoint::Ect1),
            Self::Ce => Some(quinn_udp::EcnCodepoint::Ce),
        }
    }

    /// Converts into `quinn_proto::EcnCodepoint`.
    pub const fn to_proto(self) -> Option<quinn_proto::EcnCodepoint> {
        match self {
            Self::None => None,
            Self::Ect0 => Some(quinn_proto::EcnCodepoint::Ect0),
            Self::Ect1 => Some(quinn_proto::EcnCodepoint::Ect1),
            Self::Ce => Some(quinn_proto::EcnCodepoint::Ce),
        }
    }
}

/// Exact byte offsets enforcing the 80-byte tunnel framing contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketLayout {
    /// Encapsulated QUIC payload byte length.
    pub quic_payload_len: usize,
    /// Total wire packet byte length (`64 + quic_payload_len + 16`).
    pub total_len: usize,
}

impl PacketLayout {
    /// Validates `64 + quic_payload_len + 16 <= outer_pmtu` and returns byte offsets.
    pub fn for_quic_payload(quic_payload_len: usize, outer_pmtu: u16) -> MeshResult<Self> {
        let total_len = INNER_OFFSET + quic_payload_len + AEAD_TAG_LEN;
        if total_len > outer_pmtu as usize {
            return Err(MeshError::InvalidConfig(format!(
                "tunnel packet {total_len} bytes exceeds outer_pmtu {outer_pmtu} (64 + {quic_payload_len} + 16)"
            )));
        }
        Ok(Self {
            quic_payload_len,
            total_len,
        })
    }

    /// Range `[0..16]` holding the WireGuard transport header.
    pub const fn wg_header_range() -> core::ops::Range<usize> {
        0..WG_HEADER_LEN
    }

    /// Range `[16..64]` holding the 48-byte inner IPv6+UDP header.
    pub const fn inner_header_range() -> core::ops::Range<usize> {
        WG_HEADER_LEN..INNER_OFFSET
    }

    /// Range `[64..len-16]` holding the encapsulated QUIC payload.
    pub const fn payload_range(&self) -> core::ops::Range<usize> {
        INNER_OFFSET..(INNER_OFFSET + self.quic_payload_len)
    }

    /// Range `[len-16..len]` holding the trailing Poly1305 AEAD tag.
    pub const fn tag_range(&self) -> core::ops::Range<usize> {
        (self.total_len - AEAD_TAG_LEN)..self.total_len
    }
}

/// Cryptographic work direction dispatched by the shell AEAD engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CipherDirection {
    /// Seals plaintext `[16..len-16]` and writes 16-byte tag at `[len-16..len]`.
    Encrypt,
    /// Verifies 16-byte tag at `[len-16..len]` and opens `[16..len-16]` in place.
    Decrypt,
}

/// Single AEAD task handed between the tunnel core and shell crypto workers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CipherTask {
    /// Pool slot containing the packet buffer.
    pub handle: BufHandle,
    /// Key table slot index identifying peer AEAD keys.
    pub peer_slot: u16,
    /// WireGuard receiver index carried in `[4..8]`.
    pub receiver_index: u32,
    /// 64-bit little-endian WireGuard packet counter / AEAD nonce.
    pub counter: u64,
    /// Total wire packet byte count including 16B header + 48B inner + payload + 16B tag.
    pub wire_len: usize,
    /// Encryption vs decryption direction.
    pub direction: CipherDirection,
    /// ECN bits preserved across tunnel encapsulation.
    pub ecn: EcnCodepoint,
}

/// Verified plaintext RX packet descriptor delivered from shell AEAD verification into `CoreEngine::poll`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecryptedRx {
    /// Pool slot holding verified plaintext inner IPv6+UDP+QUIC bytes at `[16..len-16]`.
    pub handle: BufHandle,
    /// Peer table index matching [`KeyTable`].
    pub peer_slot: u16,
    /// Authenticated WireGuard packet counter from `[8..16]`.
    pub counter: u64,
    /// Original total wire length (`64 + quic_payload_len + 16`).
    pub wire_len: usize,
    /// Observed ECN codepoint from the outer UDP socket.
    pub ecn: EcnCodepoint,
}

/// Deterministic action emitted by `CoreEngine::poll`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreOutput {
    /// Deliver verified QUIC payload `[64..wire_len-16]` from `handle` to inner QUIC consumer.
    DeliverInner {
        /// Pool handle holding packet bytes.
        handle: BufHandle,
        /// Peer slot index.
        peer_slot: u16,
        /// Payload offset (`64`).
        payload_offset: usize,
        /// Payload length (`<= outer_pmtu - 80`).
        payload_len: usize,
        /// Outer ECN marking.
        ecn: EcnCodepoint,
    },
    /// Queue keepalive or encrypted outbound frame through shell UDP transmitter.
    TransmitPeer {
        /// Pool handle holding sealed packet bytes.
        handle: BufHandle,
        /// Peer slot index.
        peer_slot: u16,
        /// Wire packet length.
        wire_len: usize,
        /// ECN marking to set on outer UDP socket.
        ecn: EcnCodepoint,
    },
    /// Recycle handle immediately back into [`crate::BufPool`].
    RecycleHandle {
        /// Handle no longer needed by the core.
        handle: BufHandle,
    },
}
