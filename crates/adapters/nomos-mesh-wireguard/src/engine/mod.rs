//! Deterministic, zero-allocation Sans-I/O WireGuard tunnel engine.

pub mod inner_ip;
pub mod quic_adapter;
pub mod sanstun;
pub mod timer_wheel;

use nomos_mesh::{
    AEAD_TAG_LEN, INNER_IPV6_UDP_LEN, INNER_OFFSET, MeshError, MeshResult, WG_HEADER_LEN,
};

use crate::boundary::{
    BufPool, CipherDirection, CipherTask, CoreOutput, DecryptedRx, EcnCodepoint, FixedRing,
    KeyTableUpdate, PacketLayout, SessionKeys, SliceWriter,
};

pub use inner_ip::{InnerEndpoint, InnerIpv6UdpHeader, udp_v6_checksum};
pub use quic_adapter::TunnelQuicBudget;
pub use sanstun::{PeerTunnelState, ReplayWindow};
pub use timer_wheel::{TimerAction, TimerEntry, TimerWheel};

const WG_TYPE_TRANSPORT: u32 = 4;
const DEFAULT_KEEPALIVE_NS: u64 = 25_000_000_000;
const DEFAULT_REKEY_NS: u64 = 120_000_000_000;
const TIMER_SCRATCH_SLOTS: usize = 32;

/// Single-threaded deterministic Sans-I/O tunnel state machine.
///
/// All time enters explicitly as `now_ns: u64`. No wall-clock queries, socket
/// I/O, randomness, or steady-state heap allocations happen inside [`Self::poll`]
/// or [`Self::encapsulate_quic_tx`].
#[derive(Debug)]
pub struct CoreEngine {
    budget: TunnelQuicBudget,
    peers: Vec<PeerTunnelState>,
    timers: TimerWheel,
    timer_scratch: [Option<TimerEntry>; TIMER_SCRATCH_SLOTS],
    keepalive_src: InnerEndpoint,
    keepalive_dst: InnerEndpoint,
    /// Cumulative inbound frames rejected by anti-replay or malformed inner framing.
    pub dropped_invalid: u64,
    /// Cumulative timer-triggered keepalives emitted.
    pub keepalives_emitted: u64,
    /// Cumulative deterministic rekey notifications emitted.
    pub rekey_requests: u64,
}

impl CoreEngine {
    /// Pre-allocates tunnel state for `peer_count` peers and `outer_pmtu` budget.
    pub fn new(
        peer_count: usize,
        outer_pmtu: u16,
        keepalive_src: InnerEndpoint,
        keepalive_dst: InnerEndpoint,
    ) -> MeshResult<Self> {
        let budget = TunnelQuicBudget::from_outer_pmtu(outer_pmtu)?;
        let mut peers = Vec::with_capacity(peer_count);
        for i in 0..peer_count {
            peers.push(PeerTunnelState::new(i as u16));
        }
        Ok(Self {
            budget,
            peers,
            timers: TimerWheel::with_capacity(peer_count.saturating_mul(2).max(8)),
            timer_scratch: [None; TIMER_SCRATCH_SLOTS],
            keepalive_src,
            keepalive_dst,
            dropped_invalid: 0,
            keepalives_emitted: 0,
            rekey_requests: 0,
        })
    }

    /// Tunnel QUIC payload budget (`outer_pmtu - 80`).
    pub const fn budget(&self) -> TunnelQuicBudget {
        self.budget
    }

    /// Immutable peer state slice.
    pub fn peers(&self) -> &[PeerTunnelState] {
        &self.peers
    }

    /// Installs deterministic session keys for `peer_slot` and arms keepalive/rekey timers.
    pub fn install_peer_session(
        &mut self,
        peer_slot: u16,
        keys: SessionKeys,
        now_ns: u64,
        key_updates: &mut FixedRing<KeyTableUpdate>,
    ) -> MeshResult<()> {
        let peer = self
            .peers
            .get_mut(peer_slot as usize)
            .ok_or_else(|| MeshError::InvalidConfig("peer_slot out of range".into()))?;
        peer.activate(keys.sender_index, keys.receiver_index, now_ns);
        key_updates.push(KeyTableUpdate::Install { peer_slot, keys })?;
        self.timers.schedule(TimerEntry {
            deadline_ns: now_ns.saturating_add(DEFAULT_KEEPALIVE_NS),
            peer_slot,
            action: TimerAction::Keepalive,
        })?;
        self.timers.schedule(TimerEntry {
            deadline_ns: now_ns.saturating_add(DEFAULT_REKEY_NS),
            peer_slot,
            action: TimerAction::Rekey,
        })?;
        Ok(())
    }

