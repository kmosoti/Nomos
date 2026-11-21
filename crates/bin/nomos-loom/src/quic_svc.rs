//! Pinned mTLS IPv6 QUIC RPC and RFC 9221 datagram handler wired to `KernelService`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomos_core::governor::ResourceSample;
use nomos_protocol::{ControlEnvelope, ControlOp, ControlReply, TelemetryDatagram};
use nomos_protocol_quic::ControlHandler;

use crate::governor_svc::GovernorService;
use crate::kernel_svc::KernelService;
use crate::lease::{LeaseBroker, unix_now_secs};

/// Loom server-side QUIC RPC handler wired to real `nomos_app::kernel::step`,
/// `nomos_warp::graph::Graph`, `Fence` enforcement, and `FsContentStore`/`FileLog`.
#[derive(Clone, Debug)]
pub struct LoomQuicHandler {
    node_name: String,
    governor: GovernorService,
    leases: LeaseBroker,
    kernel: Option<KernelService>,
}

impl LoomQuicHandler {
    /// Creates a handler bound to shared lease, governor, and optional kernel persistence services.
    pub fn new(node_name: String, governor: GovernorService, leases: LeaseBroker) -> Self {
        Self {
            node_name,
            governor,
            leases,
            kernel: None,
        }
    }

    /// Attaches a real durable [`KernelService`] (`nomos_app::kernel::step` + `Warp` + `FsStore`).
    pub fn with_kernel(mut self, kernel: KernelService) -> Self {
        self.kernel = Some(kernel);
        self
    }

    /// Ingests an incoming Cell [`TelemetryDatagram`] into the local governor and SSE bus.
    pub fn handle_datagram(&self, dgram: TelemetryDatagram) {
        let sample = ResourceSample {
            timestamp_ns: unix_now_secs().saturating_mul(1_000_000_000),
            cpu_some_avg10_bps: dgram.cpu_pressure_bps,
            cpu_full_avg10_bps: 0,
            mem_some_avg10_bps: dgram.memory_pressure_bps,
            mem_full_avg10_bps: 0,
            io_some_avg10_bps: dgram.io_pressure_bps,
            io_full_avg10_bps: 0,
            memory_used_bytes: 0,
            memory_total_bytes: 1,
        };
        let _ = self.governor.observe(&sample);
        self.governor.emit_event(
            "quic_datagram",
            format!(
                "node={} seq={} cpu_bps={}",
                dgram.node, dgram.seq, dgram.cpu_pressure_bps
            ),
        );
    }
}

impl ControlHandler for LoomQuicHandler {
    fn handle<'a>(
        &'a self,
        request: ControlEnvelope,
    ) -> Pin<Box<dyn Future<Output = ControlReply> + Send + 'a>> {
        Box::pin(async move {
            let gov = self.governor.status();
            let active_leases = self.leases.list_active().map(|l| l.len()).unwrap_or(0);
            let generation = request.generation.max(self.governor.generation()).max(
                self.kernel
                    .as_ref()
                    .map_or(0, KernelService::current_generation),
            );

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
                        generation,
                        digest_hex: Some(request.canon_digest_hex),
                        message: "ok".to_owned(),
                        payload_json: payload,
                    }
                }
                ControlOp::SubmitPlan => {
                    let Some(kernel) = &self.kernel else {
                        return ControlReply {
                            ok: false,
                            generation,
                            digest_hex: None,
                            message: "kernel service not configured".to_owned(),
                            payload_json: "{}".to_owned(),
                        };
                    };
                    let Some(plan_id) = request.plan_id.as_deref() else {
                        return ControlReply {
                            ok: false,
                            generation,
                            digest_hex: None,
                            message: "SubmitPlan requires plan_id".to_owned(),
                            payload_json: "{}".to_owned(),
                        };
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
                        Err(err) => ControlReply {
                            ok: false,
                            generation: kernel.current_generation(),
                            digest_hex: None,
                            message: err,
                            payload_json: "{}".to_owned(),
                        },
                    }
                }
                ControlOp::VerifyReceipt => {
                    let Some(kernel) = &self.kernel else {
                        return ControlReply {
                            ok: false,
                            generation,
                            digest_hex: None,
                            message: "kernel service not configured".to_owned(),
                            payload_json: "{}".to_owned(),
                        };
                    };
                    let Some(plan_id) = request.plan_id.as_deref() else {
                        return ControlReply {
                            ok: false,
                            generation,
                            digest_hex: None,
                            message: "VerifyReceipt requires plan_id".to_owned(),
                            payload_json: "{}".to_owned(),
                        };
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
                                generation: request.generation,
                                digest_hex: Some(request.canon_digest_hex),
                                message: detail,
                                payload_json: "{}".to_owned(),
                            }
                        }
                        Err(err) => ControlReply {
                            ok: false,
                            generation: kernel.current_generation(),
                            digest_hex: None,
                            message: err,
                            payload_json: "{}".to_owned(),
                        },
                    }
                }
            }
        })
    }
}

/// Spawns the background datagram/RPC accept loop on an already-bound `QuicControlNode`.
pub fn spawn_quic_server(
    node: nomos_protocol_quic::QuicControlNode,
    handler: Arc<LoomQuicHandler>,
) -> tokio::task::JoinHandle<()> {
    let dg_handler = Arc::clone(&handler);
    tokio::spawn(async move {
        node.serve(handler, move |dgram| dg_handler.handle_datagram(dgram))
            .await;
    })
}
