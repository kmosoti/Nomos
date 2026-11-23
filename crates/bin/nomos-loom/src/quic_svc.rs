//! Pinned mTLS IPv6 QUIC RPC and RFC 9221 datagram handler wired to `KernelService`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nomos_protocol::{ControlEnvelope, ControlOp, ControlReply, TelemetryDatagram};
use nomos_protocol_quic::ControlHandler;

use crate::governor_svc::GovernorService;
use crate::kernel_svc::KernelService;
use crate::lease::LeaseBroker;
use crate::peer_policy::{AuthorizedPeer, PeerRole, RemoteTelemetrySample, TelemetryTracker};

/// Loom server-side QUIC RPC handler wired to real `nomos_app::kernel::step`,
/// `nomos_warp::graph::Graph`, `Fence` enforcement, and `FsContentStore`/`FileLog`.
#[derive(Clone)]
pub struct LoomQuicHandler {
    node_name: String,
    governor: GovernorService,
    leases: LeaseBroker,
    kernel: Option<KernelService>,
    peers: HashMap<[u8; 32], AuthorizedPeer>,
    telemetry: Arc<Mutex<TelemetryTracker>>,
}

impl core::fmt::Debug for LoomQuicHandler {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LoomQuicHandler")
            .field("node_name", &self.node_name)
            .field("authorized_peers", &self.peers.len())
            .finish_non_exhaustive()
    }
}

impl LoomQuicHandler {
    /// Creates a handler bound to shared lease, governor, and optional kernel services.
    ///
    /// Starts with zero authorized peers (fails closed until pins are registered).
    pub fn new(node_name: String, governor: GovernorService, leases: LeaseBroker) -> Self {
        Self {
            node_name,
            governor,
            leases,
            kernel: None,
            peers: HashMap::new(),
            telemetry: Arc::new(Mutex::new(TelemetryTracker::default())),
        }
    }

    /// Attaches a real durable [`KernelService`] (`nomos_app::kernel::step` + `Warp` + `FsStore`).
    pub fn with_kernel(mut self, kernel: KernelService) -> Self {
        self.kernel = Some(kernel);
        self
    }

    /// Registers a set of authorized peer SPKI policies.
    pub fn with_authorized_peers(
        mut self,
        peers: impl IntoIterator<Item = AuthorizedPeer>,
    ) -> Self {
        for peer in peers {
            self.peers.insert(peer.pin, peer);
        }
        self
    }

    /// Registers an [`PeerRole::Authority`] SPKI SHA-256 pin authorized to invoke `SubmitPlan`.
    pub fn with_authority_pin(mut self, pin: [u8; 32]) -> Self {
        self.peers.insert(
            pin,
            AuthorizedPeer {
                pin,
                role: PeerRole::Authority,
                node_id: None,
            },
        );
        self
    }

    /// Registers a least-privilege [`PeerRole::Cell`] SPKI SHA-256 pin.
    pub fn with_cell_pin(mut self, node_id: Option<String>, pin: [u8; 32]) -> Self {
        self.peers.insert(
            pin,
            AuthorizedPeer {
                pin,
                role: PeerRole::Cell,
                node_id,
            },
        );
        self
    }

    /// Returns a snapshot of authenticated telemetry reports keyed by verified node ID.
    pub fn remote_telemetry_snapshot(&self) -> Vec<RemoteTelemetrySample> {
        self.telemetry
            .lock()
            .map(|t| t.latest_by_node.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Legacy unauthenticated datagram entrypoint (fails closed via zero peer pin).
    pub fn handle_datagram(&self, dgram: TelemetryDatagram) {
        self.handle_telemetry_authenticated([0u8; 32], dgram);
    }

    fn server_generation(&self) -> u64 {
        self.governor.generation().max(
            self.kernel
                .as_ref()
                .map_or(0, KernelService::current_generation),
        )
    }
}

impl ControlHandler for LoomQuicHandler {
    fn handle<'a>(
        &'a self,
        request: ControlEnvelope,
    ) -> Pin<Box<dyn Future<Output = ControlReply> + Send + 'a>> {
        self.handle_authenticated([0u8; 32], request)
    }

    fn handle_telemetry(&self, dg: TelemetryDatagram) {
        self.handle_telemetry_authenticated([0u8; 32], dg);
    }

