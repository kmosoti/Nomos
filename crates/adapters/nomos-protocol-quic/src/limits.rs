//! Connection admission, per-IP/per-peer caps, datagram rate limits, and SPKI extraction.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quinn::Connection;
use rustls::pki_types::CertificateDer;

use crate::cert::spki_sha256_bytes;

/// Default maximum total concurrent mTLS QUIC connections accepted by a server endpoint.
pub const DEFAULT_MAX_CONCURRENT_CONNECTIONS: usize = 64;

/// Default maximum concurrent in-flight QUIC handshakes before admission.
pub const DEFAULT_MAX_PENDING_HANDSHAKES: usize = 32;

/// Default maximum active mTLS QUIC connections permitted from a single source IP address.
pub const DEFAULT_MAX_CONNECTIONS_PER_IP: usize = 8;

/// Default maximum active mTLS QUIC connections permitted per verified peer SPKI SHA-256.
pub const DEFAULT_MAX_CONNECTIONS_PER_PEER: usize = 4;

/// Default handshake completion timeout enforced on both incoming and outgoing QUIC connections.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Default QUIC transport idle timeout before closing an inactive connection.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Default QUIC flow-control cap on concurrent bidirectional RPC streams per connection.
pub const DEFAULT_QUIC_MAX_BIDI_STREAMS: u32 = 32;

/// Maximum incoming telemetry datagram payload size in bytes.
pub const MAX_DATAGRAM_BYTES: usize = 16 * 1024;

/// Burst allowance and sustained refill rate (datagrams/sec) per connection.
const DEFAULT_DATAGRAM_BURST: f64 = 64.0;
const DEFAULT_DATAGRAM_REFILL_PER_SEC: f64 = 16.0;

/// Extracts the SHA-256 digest of the peer leaf certificate's `SubjectPublicKeyInfo`.
///
/// Returns `None` if no peer certificate chain is attached or DER parsing fails.
pub fn extract_peer_spki_sha256(conn: &Connection) -> Option<[u8; 32]> {
    let identity = conn.peer_identity()?;
    let certs = identity.downcast_ref::<Vec<CertificateDer<'static>>>()?;
    let end_entity = certs.first()?;
    spki_sha256_bytes(end_entity.as_ref()).ok()
}

/// Shared per-source-IP and per-verified-SPKI active connection tracker.
#[derive(Debug)]
pub struct ConnectionRegistry {
    per_ip: Mutex<HashMap<IpAddr, usize>>,
    per_peer: Mutex<HashMap<[u8; 32], usize>>,
    max_per_ip: usize,
    max_per_peer: usize,
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_CONNECTIONS_PER_IP,
            DEFAULT_MAX_CONNECTIONS_PER_PEER,
        )
    }
}

impl ConnectionRegistry {
    /// Creates a registry with explicit per-IP and per-peer connection limits.
    pub fn new(max_per_ip: usize, max_per_peer: usize) -> Self {
        Self {
            per_ip: Mutex::new(HashMap::new()),
            per_peer: Mutex::new(HashMap::new()),
            max_per_ip: max_per_ip.max(1),
            max_per_peer: max_per_peer.max(1),
        }
    }

    /// Reserves one connection slot for `ip`, or returns `None` if at `max_per_ip`.
    pub fn try_acquire_ip(self: &Arc<Self>, ip: IpAddr) -> Option<IpConnGuard> {
        let mut map = self.per_ip.lock().unwrap_or_else(|e| e.into_inner());
        let count = map.entry(ip).or_insert(0);
        if *count >= self.max_per_ip {
            return None;
        }
        *count += 1;
        Some(IpConnGuard {
            registry: Arc::clone(self),
            ip,
        })
    }

    /// Reserves one connection slot for `peer_spki`, or returns `None` if at `max_per_peer`.
    pub fn try_acquire_peer(self: &Arc<Self>, peer_spki: [u8; 32]) -> Option<PeerConnGuard> {
        let mut map = self.per_peer.lock().unwrap_or_else(|e| e.into_inner());
        let count = map.entry(peer_spki).or_insert(0);
        if *count >= self.max_per_peer {
            return None;
        }
        *count += 1;
        Some(PeerConnGuard {
            registry: Arc::clone(self),
            peer_spki,
        })
    }
}

/// RAII guard decrementing the source IP connection counter on drop.
#[derive(Debug)]
pub struct IpConnGuard {
    registry: Arc<ConnectionRegistry>,
    ip: IpAddr,
}

impl Drop for IpConnGuard {
    fn drop(&mut self) {
        let mut map = self
            .registry
            .per_ip
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(count) = map.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.ip);
            }
        }
    }
}

/// RAII guard decrementing the per-peer SPKI connection counter on drop.
#[derive(Debug)]
pub struct PeerConnGuard {
    registry: Arc<ConnectionRegistry>,
    peer_spki: [u8; 32],
}

impl Drop for PeerConnGuard {
    fn drop(&mut self) {
        let mut map = self
            .registry
            .per_peer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(count) = map.get_mut(&self.peer_spki) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.peer_spki);
            }
        }
    }
}

/// Per-connection token bucket throttling RFC 9221 telemetry datagrams.
#[derive(Debug)]
pub struct DatagramLimiter {
    tokens: f64,
    burst: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl Default for DatagramLimiter {
    fn default() -> Self {
        Self {
            tokens: DEFAULT_DATAGRAM_BURST,
            burst: DEFAULT_DATAGRAM_BURST,
            refill_per_sec: DEFAULT_DATAGRAM_REFILL_PER_SEC,
            last: Instant::now(),
        }
    }
}

impl DatagramLimiter {
    /// Consumes one datagram token if available at `now`.
    pub fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}
