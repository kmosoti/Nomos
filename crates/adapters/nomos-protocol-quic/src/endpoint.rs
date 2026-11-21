//! Pure-IPv6 mandatory-mTLS QUIC RPC server and client endpoint with bounded stream
//! concurrency (`Semaphore`), per-RPC timeouts, and JSON/CBOR framing.

use std::future::Future;
use std::net::{SocketAddr, SocketAddrV6};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use nomos_protocol::{
    ControlEnvelope, ControlReply, ControlTransportMetrics, ProtocolError, ProtocolResult,
    QuicStats, TelemetryDatagram,
};
use quinn::{Connection, Endpoint};
use tokio::sync::Semaphore;

use crate::cert::{PinnedIdentity, parse_sha256_hex};
use crate::frame::{WireFormat, read_cbor_frame, read_frame_auto, read_json_frame, write_frame};
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

/// Pinned mTLS QUIC endpoint capable of accepting RPC/datagrams and dialing pinned IPv6 peers.
#[derive(Clone, Debug)]
pub struct QuicControlNode {
    endpoint: Endpoint,
    identity: PinnedIdentity,
    stats: Arc<AtomicQuicStats>,
    stream_limit: Arc<Semaphore>,
    rpc_timeout: Duration,
}

impl QuicControlNode {
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
        Ok(Self {
            endpoint,
            identity,
            stats: Arc::new(AtomicQuicStats::default()),
            stream_limit: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_STREAMS)),
            rpc_timeout: DEFAULT_RPC_TIMEOUT,
        })
    }

    /// Creates a client-only pure-IPv6 mTLS QUIC endpoint presenting `identity`.
    pub fn client_ipv6(bind_addr: SocketAddrV6, identity: PinnedIdentity) -> ProtocolResult<Self> {
        let endpoint = Endpoint::client(SocketAddr::V6(bind_addr)).map_err(|e| {
            ProtocolError::Transport(format!("bind QUIC client `{bind_addr}`: {e}"))
        })?;
        Ok(Self {
            endpoint,
            identity,
            stats: Arc::new(AtomicQuicStats::default()),
            stream_limit: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_STREAMS)),
            rpc_timeout: DEFAULT_RPC_TIMEOUT,
        })
    }

    /// Overrides the bounded concurrent RPC stream semaphore limit.
    pub fn with_max_concurrent_streams(mut self, max_streams: usize) -> Self {
        self.stream_limit = Arc::new(Semaphore::new(max_streams.max(1)));
        self
    }

    /// Overrides the per-RPC stream read/write timeout.
    pub fn with_rpc_timeout(mut self, timeout: Duration) -> Self {
        self.rpc_timeout = timeout;
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
        match connecting.await {
            Ok(conn) => {
                self.stats
                    .active_connections
                    .fetch_add(1, Ordering::Relaxed);
                Ok(PinnedQuicConnection {
                    conn,
                    stats: Arc::clone(&self.stats),
                    rpc_timeout: self.rpc_timeout,
                })
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
            let handler = Arc::clone(&handler);
            let on_datagram = Arc::clone(&on_datagram);
            let stats = Arc::clone(&self.stats);
            let stream_limit = Arc::clone(&self.stream_limit);
            let rpc_timeout = self.rpc_timeout;
            tokio::spawn(async move {
                let Ok(conn) = incoming.await else {
                    stats.auth_rejections.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                stats.active_connections.fetch_add(1, Ordering::Relaxed);

                let dgram_conn = conn.clone();
                let dgram_stats = Arc::clone(&stats);
                let dgram_cb = Arc::clone(&on_datagram);
                let dgram_task = tokio::spawn(async move {
                    while let Ok(bytes) = dgram_conn.read_datagram().await {
                        let parsed = if bytes.first() == Some(&b'{') {
                            serde_json::from_slice::<TelemetryDatagram>(&bytes).ok()
                        } else {
                            ciborium::from_reader::<TelemetryDatagram, _>(bytes.as_ref()).ok()
                        };
                        if let Some(msg) = parsed {
                            dgram_stats
                                .datagrams_received
                                .fetch_add(1, Ordering::Relaxed);
                            dgram_cb(msg);
                        }
                    }
                });

                while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                    let Ok(permit) = Arc::clone(&stream_limit).acquire_owned().await else {
                        break;
                    };
                    let handler = Arc::clone(&handler);
                    let stats = Arc::clone(&stats);
                    tokio::spawn(async move {
                        let _permit = permit;
                        let read_res = tokio::time::timeout(
                            rpc_timeout,
                            read_frame_auto::<ControlEnvelope>(&mut recv),
                        )
                        .await;
                        if let Ok(Ok((fmt, req))) = read_res {
                            let reply = handler.handle(req).await;
                            let write_res = tokio::time::timeout(
                                rpc_timeout,
                                write_frame(&mut send, fmt, &reply),
                            )
                            .await;
                            if matches!(write_res, Ok(Ok(()))) {
                                stats.rpc_calls.fetch_add(1, Ordering::Relaxed);
                            }
                            let _ = send.finish();
                        }
                    });
                }
                dgram_task.abort();
                stats.active_connections.fetch_sub(1, Ordering::Relaxed);
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

/// Active authenticated mTLS QUIC connection to a fingerprint-verified peer.
#[derive(Clone, Debug)]
pub struct PinnedQuicConnection {
    conn: Connection,
    stats: Arc<AtomicQuicStats>,
    rpc_timeout: Duration,
}

impl PinnedQuicConnection {
    /// Sends a JSON-framed RPC envelope over a fresh bidirectional QUIC stream.
    pub async fn rpc(&self, request: &ControlEnvelope) -> ProtocolResult<ControlReply> {
        self.rpc_with_format(request, WireFormat::Json).await
    }

    /// Sends a canonical binary CBOR-framed RPC envelope over a fresh bidirectional QUIC stream.
    pub async fn rpc_cbor(&self, request: &ControlEnvelope) -> ProtocolResult<ControlReply> {
        self.rpc_with_format(request, WireFormat::Cbor).await
    }

    /// Sends an RPC envelope in `format` with bounded per-RPC timeout.
    pub async fn rpc_with_format(
        &self,
        request: &ControlEnvelope,
        format: WireFormat,
    ) -> ProtocolResult<ControlReply> {
        let (mut send, mut recv) = self
            .conn
            .open_bi()
            .await
            .map_err(|e| ProtocolError::Transport(format!("open_bi: {e}")))?;
        tokio::time::timeout(self.rpc_timeout, write_frame(&mut send, format, request))
            .await
            .map_err(|_| ProtocolError::Transport("RPC write timed out".into()))??;
        send.finish()
            .map_err(|e| ProtocolError::Transport(format!("finish RPC send: {e}")))?;
        let reply = match format {
            WireFormat::Json => tokio::time::timeout(self.rpc_timeout, read_json_frame(&mut recv))
                .await
                .map_err(|_| ProtocolError::Transport("RPC read timed out".into()))??,
            WireFormat::Cbor => tokio::time::timeout(self.rpc_timeout, read_cbor_frame(&mut recv))
                .await
                .map_err(|_| ProtocolError::Transport("RPC read timed out".into()))??,
        };
        self.stats.rpc_calls.fetch_add(1, Ordering::Relaxed);
        Ok(reply)
    }

    /// Sends an unreliable RFC 9221 QUIC datagram (JSON) over the authenticated session.
    pub fn send_datagram(&self, dgram: &TelemetryDatagram) -> ProtocolResult<()> {
        let payload = serde_json::to_vec(dgram)
            .map_err(|e| ProtocolError::Serialization(format!("encode datagram: {e}")))?;
        self.conn
            .send_datagram(Bytes::from(payload))
            .map_err(|e| ProtocolError::Transport(format!("send_datagram: {e}")))?;
        self.stats.datagrams_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Sends an unreliable RFC 9221 QUIC datagram encoded in canonical CBOR.
    pub fn send_datagram_cbor(&self, dgram: &TelemetryDatagram) -> ProtocolResult<()> {
        let mut payload = Vec::new();
        ciborium::into_writer(dgram, &mut payload)
            .map_err(|e| ProtocolError::Serialization(format!("encode CBOR datagram: {e}")))?;
        self.conn
            .send_datagram(Bytes::from(payload))
            .map_err(|e| ProtocolError::Transport(format!("send_datagram: {e}")))?;
        self.stats.datagrams_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Reads the next unreliable datagram from the connection.
    pub async fn recv_datagram(&self) -> ProtocolResult<TelemetryDatagram> {
        let bytes = self
            .conn
            .read_datagram()
            .await
            .map_err(|e| ProtocolError::Transport(format!("read_datagram: {e}")))?;
        self.stats
            .datagrams_received
            .fetch_add(1, Ordering::Relaxed);
        if bytes.first() == Some(&b'{') {
            serde_json::from_slice(&bytes)
                .map_err(|e| ProtocolError::Serialization(format!("decode datagram JSON: {e}")))
        } else {
            ciborium::from_reader(bytes.as_ref())
                .map_err(|e| ProtocolError::Serialization(format!("decode datagram CBOR: {e}")))
        }
    }
}
