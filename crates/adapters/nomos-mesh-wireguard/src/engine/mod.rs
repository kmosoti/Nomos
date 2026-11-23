//! Deterministic, zero-allocation Sans-I/O WireGuard tunnel engine.

mod framing;
pub mod handshake;
pub mod handshake_crypto;
pub mod inner_ip;
pub mod quic_adapter;
pub mod sanstun;
pub mod timer_wheel;

use std::net::Ipv6Addr;

use nomos_mesh::{MeshError, MeshResult};

use crate::boundary::{
    BufPool, CipherTask, CoreOutput, DecryptedRx, EcnCodepoint, FixedRing, KeyTableUpdate,
    SessionKeys,
};
use framing::{encode_tx_slot, verify_and_deliver_rx};

pub use handshake::{
    COOKIE_REPLY_LEN, CookieGenerator, INITIATION_LEN, InitiatorHandshake, MSG_TYPE_COOKIE_REPLY,
    MSG_TYPE_INITIATION, MSG_TYPE_RESPONSE, RESPONSE_LEN, ResponderOutcome, StaticKeypair,
    consume_cookie_reply, initiate_handshake, respond_handshake,
};
pub use inner_ip::{
    InnerDestinationPolicy, InnerEndpoint, InnerIpv6UdpHeader, udp_v6_checksum,
    validate_inner_endpoints,
};
pub use quic_adapter::TunnelQuicBudget;
pub use sanstun::{PeerTunnelState, ReplayWindow, ipv6_in_prefix};
pub use timer_wheel::{TimerAction, TimerEntry, TimerWheel};

