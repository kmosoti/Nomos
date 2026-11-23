//! Peer SPKI authorization policy and remote telemetry tracking for `nomos-loom` QUIC RPC.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use nomos_protocol::TelemetryDatagram;
use nomos_protocol_quic::parse_sha256_hex;

/// Maximum per-pin node telemetry entries tracked in coordinator memory.
pub const MAX_TRACKED_REMOTE_NODES: usize = 1024;

/// Maximum allowed basis-points reading (`100.00%`) accepted in remote telemetry datagrams.
pub const MAX_PRESSURE_BPS: u16 = 10_000;

/// Minimum interval enforced between accepted telemetry datagrams from the same peer pin.
const MIN_TELEMETRY_INTERVAL: Duration = Duration::from_millis(5);

/// Authorization role assigned to an mTLS-pinned QUIC peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerRole {
    /// Coordinator authority permitted to submit Plans and query control status.
    Authority,
    /// Managed host Cell permitted to send telemetry, status/heartbeats, and VerifyReceipt.
    Cell,
}

/// Verified peer policy tying an SPKI SHA-256 pin to a role and optional node identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedPeer {
    /// 32-byte SHA-256 digest of the peer's certificate `SubjectPublicKeyInfo`.
    pub pin: [u8; 32],
    /// Role authorized for this peer.
    pub role: PeerRole,
    /// Optional explicit logical `node_id` enforced on `TelemetryDatagram.node`.
    pub node_id: Option<String>,
}

/// Parses a `--allowed-client-pin` entry: `authority:<HEX>`, `cell:<NODE>:<HEX>`,
/// `cell:<HEX>`, or bare `<HEX>` (which defaults to least-privilege [`PeerRole::Cell`]).
pub fn parse_client_pin_spec(raw: &str) -> Result<AuthorizedPeer, String> {
    let trimmed = raw.trim();
    if let Some(hex) = trimmed.strip_prefix("authority:") {
        let pin =
            parse_sha256_hex(hex).map_err(|e| format!("invalid authority pin `{raw}`: {e}"))?;
        return Ok(AuthorizedPeer {
            pin,
            role: PeerRole::Authority,
            node_id: None,
        });
    }
    if let Some(rest) = trimmed.strip_prefix("cell:") {
        if let Some((node, hex)) = rest.split_once(':') {
            validate_node_id(node)?;
            let pin =
                parse_sha256_hex(hex).map_err(|e| format!("invalid cell pin `{raw}`: {e}"))?;
            return Ok(AuthorizedPeer {
                pin,
                role: PeerRole::Cell,
                node_id: Some(node.to_owned()),
            });
        }
        let pin = parse_sha256_hex(rest).map_err(|e| format!("invalid cell pin `{raw}`: {e}"))?;
        return Ok(AuthorizedPeer {
            pin,
            role: PeerRole::Cell,
            node_id: None,
        });
    }
    let pin = parse_sha256_hex(trimmed).map_err(|e| format!("invalid client pin `{raw}`: {e}"))?;
    Ok(AuthorizedPeer {
        pin,
        role: PeerRole::Cell,
        node_id: None,
    })
}

/// Parses a `--cell-pin` entry: `<NODE>:<HEX>` or bare `<HEX>`.
pub fn parse_cell_pin_spec(raw: &str) -> Result<AuthorizedPeer, String> {
    let trimmed = raw.trim();
    if let Some((node, hex)) = trimmed.split_once(':') {
        validate_node_id(node)?;
        let pin = parse_sha256_hex(hex).map_err(|e| format!("invalid cell pin `{raw}`: {e}"))?;
        Ok(AuthorizedPeer {
            pin,
            role: PeerRole::Cell,
            node_id: Some(node.to_owned()),
        })
    } else {
        let pin =
            parse_sha256_hex(trimmed).map_err(|e| format!("invalid cell pin `{raw}`: {e}"))?;
        Ok(AuthorizedPeer {
            pin,
            role: PeerRole::Cell,
            node_id: None,
        })
    }
}

