//! Real transition-kernel, Warp graph, Fence, and `FsContentStore`/`FileLog` coordinator service.

use std::path::Path;
use std::sync::{Arc, Mutex};

use nomos_app::kernel::{Canon as KernelCanon, Event, Input, KernelSnapshot, Plan, Policy, step};
use nomos_canon::artifact::{Profile, Reader, canon_id, decode};
use nomos_core::effect::{EffectKey, Receipt};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{Digest, ResourceKey, ResourcePath};
use nomos_store::{ContentStore, EventLog};
use nomos_store_fs::{FileLog, FsContentStore};
use nomos_warp::graph::{Graph, Vertex};
use serde::{Deserialize, Serialize};

/// Non-secret summary returned when evaluating (`trace`) or enforcing a Canon plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSummary {
    /// Accepted or evaluated plan identifier.
    pub plan_id: String,
    /// Generation watermark checked against `Fence`.
    pub generation: u64,
    /// Canonical SHA-256 digest hex of the verified Canon artifact.
    pub canon_digest_hex: String,
    /// Number of vertices compiled into the Warp DAG.
    pub warp_vertices: usize,
    /// Number of dependency edges compiled into the Warp DAG.
    pub warp_edges: usize,
    /// True if persisted to `FsContentStore` and `FileLog` (`enforce`), false for `trace`.
    pub persisted: bool,
    /// Number of kernel events emitted by `nomos_app::kernel::step`.
    pub kernel_events: usize,
}

/// Request payload for `ControlOp::VerifyReceipt`.
#[derive(Clone, Debug, Deserialize)]
pub struct ReceiptPayload {
    /// Target resource path (for example `/etc/nomos/demo.conf`).
    pub resource_path: String,
    /// Reconciliation round iteration index (defaults to 0).
    #[serde(default)]
    pub iteration: u32,
    /// Outcome string: `"accepted"`, `"started"`, `"completed"`, `"failed"`, or `"refused"`.
    pub outcome: String,
    /// Whether `"completed"` changed the resource (defaults to true).
    #[serde(default = "default_changed")]
    pub changed: bool,
}

const fn default_changed() -> bool {
    true
}

/// Thread-safe kernel coordinator backed by `nomos-app::kernel::step`, `nomos-warp`,
/// `Fence`, and `nomos-store-fs` (`FsContentStore` + durable `FileLog<Input>`).
#[derive(Clone)]
pub struct KernelService {
    inner: Arc<Mutex<KernelInner>>,
}

struct KernelInner {
    snapshot: KernelSnapshot,
    store: FsContentStore,
    journal: FileLog<Input>,
}

impl core::fmt::Debug for KernelService {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("KernelService").finish_non_exhaustive()
    }
}