const DEFAULT_KEEPALIVE_NS: u64 = 25_000_000_000;
/// Soft rekey interval (120 seconds per WireGuard specification).
pub const REKEY_AFTER_TIME_NS: u64 = 120_000_000_000;
/// Hard session expiry lifetime (180 seconds) retiring unrotated sessions.
pub const REJECT_AFTER_TIME_NS: u64 = 180_000_000_000;
/// Default byte-counter threshold triggering proactive session rekey (1 GiB).
pub const DEFAULT_REKEY_AFTER_BYTES: u64 = 1024 * 1024 * 1024;
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
    rekey_after_bytes: u64,
    /// Cumulative inbound frames rejected by anti-replay, AllowedIPs, checksum, or framing.
    pub dropped_invalid: u64,
    /// Cumulative timer-triggered keepalives emitted.
    pub keepalives_emitted: u64,
    /// Cumulative deterministic rekey notifications emitted.
    pub rekey_requests: u64,
    /// Cumulative expired sessions retired.
    pub sessions_retired: u64,
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
            timers: TimerWheel::with_capacity(peer_count.saturating_mul(3).max(12)),
            timer_scratch: [None; TIMER_SCRATCH_SLOTS],
            keepalive_src,
            keepalive_dst,
            rekey_after_bytes: DEFAULT_REKEY_AFTER_BYTES,
            dropped_invalid: 0,
            keepalives_emitted: 0,
            rekey_requests: 0,
            sessions_retired: 0,
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

    /// Earliest armed timer deadline in `O(1)`.
    pub const fn next_wakeup_ns(&self) -> Option<u64> {
        self.timers.next_wakeup_ns()
    }

    /// Configures an inner IPv6 `AllowedIPs` source prefix enforced on RX for `peer_slot`.
    pub fn set_peer_allowed_prefix(
        &mut self,
        peer_slot: u16,
        prefix: Ipv6Addr,
        prefix_len: u8,
    ) -> MeshResult<()> {
        let peer = self
            .peers
            .get_mut(peer_slot as usize)
            .ok_or_else(|| MeshError::InvalidConfig("peer_slot out of range".into()))?;
        peer.allowed_prefix = Some((prefix, prefix_len.min(128)));
        Ok(())
    }

    /// Configures an authorized inner destination IPv6 prefix and optional UDP port range for `peer_slot`.
    pub fn set_peer_allowed_destination(
        &mut self,
        peer_slot: u16,
        dst_prefix: Ipv6Addr,
        dst_prefix_len: u8,
        port_range: Option<(u16, u16)>,
    ) -> MeshResult<()> {
        if let Some((min_p, max_p)) = port_range
            && (min_p == 0 || min_p > max_p)
        {
            return Err(MeshError::InvalidConfig(
                "invalid inner destination UDP port range".into(),
            ));
        }
        let peer = self
            .peers
            .get_mut(peer_slot as usize)
            .ok_or_else(|| MeshError::InvalidConfig("peer_slot out of range".into()))?;
        peer.allowed_dst = Some(InnerDestinationPolicy {
            prefix: (dst_prefix, dst_prefix_len.min(128)),
            port_range,
        });
        Ok(())
    }

    /// Configures the per-session byte-counter rekey threshold.
    pub fn set_rekey_after_bytes(&mut self, bytes: u64) {
        self.rekey_after_bytes = bytes.max(1);
    }

    /// Installs deterministic session keys for `peer_slot` and arms keepalive/rekey/expiry timers.
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
        key_updates.push(KeyTableUpdate::Install {
            peer_slot,
            keys: Box::new(keys),
        })?;
        self.timers.schedule(TimerEntry {
            deadline_ns: now_ns.saturating_add(DEFAULT_KEEPALIVE_NS),
            peer_slot,
            action: TimerAction::Keepalive,
        })?;
        self.timers.schedule(TimerEntry {
            deadline_ns: now_ns.saturating_add(REKEY_AFTER_TIME_NS),
            peer_slot,
            action: TimerAction::Rekey,
        })?;
        self.timers.schedule(TimerEntry {
            deadline_ns: now_ns.saturating_add(REJECT_AFTER_TIME_NS),
            peer_slot,
            action: TimerAction::SessionExpire,
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
        let outer_pmtu = self.budget.outer_pmtu();
        for rx in rx_batch {
            let accepted = match self.peers.get_mut(rx.peer_slot as usize) {
                Some(peer) => verify_and_deliver_rx(peer, outer_pmtu, rx, pool, out)?,
                None => false,
            };
            if !accepted {
                self.dropped_invalid = self.dropped_invalid.saturating_add(1);
                let _ = out.push(CoreOutput::RecycleHandle { handle: rx.handle });
            }
        }

        for peer in &mut self.peers {
            if peer.established
                && !peer.rekey_pending
                && peer.session_bytes >= self.rekey_after_bytes
            {
                peer.rekey_pending = true;
                self.rekey_requests = self.rekey_requests.saturating_add(1);
                let _ = out.push(CoreOutput::RekeyRequired {
                    peer_slot: peer.peer_slot,
                });
            }
        }

        self.poll_expired_timers(now_ns, pool, out, cipher_out);
        Ok(())
    }

    fn poll_expired_timers(
        &mut self,
        now_ns: u64,
        pool: &mut BufPool,
        out: &mut FixedRing<CoreOutput>,
        cipher_out: &mut FixedRing<CipherTask>,
    ) {
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
                        let _ = self.timers.schedule(TimerEntry {
                            deadline_ns: now_ns.saturating_add(DEFAULT_KEEPALIVE_NS),
                            peer_slot: entry.peer_slot,
                            action: TimerAction::Keepalive,
                        });
                    }
                }
                TimerAction::Rekey => {
                    if let Some(peer) = self.peers.get_mut(entry.peer_slot as usize)
                        && peer.established
                    {
                        peer.rekey_pending = true;
                        self.rekey_requests = self.rekey_requests.saturating_add(1);
                        let _ = out.push(CoreOutput::RekeyRequired {
                            peer_slot: entry.peer_slot,
                        });
                    }
                }
                TimerAction::SessionExpire => {
                    if let Some(peer) = self.peers.get_mut(entry.peer_slot as usize)
                        && peer.established
                        && now_ns.saturating_sub(peer.last_handshake_ns) >= REJECT_AFTER_TIME_NS
                    {
                        peer.retire();
                        self.timers.cancel_peer(entry.peer_slot);
                        self.sessions_retired = self.sessions_retired.saturating_add(1);
                        let _ = out.push(CoreOutput::RetirePeer {
                            peer_slot: entry.peer_slot,
                        });
                    }
                }
            }
        }
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
        let outer_pmtu = self.budget.outer_pmtu();
        let peer = self
            .peers
            .get_mut(peer_slot as usize)
            .ok_or(MeshError::PoolExhausted)?;
        encode_tx_slot(peer, outer_pmtu, quic_payload, src, dst, ecn, pool)
    }
}
