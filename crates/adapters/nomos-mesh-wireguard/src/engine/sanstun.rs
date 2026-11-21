//! Sans-I/O tunnel anti-replay window, AllowedIPs check, and session state machine.

use std::net::Ipv6Addr;

use nomos_mesh::{MeshError, MeshResult};

const WINDOW_WORDS: usize = 32; // 32 * 64 = 2048 sequence slots

/// Checks whether `addr` falls within IPv6 CIDR `prefix/prefix_len` with zero allocation.
#[inline]
pub const fn ipv6_in_prefix(addr: Ipv6Addr, prefix: Ipv6Addr, prefix_len: u8) -> bool {
    if prefix_len == 0 {
        return true;
    }
    if prefix_len >= 128 {
        return u128::from_be_bytes(addr.octets()) == u128::from_be_bytes(prefix.octets());
    }
    let mask = !0u128 << (128 - prefix_len as u32);
    (u128::from_be_bytes(addr.octets()) & mask) == (u128::from_be_bytes(prefix.octets()) & mask)
}

/// Fixed-size 2048-bit RFC 6479 sliding anti-replay window (zero heap allocation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayWindow {
    last: u64,
    initialized: bool,
    bitmap: [u64; WINDOW_WORDS],
}

impl Default for ReplayWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplayWindow {
    /// Creates a cleared replay window ready for counter `0..`.
    pub const fn new() -> Self {
        Self {
            last: 0,
            initialized: false,
            bitmap: [0u64; WINDOW_WORDS],
        }
    }

    /// Resets window state on session key rotation.
    pub fn reset(&mut self) {
        self.last = 0;
        self.initialized = false;
        self.bitmap.fill(0);
    }

    /// Accepts fresh packet counter `counter`, rejecting duplicates and stale frames.
    pub fn check_and_accept(&mut self, counter: u64) -> MeshResult<()> {
        let window_bits = (WINDOW_WORDS * 64) as u64;
        if !self.initialized {
            self.initialized = true;
            self.last = counter;
            let slot = counter % window_bits;
            self.bitmap[(slot / 64) as usize] |= 1u64 << (slot % 64);
            return Ok(());
        }

        if counter > self.last {
            let diff = counter - self.last;
            if diff >= window_bits {
                self.bitmap.fill(0);
            } else {
                for bit in 1..=diff {
                    let slot = (self.last + bit) % window_bits;
                    self.bitmap[(slot / 64) as usize] &= !(1u64 << (slot % 64));
                }
            }
            let slot = counter % window_bits;
            self.bitmap[(slot / 64) as usize] |= 1u64 << (slot % 64);
            self.last = counter;
            return Ok(());
        }

        let age = self.last - counter;
        if age >= window_bits {
            return Err(MeshError::ReplayDetected(counter));
        }
        let slot = counter % window_bits;
        let word = (slot / 64) as usize;
        let mask = 1u64 << (slot % 64);
        if (self.bitmap[word] & mask) != 0 {
            return Err(MeshError::ReplayDetected(counter));
        }
        self.bitmap[word] |= mask;
        Ok(())
    }
}

/// Deterministic peer handshake, AllowedIPs, and traffic accounting state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerTunnelState {
    /// Configured peer slot index.
    pub peer_slot: u16,
    /// True once session keys are active.
    pub established: bool,
    /// Remote peer's receiver index used on outbound WG headers.
    pub sender_index: u32,
    /// Local receiver index for inbound lookup.
    pub receiver_index: u32,
    /// Monotonic outbound counter (`[8..16]`).
    pub tx_counter: u64,
    /// Anti-replay window over inbound `[8..16]` counters.
    pub replay: ReplayWindow,
    /// Monotonic timestamp (`now_ns`) of latest handshake/key installation.
    pub last_handshake_ns: u64,
    /// Optional inner IPv6 `AllowedIPs` source prefix enforced on RX.
    pub allowed_prefix: Option<(Ipv6Addr, u8)>,
    /// True once a rekey request has been emitted for the current epoch.
    pub rekey_pending: bool,
    /// Bytes transferred in the current key epoch (for byte-counter rekeying).
    pub session_bytes: u64,
    /// Accepted inbound payload bytes.
    pub rx_bytes: u64,
    /// Encapsulated outbound payload bytes.
    pub tx_bytes: u64,
    /// Accepted inbound packets.
    pub rx_packets: u64,
    /// Encapsulated outbound packets.
    pub tx_packets: u64,
}

impl PeerTunnelState {
    /// Initializes an unestablished peer slot.
    pub const fn new(peer_slot: u16) -> Self {
        Self {
            peer_slot,
            established: false,
            sender_index: 0,
            receiver_index: 0,
            tx_counter: 0,
            replay: ReplayWindow::new(),
            last_handshake_ns: 0,
            allowed_prefix: None,
            rekey_pending: false,
            session_bytes: 0,
            rx_bytes: 0,
            tx_bytes: 0,
            rx_packets: 0,
            tx_packets: 0,
        }
    }

    /// Marks session keys active and resets epoch counters.
    pub fn activate(&mut self, sender_index: u32, receiver_index: u32, now_ns: u64) {
        self.established = true;
        self.sender_index = sender_index;
        self.receiver_index = receiver_index;
        self.tx_counter = 0;
        self.replay.reset();
        self.last_handshake_ns = now_ns;
        self.rekey_pending = false;
        self.session_bytes = 0;
    }

    /// Retires an expired session, blocking further RX/TX until rekeyed.
    pub fn retire(&mut self) {
        self.established = false;
        self.rekey_pending = false;
        self.replay.reset();
        self.session_bytes = 0;
    }

    /// Allocates next deterministic TX counter.
    pub fn next_tx_counter(&mut self) -> u64 {
        let ctr = self.tx_counter;
        self.tx_counter = self.tx_counter.saturating_add(1);
        ctr
    }
}