/// Validates a peer-supplied telemetry `node_id` identifier.
pub fn validate_node_id(node: &str) -> Result<(), String> {
    if node.is_empty() || node.len() > 128 {
        return Err("node_id must be 1..=128 bytes".to_owned());
    }
    if !node
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(format!("invalid node_id `{node}`"));
    }
    Ok(())
}

/// Snapshot of last authenticated telemetry accepted from a remote Cell node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTelemetrySample {
    /// Logical node identifier verified against the sender's SPKI pin.
    pub node: String,
    /// Last accepted generation.
    pub generation: u64,
    /// Last accepted sequence number within `generation`.
    pub seq: u64,
    /// Clamped CPU pressure basis points (`0..=10_000`).
    pub cpu_pressure_bps: u16,
    /// Clamped memory pressure basis points (`0..=10_000`).
    pub memory_pressure_bps: u16,
    /// Clamped IO pressure basis points (`0..=10_000`).
    pub io_pressure_bps: u16,
    /// Active leases reported by node.
    pub active_leases: u32,
}

#[derive(Default)]
pub(crate) struct TelemetryTracker {
    per_peer: HashMap<[u8; 32], PeerTelemetryCursor>,
    node_owners: HashMap<String, [u8; 32]>,
    pub(crate) latest_by_node: BTreeMap<String, RemoteTelemetrySample>,
}

struct PeerTelemetryCursor {
    bound_node: String,
    last_gen_seq: (u64, u64),
    last_accepted_at: Option<Instant>,
}

impl TelemetryTracker {
    pub(crate) fn accept(
        &mut self,
        peer_spki: [u8; 32],
        explicit_node: Option<&str>,
        dgram: &TelemetryDatagram,
        now: Instant,
    ) -> Result<RemoteTelemetrySample, &'static str> {
        if validate_node_id(&dgram.node).is_err() {
            return Err("invalid node identifier");
        }
        if dgram.cpu_pressure_bps > MAX_PRESSURE_BPS
            || dgram.memory_pressure_bps > MAX_PRESSURE_BPS
            || dgram.io_pressure_bps > MAX_PRESSURE_BPS
        {
            return Err("telemetry pressure bps exceeds 10000");
        }
        if let Some(expected_node) = explicit_node
            && expected_node != dgram.node
        {
            return Err("node_id mismatch for peer SPKI pin");
        }
        if let Some(owner) = self.node_owners.get(&dgram.node)
            && *owner != peer_spki
        {
            return Err("node_id already claimed by another peer SPKI pin");
        }

        if let Some(cursor) = self.per_peer.get(&peer_spki) {
            if cursor.bound_node != dgram.node {
                return Err("node_id mismatch against bound SPKI session");
            }
            let next_pos = (dgram.generation, dgram.seq);
            if next_pos <= cursor.last_gen_seq {
                return Err("stale or replayed telemetry generation/seq");
            }
            if let Some(prev_at) = cursor.last_accepted_at
                && now.duration_since(prev_at) < MIN_TELEMETRY_INTERVAL
            {
                return Err("telemetry datagram rate limited");
            }
        } else if self.latest_by_node.len() >= MAX_TRACKED_REMOTE_NODES {
            return Err("remote telemetry table capacity reached");
        }

        self.node_owners.insert(dgram.node.clone(), peer_spki);
        self.per_peer.insert(
            peer_spki,
            PeerTelemetryCursor {
                bound_node: dgram.node.clone(),
                last_gen_seq: (dgram.generation, dgram.seq),
                last_accepted_at: Some(now),
            },
        );
        let sample = RemoteTelemetrySample {
            node: dgram.node.clone(),
            generation: dgram.generation,
            seq: dgram.seq,
            cpu_pressure_bps: dgram.cpu_pressure_bps,
            memory_pressure_bps: dgram.memory_pressure_bps,
            io_pressure_bps: dgram.io_pressure_bps,
            active_leases: dgram.active_leases,
        };
        self.latest_by_node
            .insert(dgram.node.clone(), sample.clone());
        Ok(sample)
    }
}
