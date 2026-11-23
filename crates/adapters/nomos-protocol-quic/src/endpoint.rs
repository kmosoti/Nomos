//! Pure-IPv6 mandatory-mTLS QUIC RPC server and client endpoint with bounded stream
//! concurrency (`Semaphore`), per-RPC timeouts, and JSON/CBOR framing.

use std::future::Future;
use std::net::{SocketAddr, SocketAddrV6};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nomos_protocol::{
    ControlEnvelope, ControlReply, ControlTransportMetrics, ProtocolError, ProtocolResult,
    QuicStats, TelemetryDatagram,
};
use quinn::{Endpoint, VarInt};
use tokio::sync::Semaphore;

use crate::cert::{PinnedIdentity, parse_sha256_hex};
use crate::limits::{
    ConnectionRegistry, DEFAULT_HANDSHAKE_TIMEOUT, DEFAULT_MAX_CONCURRENT_CONNECTIONS,
    DEFAULT_MAX_CONNECTIONS_PER_IP, DEFAULT_MAX_CONNECTIONS_PER_PEER,
    DEFAULT_MAX_PENDING_HANDSHAKES, extract_peer_spki_sha256,
};
pub use crate::session::PinnedQuicConnection;
use crate::session::serve_connection;
use crate::tls::{build_client_config, build_server_config};

/// Default maximum concurrent bidirectional RPC streams served per endpoint.
pub const DEFAULT_MAX_CONCURRENT_STREAMS: usize = 128;

/// Default per-RPC stream read/write timeout.
pub const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(10);

/// Async handler trait invoked on each verified incoming RPC envelope.
pub trait ControlHandler: Send + Sync + 'static {
    /// Processes one bidirectional RPC request and returns a reply envelope.
    fn handle<'a>(
        &'a self,
        request: ControlEnvelope,
    ) -> Pin<Box<dyn Future<Output = ControlReply> + Send + 'a>>;

    /// Processes one incoming telemetry datagram.
    fn handle_telemetry(&self, _dg: TelemetryDatagram) {}

    /// Processes one bidirectional RPC request alongside the mTLS-verified peer SPKI SHA-256.
    fn handle_authenticated<'a>(
        &'a self,
        _peer_spki_sha256: [u8; 32],
        request: ControlEnvelope,
    ) -> Pin<Box<dyn Future<Output = ControlReply> + Send + 'a>> {
        self.handle(request)
    }

    /// Processes one incoming telemetry datagram alongside the mTLS-verified peer SPKI SHA-256.
    fn handle_telemetry_authenticated(&self, _peer_spki_sha256: [u8; 32], dg: TelemetryDatagram) {
        self.handle_telemetry(dg);
    }
}

/// Atomic transport counters shared across server accept loops and client connections.
#[derive(Debug, Default)]
pub struct AtomicQuicStats {
    active_connections: AtomicU64,
    rpc_calls: AtomicU64,
    datagrams_sent: AtomicU64,
    datagrams_received: AtomicU64,
    auth_rejections: AtomicU64,
}

