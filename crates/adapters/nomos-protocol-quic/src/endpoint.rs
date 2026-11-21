//! Pure-IPv6 QUIC RPC server and client endpoint with SHA-256 fingerprint pinning.

use std::future::Future;
use std::net::{SocketAddr, SocketAddrV6};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use nomos_protocol::{
    ControlEnvelope, ControlReply, ControlTransportMetrics, ProtocolError, ProtocolResult,
    QuicStats, TelemetryDatagram,
};
use quinn::{Connection, Endpoint};

use crate::cert::{PinnedIdentity, parse_sha256_hex};
use crate::frame::{read_json_frame, write_json_frame};
use crate::tls::{build_client_config, build_server_config};

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

/// Pinned QUIC endpoint capable of accepting server RPC/datagrams and dialing pinned IPv6 peers.
#[derive(Clone, Debug)]
pub struct QuicControlNode {
    endpoint: Endpoint,
    identity: PinnedIdentity,
    stats: Arc<AtomicQuicStats>,
}

impl QuicControlNode {
    /// Binds a pure-IPv6 QUIC endpoint presenting `identity`.
    pub fn bind_ipv6(
        bind_addr: SocketAddrV6,
        identity: PinnedIdentity,
        allowed_client_pins: Option<Vec<[u8; 32]>>,
    ) -> ProtocolResult<Self> {
        let server_cfg = build_server_config(&identity, allowed_client_pins)?;
        let endpoint = Endpoint::server(server_cfg, SocketAddr::V6(bind_addr))
            .map_err(|e| ProtocolError::Transport(format!("bind QUIC `{bind_addr}`: {e}")))?;
        Ok(Self {
            endpoint,
            identity,
            stats: Arc::new(AtomicQuicStats::default()),
        })
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

    /// Dials a pure-IPv6 server and verifies its certificate against `expected_sha256_hex`.
    pub async fn connect_ipv6(
        &self,
        remote: SocketAddrV6,
        server_name: &str,
        expected_sha256_hex: &str,
    ) -> ProtocolResult<PinnedQuicConnection> {
        let expected = parse_sha256_hex(expected_sha256_hex)?;
        let client_cfg = build_client_config(Some(&self.identity), expected)?;
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

    /// Runs the server accept loop dispatching bidirectional RPC streams and RFC 9221 datagrams.
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
                        if let Ok(msg) = serde_json::from_slice::<TelemetryDatagram>(&bytes) {
                            dgram_stats
                                .datagrams_received
                                .fetch_add(1, Ordering::Relaxed);
                            dgram_cb(msg);
                        }
                    }
                });

                while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                    let handler = Arc::clone(&handler);
                    let stats = Arc::clone(&stats);
                    tokio::spawn(async move {
                        if let Ok(req) = read_json_frame::<ControlEnvelope>(&mut recv).await {
                            let reply = handler.handle(req).await;
                            if write_json_frame(&mut send, &reply).await.is_ok() {
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

/// Active authenticated QUIC connection to a fingerprint-verified peer.
#[derive(Clone, Debug)]
pub struct PinnedQuicConnection {
    conn: Connection,
    stats: Arc<AtomicQuicStats>,
}

impl PinnedQuicConnection {
    /// Sends a single RPC envelope over a fresh bidirectional QUIC stream and awaits reply.
    pub async fn rpc(&self, request: &ControlEnvelope) -> ProtocolResult<ControlReply> {
        let (mut send, mut recv) = self
            .conn
            .open_bi()
            .await
            .map_err(|e| ProtocolError::Transport(format!("open_bi: {e}")))?;
        write_json_frame(&mut send, request).await?;
        send.finish()
            .map_err(|e| ProtocolError::Transport(format!("finish RPC send: {e}")))?;
        let reply = read_json_frame(&mut recv).await?;
        self.stats.rpc_calls.fetch_add(1, Ordering::Relaxed);
        Ok(reply)
    }

    /// Sends an unreliable RFC 9221 QUIC datagram over the authenticated session.
    pub fn send_datagram(&self, dgram: &TelemetryDatagram) -> ProtocolResult<()> {
        let payload = serde_json::to_vec(dgram)
            .map_err(|e| ProtocolError::Serialization(format!("encode datagram: {e}")))?;
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
        serde_json::from_slice(&bytes)
            .map_err(|e| ProtocolError::Serialization(format!("decode datagram: {e}")))
    }
}
