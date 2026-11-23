//! Active authenticated mTLS QUIC client session wrapper (`PinnedQuicConnection`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use nomos_protocol::{
    ControlEnvelope, ControlReply, ProtocolError, ProtocolResult, TelemetryDatagram,
};
use quinn::Connection;
use tokio::sync::Semaphore;

use crate::endpoint::{AtomicQuicStats, ControlHandler};
use crate::frame::{WireFormat, read_cbor_frame, read_frame_auto, read_json_frame, write_frame};
use crate::limits::{DatagramLimiter, MAX_DATAGRAM_BYTES};

/// Active authenticated mTLS QUIC connection to a fingerprint-verified peer.
#[derive(Clone, Debug)]
pub struct PinnedQuicConnection {
    conn: Connection,
    stats: Arc<AtomicQuicStats>,
    rpc_timeout: Duration,
}

impl PinnedQuicConnection {
    pub(crate) fn new(
        conn: Connection,
        stats: Arc<AtomicQuicStats>,
        rpc_timeout: Duration,
    ) -> Self {
        Self {
            conn,
            stats,
            rpc_timeout,
        }
    }

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
        self.stats.inc_rpc_calls();
        Ok(reply)
    }

    /// Sends an unreliable RFC 9221 QUIC datagram (JSON) over the authenticated session.
    pub fn send_datagram(&self, dgram: &TelemetryDatagram) -> ProtocolResult<()> {
        let payload = serde_json::to_vec(dgram)
            .map_err(|e| ProtocolError::Serialization(format!("encode datagram: {e}")))?;
        self.conn
            .send_datagram(Bytes::from(payload))
            .map_err(|e| ProtocolError::Transport(format!("send_datagram: {e}")))?;
        self.stats.inc_datagrams_sent();
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
        self.stats.inc_datagrams_sent();
        Ok(())
    }

    /// Reads the next unreliable datagram from the connection.
    pub async fn recv_datagram(&self) -> ProtocolResult<TelemetryDatagram> {
        let bytes = self
            .conn
            .read_datagram()
            .await
            .map_err(|e| ProtocolError::Transport(format!("read_datagram: {e}")))?;
        self.stats.inc_datagrams_received();
        if bytes.first() == Some(&b'{') {
            serde_json::from_slice(&bytes)
                .map_err(|e| ProtocolError::Serialization(format!("decode datagram JSON: {e}")))
        } else {
            ciborium::from_reader(bytes.as_ref())
                .map_err(|e| ProtocolError::Serialization(format!("decode datagram CBOR: {e}")))
        }
    }
}

pub(crate) async fn serve_connection<H, F>(
    conn: Connection,
    peer_spki: [u8; 32],
    handler: Arc<H>,
    on_datagram: Arc<F>,
    stats: Arc<AtomicQuicStats>,
    stream_limit: Arc<Semaphore>,
    rpc_timeout: Duration,
) where
    H: ControlHandler,
    F: Fn(TelemetryDatagram) + Send + Sync + 'static,
{
    let dgram_conn = conn.clone();
    let dgram_stats = Arc::clone(&stats);
    let dgram_cb = Arc::clone(&on_datagram);
    let dgram_handler = Arc::clone(&handler);
    let dgram_task = tokio::spawn(async move {
        let mut limiter = DatagramLimiter::default();
        while let Ok(bytes) = dgram_conn.read_datagram().await {
            if bytes.len() > MAX_DATAGRAM_BYTES || !limiter.allow(Instant::now()) {
                dgram_stats.inc_auth_rejections();
                continue;
            }
            let parsed = if bytes.first() == Some(&b'{') {
                serde_json::from_slice::<TelemetryDatagram>(&bytes).ok()
            } else {
                ciborium::from_reader::<TelemetryDatagram, _>(bytes.as_ref()).ok()
            };
            if let Some(msg) = parsed {
                dgram_stats.inc_datagrams_received();
                dgram_cb(msg.clone());
                dgram_handler.handle_telemetry_authenticated(peer_spki, msg);
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
            let read_res =
                tokio::time::timeout(rpc_timeout, read_frame_auto::<ControlEnvelope>(&mut recv))
                    .await;
            if let Ok(Ok((fmt, req))) = read_res {
                let reply = handler.handle_authenticated(peer_spki, req).await;
                let write_res =
                    tokio::time::timeout(rpc_timeout, write_frame(&mut send, fmt, &reply)).await;
                if matches!(write_res, Ok(Ok(()))) {
                    stats.inc_rpc_calls();
                }
                let _ = send.finish();
            }
        });
    }
    dgram_task.abort();
}