impl AtomicQuicStats {
    pub(crate) fn inc_rpc_calls(&self) {
        self.rpc_calls.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_datagrams_sent(&self) {
        self.datagrams_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_datagrams_received(&self) {
        self.datagrams_received.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn inc_auth_rejections(&self) {
        self.auth_rejections.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> QuicStats {
        QuicStats {
            active_connections: self.active_connections.load(Ordering::Relaxed),
            rpc_calls: self.rpc_calls.load(Ordering::Relaxed),
            datagrams_sent: self.datagrams_sent.load(Ordering::Relaxed),
            datagrams_received: self.datagrams_received.load(Ordering::Relaxed),
            auth_rejections: self.auth_rejections.load(Ordering::Relaxed),
        }
    }
}

struct ActiveConnectionGuard(Arc<AtomicQuicStats>);

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.0.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Pinned mTLS QUIC endpoint capable of accepting RPC/datagrams and dialing pinned IPv6 peers.
#[derive(Clone, Debug)]
pub struct QuicControlNode {
    endpoint: Endpoint,
    identity: PinnedIdentity,
    stats: Arc<AtomicQuicStats>,
    stream_limit: Arc<Semaphore>,
    conn_limit: Arc<Semaphore>,
    handshake_limit: Arc<Semaphore>,
    conn_registry: Arc<ConnectionRegistry>,
    rpc_timeout: Duration,
    handshake_timeout: Duration,
}

impl QuicControlNode {
    fn from_endpoint(endpoint: Endpoint, identity: PinnedIdentity) -> Self {
        Self {
            endpoint,
            identity,
            stats: Arc::new(AtomicQuicStats::default()),
            stream_limit: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_STREAMS)),
            conn_limit: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_CONNECTIONS)),
            handshake_limit: Arc::new(Semaphore::new(DEFAULT_MAX_PENDING_HANDSHAKES)),
            conn_registry: Arc::new(ConnectionRegistry::new(
                DEFAULT_MAX_CONNECTIONS_PER_IP,
                DEFAULT_MAX_CONNECTIONS_PER_PEER,
            )),
            rpc_timeout: DEFAULT_RPC_TIMEOUT,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    /// Binds a pure-IPv6 mandatory-mTLS QUIC server endpoint presenting `identity`
    /// and enforcing `allowed_client_pins` (must be non-empty).
    pub fn bind_ipv6(
        bind_addr: SocketAddrV6,
        identity: PinnedIdentity,
        allowed_client_pins: Vec<[u8; 32]>,
    ) -> ProtocolResult<Self> {
        let server_cfg = build_server_config(&identity, allowed_client_pins)?;
        let endpoint = Endpoint::server(server_cfg, SocketAddr::V6(bind_addr))
            .map_err(|e| ProtocolError::Transport(format!("bind QUIC `{bind_addr}`: {e}")))?;
        Ok(Self::from_endpoint(endpoint, identity))
    }

    /// Creates a client-only pure-IPv6 mTLS QUIC endpoint presenting `identity`.
    pub fn client_ipv6(bind_addr: SocketAddrV6, identity: PinnedIdentity) -> ProtocolResult<Self> {
        let endpoint = Endpoint::client(SocketAddr::V6(bind_addr)).map_err(|e| {
            ProtocolError::Transport(format!("bind QUIC client `{bind_addr}`: {e}"))
        })?;
        Ok(Self::from_endpoint(endpoint, identity))
    }

    /// Overrides the bounded concurrent RPC stream semaphore limit.
    pub fn with_max_concurrent_streams(mut self, max_streams: usize) -> Self {
        self.stream_limit = Arc::new(Semaphore::new(max_streams.max(1)));
        self
    }

    /// Overrides the bounded concurrent active QUIC connection semaphore limit.
    pub fn with_max_concurrent_connections(mut self, max_connections: usize) -> Self {
        let capped = max_connections.max(1);
        self.conn_limit = Arc::new(Semaphore::new(capped));
        self
    }

    /// Overrides per-IP and per-peer connection limits enforced on accepted sessions.
    pub fn with_per_peer_limits(mut self, max_per_ip: usize, max_per_peer: usize) -> Self {
        self.conn_registry = Arc::new(ConnectionRegistry::new(max_per_ip, max_per_peer));
        self
    }

    /// Overrides the per-RPC stream read/write timeout.
    pub fn with_rpc_timeout(mut self, timeout: Duration) -> Self {
        self.rpc_timeout = timeout;
        self
    }

    /// Overrides the QUIC TLS handshake timeout.
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Returns actual local IPv6 socket address (useful when bound to port 0).
    pub fn local_addr(&self) -> ProtocolResult<SocketAddrV6> {
        match self
            .endpoint
            .local_addr()
            .map_err(|e| ProtocolError::Transport(format!("local_addr: {e}")))?
        {
            SocketAddr::V6(v6) => Ok(v6),
            SocketAddr::V4(v4) => Err(ProtocolError::InvalidAddress(format!(
                "expected pure IPv6 socket, got IPv4 {v4}"
            ))),
        }
    }

    /// Dials a pure-IPv6 server and verifies its certificate/SPKI against `expected_sha256_hex`.
    pub async fn connect_ipv6(
        &self,
        remote: SocketAddrV6,
        server_name: &str,
        expected_sha256_hex: &str,
    ) -> ProtocolResult<PinnedQuicConnection> {
        let expected = parse_sha256_hex(expected_sha256_hex)?;
        let client_cfg = build_client_config(&self.identity, expected)?;
        let connecting = self
            .endpoint
            .connect_with(client_cfg, SocketAddr::V6(remote), server_name)
            .map_err(|e| ProtocolError::Transport(format!("connect init `{remote}`: {e}")))?;
        let conn_res = tokio::time::timeout(self.handshake_timeout, connecting)
            .await
            .map_err(|_| {
                ProtocolError::Transport(format!("QUIC handshake `{remote}` timed out"))
            })?;
        match conn_res {
            Ok(conn) => {
                self.stats
                    .active_connections
                    .fetch_add(1, Ordering::Relaxed);
                Ok(PinnedQuicConnection::new(
                    conn,
                    Arc::clone(&self.stats),
                    self.rpc_timeout,
                ))
            }
            Err(err) => {
                let text = err.to_string();
                if text.contains("certificate") || text.contains("ApplicationVerificationFailure") {
                    self.stats.auth_rejections.fetch_add(1, Ordering::Relaxed);
                    return Err(ProtocolError::FingerprintMismatch {
                        expected: expected_sha256_hex.to_owned(),
                        actual: text,
                    });
                }
                Err(ProtocolError::Transport(format!(
                    "QUIC handshake `{remote}`: {text}"
                )))
            }
        }
    }

    /// Runs the server accept loop dispatching bounded RPC streams (JSON/CBOR) and datagrams.
    pub async fn serve<H, F>(self, handler: Arc<H>, on_datagram: F)
    where
        H: ControlHandler,
        F: Fn(TelemetryDatagram) + Send + Sync + 'static,
    {
        let on_datagram = Arc::new(on_datagram);
        while let Some(incoming) = self.endpoint.accept().await {
            let Ok(hs_permit) = Arc::clone(&self.handshake_limit).try_acquire_owned() else {
                self.stats.auth_rejections.fetch_add(1, Ordering::Relaxed);
                incoming.refuse();
                continue;
            };
            let Ok(conn_permit) = Arc::clone(&self.conn_limit).try_acquire_owned() else {
                self.stats.auth_rejections.fetch_add(1, Ordering::Relaxed);
                incoming.refuse();
                continue;
            };
            let Some(ip_guard) = self
                .conn_registry
                .try_acquire_ip(incoming.remote_address().ip())
            else {
                self.stats.auth_rejections.fetch_add(1, Ordering::Relaxed);
                incoming.refuse();
                continue;
            };

            let handler = Arc::clone(&handler);
            let on_datagram = Arc::clone(&on_datagram);
            let stats = Arc::clone(&self.stats);
            let stream_limit = Arc::clone(&self.stream_limit);
            let conn_registry = Arc::clone(&self.conn_registry);
            let rpc_timeout = self.rpc_timeout;
            let handshake_timeout = self.handshake_timeout;

            tokio::spawn(async move {
                let _conn_permit = conn_permit;
                let _ip_guard = ip_guard;
                let accepted = tokio::time::timeout(handshake_timeout, incoming).await;
                drop(hs_permit);
                let Ok(Ok(conn)) = accepted else {
                    stats.auth_rejections.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let Some(peer_spki) = extract_peer_spki_sha256(&conn) else {
                    stats.auth_rejections.fetch_add(1, Ordering::Relaxed);
                    conn.close(VarInt::from_u32(1), b"missing peer SPKI");
                    return;
                };
                let Some(_peer_guard) = conn_registry.try_acquire_peer(peer_spki) else {
                    stats.auth_rejections.fetch_add(1, Ordering::Relaxed);
                    conn.close(VarInt::from_u32(2), b"peer connection limit");
                    return;
                };
                stats.active_connections.fetch_add(1, Ordering::Relaxed);
                let _active_guard = ActiveConnectionGuard(Arc::clone(&stats));
                serve_connection(
                    conn,
                    peer_spki,
                    handler,
                    on_datagram,
                    stats,
                    stream_limit,
                    rpc_timeout,
                )
                .await;
            });
        }
    }

    /// Gracefully shuts down the underlying Quinn endpoint.
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }
}

impl ControlTransportMetrics for QuicControlNode {
    fn local_fingerprint_hex(&self) -> &str {
        self.identity.fingerprint_hex()
    }

    fn stats(&self) -> QuicStats {
        self.stats.snapshot()
    }
}
