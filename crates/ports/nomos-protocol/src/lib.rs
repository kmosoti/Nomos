//! Protocol port: transport-neutral Loom ↔ Cell RPC and datagram contracts.

use core::fmt;
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::Digest;
use serde::{Deserialize, Serialize};

/// Result alias for control protocol operations.
pub type ProtocolResult<T> = Result<T, ProtocolError>;

/// Errors raised by pinned control-plane protocol transport adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    /// Invalid IPv6 socket address or server name.
    InvalidAddress(String),
    /// Certificate generation, encoding, or parsing error.
    Certificate(String),
    /// Peer TLS certificate SHA-256 fingerprint did not match expected pin.
    FingerprintMismatch {
        /// Expected SHA-256 certificate digest (lowercase hex).
        expected: String,
        /// Observed peer certificate digest (lowercase hex).
        actual: String,
    },
    /// Wire frame exceeded configured maximum frame size or truncated prematurely.
    Framing(String),
    /// Request or response JSON payload serialization failed.
    Serialization(String),
    /// Underlying QUIC stream, datagram, or connection failure.
    Transport(String),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::InvalidAddress(msg) => write!(f, "invalid control address: {msg}"),
            ProtocolError::Certificate(msg) => write!(f, "certificate error: {msg}"),
            ProtocolError::FingerprintMismatch { expected, actual } => write!(
                f,
                "certificate SHA-256 pin mismatch: expected {expected}, got {actual}"
            ),
            ProtocolError::Framing(msg) => write!(f, "protocol framing error: {msg}"),
            ProtocolError::Serialization(msg) => write!(f, "protocol serialization error: {msg}"),
            ProtocolError::Transport(msg) => write!(f, "protocol transport error: {msg}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// Operation kind carried over bidirectional Loom ↔ Cell RPC streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOp {
    /// Queries peer readiness, generation watermark, and active lease count.
    Status,
    /// Dispatches a compiled Canon or enforcement plan identifier and digest.
    SubmitPlan,
    /// Acknowledges verification or enforcement receipts.
    VerifyReceipt,
    /// Heartbeat carrying current generation fence.
    Heartbeat,
    /// Requests host telemetry snapshot.
    TelemetrySnapshot,
}

/// Authenticated RPC request envelope referencing domain [`PlanId`], [`Generation`], and [`Digest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlEnvelope {
    /// Operation requested on the control stream.
    pub op: ControlOp,
    /// Optional domain plan identifier string validated via [`PlanId`].
    pub plan_id: Option<String>,
    /// Authority generation fence value.
    pub generation: u64,
    /// Canonical SHA-256 / BLAKE3 64-hex digest bound to the operation.
    pub canon_digest_hex: String,
    /// JSON payload body (non-secret metadata only; invariant N8).
    pub payload_json: String,
}

impl ControlEnvelope {
    /// Constructs an envelope from typed `nomos-core` identifiers.
    pub fn new(
        op: ControlOp,
        plan_id: Option<&PlanId>,
        generation: Generation,
        digest: &Digest,
        payload_json: impl Into<String>,
    ) -> Self {
        Self {
            op,
            plan_id: plan_id.map(|id| id.as_str().to_owned()),
            generation: generation.0,
            canon_digest_hex: digest_to_hex(digest),
            payload_json: payload_json.into(),
        }
    }

    /// Parses [`Self::plan_id`] as a validated [`PlanId`] when present.
    pub fn typed_plan_id(&self) -> Option<PlanId> {
        self.plan_id.as_deref().and_then(PlanId::new)
    }

    /// Returns typed [`Generation`].
    pub fn typed_generation(&self) -> Generation {
        Generation(self.generation)
    }

    /// Parses [`Self::canon_digest_hex`] as a typed [`Digest`].
    pub fn typed_digest(&self) -> Option<Digest> {
        Digest::from_hex(&self.canon_digest_hex).ok()
    }
}

/// Authenticated RPC reply envelope returned over the bidirectional stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlReply {
    /// True when the operation was accepted by the remote peer.
    pub ok: bool,
    /// Peer's current generation watermark.
    pub generation: u64,
    /// Optional resulting digest hex.
    pub digest_hex: Option<String>,
    /// Diagnostic message (never contains secrets).
    pub message: String,
    /// JSON response body.
    pub payload_json: String,
}

/// Unreliable RFC 9221 control datagram conveying pressure telemetry or heartbeats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryDatagram {
    /// Logical node label.
    pub node: String,
    /// Current authority generation.
    pub generation: u64,
    /// Monotonic datagram sequence counter.
    pub seq: u64,
    /// CPU PSI `some avg10` scaled to basis points (`0..=10000`).
    pub cpu_pressure_bps: u16,
    /// Memory PSI `some avg10` scaled to basis points (`0..=10000`).
    pub memory_pressure_bps: u16,
    /// IO PSI `some avg10` scaled to basis points (`0..=10000`).
    pub io_pressure_bps: u16,
    /// Active workload leases hosted on the node.
    pub active_leases: u32,
}

/// Runtime connection and datagram counters for QUIC transport telemetry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicStats {
    /// Currently open authenticated QUIC connections.
    pub active_connections: u64,
    /// Completed RPC round-trips.
    pub rpc_calls: u64,
    /// Unreliable datagrams transmitted.
    pub datagrams_sent: u64,
    /// Unreliable datagrams received.
    pub datagrams_received: u64,
    /// Connections rejected due to certificate fingerprint mismatch.
    pub auth_rejections: u64,
}

/// Port contract for querying transport telemetry and pin identity.
pub trait ControlTransportMetrics {
    /// Returns SHA-256 certificate fingerprint (lowercase hex) presented by this node.
    fn local_fingerprint_hex(&self) -> &str;

    /// Snapshot of current QUIC transport statistics.
    fn stats(&self) -> QuicStats;
}

fn digest_to_hex(digest: &Digest) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest.as_bytes() {
        use core::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}