    /// Advances tunnel state for `now_ns` and verified `rx_batch` without heap allocation.
    pub fn poll(
        &mut self,
        now_ns: u64,
        rx_batch: &[DecryptedRx],
        pool: &mut BufPool,
        out: &mut FixedRing<CoreOutput>,
        cipher_out: &mut FixedRing<CipherTask>,
    ) -> MeshResult<()> {
        for rx in rx_batch {
            if !self.process_rx(rx, pool, out)? {
                self.dropped_invalid = self.dropped_invalid.saturating_add(1);
                let _ = out.push(CoreOutput::RecycleHandle { handle: rx.handle });
            }
        }

        let expired = self.timers.poll_expired(now_ns, &mut self.timer_scratch);
        for idx in 0..expired {
            let Some(entry) = self.timer_scratch[idx].take() else {
                continue;
            };
            match entry.action {
                TimerAction::Keepalive => {
                    if let Some(peer) = self.peers.get(entry.peer_slot as usize)
                        && peer.established
                        && let Ok(task) = self.encapsulate_quic_tx(
                            entry.peer_slot,
                            &[],
                            self.keepalive_src,
                            self.keepalive_dst,
                            EcnCodepoint::None,
                            pool,
                        )
                    {
                        let _ = cipher_out.push(task);
                        self.keepalives_emitted = self.keepalives_emitted.saturating_add(1);
                    }
                    let _ = self.timers.schedule(TimerEntry {
                        deadline_ns: now_ns.saturating_add(DEFAULT_KEEPALIVE_NS),
                        peer_slot: entry.peer_slot,
                        action: TimerAction::Keepalive,
                    });
                }
                TimerAction::Rekey => {
                    self.rekey_requests = self.rekey_requests.saturating_add(1);
                    let _ = self.timers.schedule(TimerEntry {
                        deadline_ns: now_ns.saturating_add(DEFAULT_REKEY_NS),
                        peer_slot: entry.peer_slot,
                        action: TimerAction::Rekey,
                    });
                }
            }
        }
        Ok(())
    }

    /// Encapsulates an outbound QUIC payload into `[0..16][16..64][64..len-16][len-16..len]` without allocating.
    pub fn encapsulate_quic_tx(
        &mut self,
        peer_slot: u16,
        quic_payload: &[u8],
        src: InnerEndpoint,
        dst: InnerEndpoint,
        ecn: EcnCodepoint,
        pool: &mut BufPool,
    ) -> MeshResult<CipherTask> {
        let layout = PacketLayout::for_quic_payload(quic_payload.len(), self.budget.outer_pmtu())?;
        let peer = self
            .peers
            .get_mut(peer_slot as usize)
            .ok_or(MeshError::PoolExhausted)?;
        if !peer.established {
            return Err(MeshError::Crypto("peer session not established".into()));
        }

        let counter = peer.next_tx_counter();
        let receiver_index = peer.sender_index;
        let handle = pool.acquire()?;
        let slot = pool.slice_mut(handle)?;
        if slot.len() < layout.total_len {
            pool.release(handle);
            return Err(MeshError::PoolExhausted);
        }

        // [0..16] WireGuard transport header (type=4, receiver_index LE, counter LE).
        {
            let mut wg = SliceWriter::new(&mut slot[..WG_HEADER_LEN]);
            wg.write_u32_le(WG_TYPE_TRANSPORT)?;
            wg.write_u32_le(receiver_index)?;
            wg.write_u64_le(counter)?;
        }

        // [16..64] Inner IPv6 + UDP header.
        let inner_hdr = InnerIpv6UdpHeader {
            src,
            dst,
            payload_len: quic_payload.len() as u16,
            traffic_class: match ecn {
                EcnCodepoint::None => 0,
                EcnCodepoint::Ect1 => 0b01,
                EcnCodepoint::Ect0 => 0b10,
                EcnCodepoint::Ce => 0b11,
            },
            udp_checksum: 0,
        };
        inner_hdr.encode_into(
            quic_payload,
            &mut slot[WG_HEADER_LEN..(WG_HEADER_LEN + INNER_IPV6_UDP_LEN)],
        )?;

        // [64..len-16] QUIC payload, [len-16..len] zeroed Poly1305 tag tailroom.
        slot[layout.payload_range()].copy_from_slice(quic_payload);
        slot[layout.tag_range()].fill(0);

        peer.tx_packets = peer.tx_packets.saturating_add(1);
        peer.tx_bytes = peer.tx_bytes.saturating_add(quic_payload.len() as u64);

        Ok(CipherTask {
            handle,
            peer_slot,
            receiver_index,
            counter,
            wire_len: layout.total_len,
            direction: CipherDirection::Encrypt,
            ecn,
        })
    }

    fn process_rx(
        &mut self,
        rx: &DecryptedRx,
        pool: &BufPool,
        out: &mut FixedRing<CoreOutput>,
    ) -> MeshResult<bool> {
        let Some(peer) = self.peers.get_mut(rx.peer_slot as usize) else {
            return Ok(false);
        };
        if !peer.established {
            return Ok(false);
        }
        if rx.wire_len < INNER_OFFSET + AEAD_TAG_LEN
            || rx.wire_len > self.budget.outer_pmtu() as usize
        {
            return Ok(false);
        }
        if peer.replay.check_and_accept(rx.counter).is_err() {
            return Ok(false);
        }

        let buf = pool.slice(rx.handle, rx.wire_len)?;
        let Ok(hdr) = InnerIpv6UdpHeader::decode_from(&buf[WG_HEADER_LEN..INNER_OFFSET]) else {
            return Ok(false);
        };
        let payload_len = rx.wire_len - INNER_OFFSET - AEAD_TAG_LEN;
        if hdr.payload_len as usize != payload_len {
            return Ok(false);
        }

        peer.rx_packets = peer.rx_packets.saturating_add(1);
        peer.rx_bytes = peer.rx_bytes.saturating_add(payload_len as u64);
        out.push(CoreOutput::DeliverInner {
            handle: rx.handle,
            peer_slot: rx.peer_slot,
            payload_offset: INNER_OFFSET,
            payload_len,
            ecn: rx.ecn,
        })?;
        Ok(true)
    }
}