impl KernelService {
    /// Opens or creates durable `FsContentStore` + `FileLog<Input>` under `state_dir`
    /// and replays all journaled inputs into `KernelSnapshot`.
    pub fn open(state_dir: &Path) -> Result<Self, String> {
        let store = FsContentStore::open(state_dir)
            .map_err(|e| format!("open FsContentStore `{}`: {e:?}", state_dir.display()))?;
        let log_path = state_dir.join("journal.log");
        let (journal, _recovery) = FileLog::<Input>::open(&log_path)
            .map_err(|e| format!("open FileLog `{}`: {e}", log_path.display()))?;
        let mut snapshot = KernelSnapshot::new();
        for input in journal.events() {
            snapshot = step(&snapshot, input).snapshot;
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(KernelInner {
                snapshot,
                store,
                journal,
            })),
        })
    }

    /// Current generation accepted by the kernel's `Fence`.
    pub fn current_generation(&self) -> u64 {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.snapshot.fence().accepted().map(|(gen_val, _)| gen_val.0))
            .unwrap_or(0)
    }

    /// Count of persisted journal records in `FileLog`.
    pub fn journal_len(&self) -> usize {
        self.inner
            .lock()
            .map(|g| g.journal.events().len())
            .unwrap_or(0)
    }

    /// Verifies Canon bytes, compiles the Warp DAG, enforces `Fence` via `kernel::step`,
    /// and persists to `FsContentStore` + `FileLog` when `enforce` is true.
    pub fn submit_plan(
        &self,
        plan_id_str: &str,
        generation: u64,
        expected_digest_hex: Option<&str>,
        canon_bytes: &[u8],
        enforce: bool,
    ) -> Result<PlanSummary, String> {
        let plan_id =
            PlanId::new(plan_id_str).ok_or_else(|| format!("invalid plan_id `{plan_id_str}`"))?;
        let (decoded, profile) = decode_canon_auto(canon_bytes)?;
        let cid = canon_id(&decoded.canon, profile);
        let actual_hex = hex::encode(cid.as_bytes());
        if let Some(expected) = expected_digest_hex
            && !expected.is_empty()
            && expected != actual_hex
        {
            return Err(format!(
                "canon digest mismatch: expected {expected}, computed {actual_hex}"
            ));
        }

        let kernel_canon = KernelCanon::try_from(&decoded.canon)
            .map_err(|e| format!("convert Canon to kernel model: {e:?}"))?;
        let vertices: Vec<Vertex> = kernel_canon
            .resources()
            .values()
            .map(|m| Vertex::action(m.key().clone(), m.keys.clone()))
            .collect();
        let compiled_graph = Graph::compile(vertices, kernel_canon.edges().to_vec())
            .map_err(|e| format!("Warp graph compile failed: {e:?}"))?;
        let warp_vertices = compiled_graph.vertices().count();
        let warp_edges = kernel_canon.edges().len();

        let plan = Plan {
            id: plan_id,
            generation: Generation(generation),
            canon: kernel_canon,
            bound: 4,
            policy: Policy::new(4, 30, 30, 5),
            expires: None,
        };
        let input = Input::Enforce(plan);

        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "kernel mutex poisoned".to_owned())?;
        let decision = step(&guard.snapshot, input.clone());
        for ev in &decision.events {
            if let Event::PlanRejected { reason, .. } = ev {
                return Err(format!(
                    "kernel rejected plan under Fence/policy: {reason:?}"
                ));
            }
        }
        let event_count = decision.events.len();

        if enforce {
            let stored_digest = guard
                .store
                .put(canon_bytes)
                .map_err(|e| format!("FsContentStore put failed: {e:?}"))?;
            let _ = Digest::from_bytes(*stored_digest.as_bytes());
            guard
                .journal
                .append(&[input])
                .map_err(|_| "FileLog append failed (log full or poisoned)".to_owned())?;
            guard.snapshot = decision.snapshot;
        }

        Ok(PlanSummary {
            plan_id: plan_id_str.to_owned(),
            generation,
            canon_digest_hex: actual_hex,
            warp_vertices,
            warp_edges,
            persisted: enforce,
            kernel_events: event_count,
        })
    }

    /// Validates an incoming execution receipt against `Fence` and `nomos_app::kernel::step`.
    pub fn verify_receipt(
        &self,
        plan_id_str: &str,
        generation: u64,
        payload_json: &str,
    ) -> Result<String, String> {
        let plan_id =
            PlanId::new(plan_id_str).ok_or_else(|| format!("invalid plan_id `{plan_id_str}`"))?;
        let payload: ReceiptPayload = serde_json::from_str(payload_json)
            .map_err(|e| format!("invalid receipt payload JSON: {e}"))?;
        let path = ResourcePath::new(&payload.resource_path)
            .map_err(|e| format!("invalid receipt resource_path: {e:?}"))?;
        let key = EffectKey::new(
            plan_id,
            Generation(generation),
            payload.iteration,
            ResourceKey::File(path),
        );
        let receipt = match payload.outcome.as_str() {
            "accepted" => Receipt::Accepted,
            "started" => Receipt::Started,
            "completed" => Receipt::Completed {
                changed: payload.changed,
            },
            "failed" => Receipt::Failed,
            "refused" => Receipt::Refused,
            other => return Err(format!("unknown receipt outcome `{other}`")),
        };

        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "kernel mutex poisoned".to_owned())?;
        if let Some((current, _)) = guard.snapshot.fence().accepted()
            && Generation(generation) < current
        {
            return Err(format!(
                "stale receipt generation {generation} behind fence {}",
                current.0
            ));
        }

        let input = Input::Receipt(key, receipt);
        let decision = step(&guard.snapshot, input.clone());
        for ev in &decision.events {
            if let Event::ReceiptIgnored { why, .. } = ev {
                return Err(format!("kernel rejected receipt: {why:?}"));
            }
        }
        guard
            .journal
            .append(&[input])
            .map_err(|_| "FileLog append failed".to_owned())?;
        guard.snapshot = decision.snapshot;
        Ok(format!("accepted ({} events)", decision.events.len()))
    }
}

fn decode_canon_auto(bytes: &[u8]) -> Result<(nomos_canon::artifact::Decoded, Profile), String> {
    if bytes
        .first()
        .is_some_and(|b| *b == b'{' || b.is_ascii_whitespace())
    {
        decode(bytes, Profile::Jcs, &Reader::current())
            .map(|d| (d, Profile::Jcs))
            .map_err(|e| format!("decode Canon JCS: {e:?}"))
    } else {
        decode(bytes, Profile::Cbor, &Reader::current())
            .map(|d| (d, Profile::Cbor))
            .map_err(|e| format!("decode Canon CBOR: {e:?}"))
    }
}