    fn handle_telemetry_authenticated(&self, peer_spki_sha256: [u8; 32], dgram: TelemetryDatagram) {
        let Some(peer) = self.peers.get(&peer_spki_sha256) else {
            return;
        };
        let Ok(mut tracker) = self.telemetry.lock() else {
            return;
        };
        if let Ok(accepted) = tracker.accept(
            peer_spki_sha256,
            peer.node_id.as_deref(),
            &dgram,
            Instant::now(),
        ) {
            self.governor.emit_event(
                "quic_datagram",
                format!(
                    "node={} gen={} seq={} cpu_bps={}",
                    accepted.node, accepted.generation, accepted.seq, accepted.cpu_pressure_bps
                ),
            );
        }
    }

    fn handle_authenticated<'a>(
        &'a self,
        peer_spki_sha256: [u8; 32],
        request: ControlEnvelope,
    ) -> Pin<Box<dyn Future<Output = ControlReply> + Send + 'a>> {
        Box::pin(async move {
            let server_gen = self.server_generation();
            let Some(peer) = self.peers.get(&peer_spki_sha256) else {
                return reject_reply(server_gen, "unauthenticated or unknown QUIC peer pin");
            };

            let gov = self.governor.status();
            let active_leases = self.leases.list_active().map(|l| l.len()).unwrap_or(0);

            match request.op {
                ControlOp::Status | ControlOp::Heartbeat | ControlOp::TelemetrySnapshot => {
                    let payload = serde_json::json!({
                        "coordinator": self.node_name,
                        "tier": gov.tier,
                        "admit_batch": gov.admit_batch,
                        "active_leases": active_leases,
                        "journal_records": self.kernel.as_ref().map_or(0, KernelService::journal_len),
                    })
                    .to_string();
                    ControlReply {
                        ok: true,
                        generation: server_gen,
                        digest_hex: Some(request.canon_digest_hex),
                        message: "ok".to_owned(),
                        payload_json: payload,
                    }
                }
                ControlOp::SubmitPlan => {
                    if peer.role != PeerRole::Authority {
                        return reject_reply(
                            server_gen,
                            "forbidden: peer role Cell cannot execute SubmitPlan",
                        );
                    }
                    let Some(kernel) = &self.kernel else {
                        return reject_reply(server_gen, "kernel service not configured");
                    };
                    let Some(plan_id) = request.plan_id.as_deref() else {
                        return reject_reply(server_gen, "SubmitPlan requires plan_id");
                    };
                    match kernel.submit_plan(
                        plan_id,
                        request.generation,
                        Some(&request.canon_digest_hex),
                        request.payload_json.as_bytes(),
                        true,
                    ) {
                        Ok(summary) => {
                            self.governor.emit_event(
                                "plan_accepted",
                                format!(
                                    "plan_id={} gen={} vertices={} edges={}",
                                    summary.plan_id,
                                    summary.generation,
                                    summary.warp_vertices,
                                    summary.warp_edges
                                ),
                            );
                            ControlReply {
                                ok: true,
                                generation: summary.generation,
                                digest_hex: Some(summary.canon_digest_hex.clone()),
                                message: "plan accepted and persisted".to_owned(),
                                payload_json: serde_json::to_string(&summary)
                                    .unwrap_or_else(|_| "{}".to_owned()),
                            }
                        }
                        Err(err) => reject_reply(kernel.current_generation(), err),
                    }
                }
                ControlOp::VerifyReceipt => {
                    let Some(kernel) = &self.kernel else {
                        return reject_reply(server_gen, "kernel service not configured");
                    };
                    let Some(plan_id) = request.plan_id.as_deref() else {
                        return reject_reply(server_gen, "VerifyReceipt requires plan_id");
                    };
                    match kernel.verify_receipt(plan_id, request.generation, &request.payload_json)
                    {
                        Ok(detail) => {
                            self.governor.emit_event(
                                "receipt_verified",
                                format!("plan_id={plan_id} gen={} {detail}", request.generation),
                            );
                            ControlReply {
                                ok: true,
                                generation: kernel.current_generation(),
                                digest_hex: Some(request.canon_digest_hex),
                                message: detail,
                                payload_json: "{}".to_owned(),
                            }
                        }
                        Err(err) => reject_reply(kernel.current_generation(), err),
                    }
                }
            }
        })
    }
}

fn reject_reply(generation: u64, message: impl Into<String>) -> ControlReply {
    ControlReply {
        ok: false,
        generation,
        digest_hex: None,
        message: message.into(),
        payload_json: "{}".to_owned(),
    }
}

/// Spawns the background datagram/RPC accept loop on an already-bound `QuicControlNode`.
pub fn spawn_quic_server(
    node: nomos_protocol_quic::QuicControlNode,
    handler: Arc<LoomQuicHandler>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        node.serve(handler, |_| {}).await;
    })
}
