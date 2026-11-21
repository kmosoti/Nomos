//! Headless IPv6 Axum HTTP API, SSE event stream, and optional `--web-assets-dir` server.

use std::convert::Infallible;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use nomos_cipher::{CipherPort, SecretPolicy};
use nomos_cipher_age_vtpm::{AgeVtpmVault, MemoryTargetTransport};
use nomos_cloud::{AuthPosture, CloudSubstrate};
use nomos_core::cipher::Secret;
use nomos_core::plan::Generation;
use nomos_mesh::MeshPort;
use nomos_protocol::QuicStats;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::governor_svc::GovernorService;
use crate::lease::{IssueLeaseRequest, LeaseBroker, LeaseRecord, staged_credential_names};
use crate::telemetry::{GovernorStatusDto, SampleInputDto, TelemetryResponse};

/// Shared composition-root state backing HTTP routes.
#[derive(Clone)]
pub struct LoomState {
    /// Logical coordinator node name.
    pub node_name: String,
    /// SQLite WAL lease broker.
    pub leases: LeaseBroker,
    /// Stateful PSI/cgroup governor service.
    pub governor: GovernorService,
    /// Optional local age/vTPM cipher vault directory for posture checks.
    pub vault_dir: Option<PathBuf>,
    /// Optional mesh adapter for posture reporting.
    pub mesh: Option<Arc<dyn MeshPort + Send + Sync>>,
    /// Optional cloud adapter for auth posture reporting.
    pub cloud: Option<Arc<dyn CloudSubstrate>>,
    /// Snapshot callback returning current QUIC transport counters.
    pub quic_stats: Arc<dyn Fn() -> QuicStats + Send + Sync>,
    /// Optional prebuilt web assets directory (`--web-assets-dir`).
    pub web_assets_dir: Option<PathBuf>,
    cached_cloud_auth: Arc<Mutex<Option<AuthPosture>>>,
}

impl LoomState {
    /// Constructs state from core services and optional web assets root.
    pub fn new(
        node_name: impl Into<String>,
        leases: LeaseBroker,
        governor: GovernorService,
        vault_dir: Option<PathBuf>,
        web_assets_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            node_name: node_name.into(),
            leases,
            governor,
            vault_dir,
            mesh: None,
            cloud: None,
            quic_stats: Arc::new(QuicStats::default),
            web_assets_dir,
            cached_cloud_auth: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    node: String,
    generation: u64,
    headless: bool,
}

#[derive(Serialize)]
struct LeaseIssuedResponse {
    lease: LeaseRecord,
    token: String,
}

#[derive(Deserialize)]
struct VerifyLeaseRequest {
    token: String,
}

#[derive(Serialize)]
struct VerifyLeaseResponse {
    valid: bool,
    lease: Option<LeaseRecord>,
}

/// Builds the Axum router with JSON routes, SSE `/v1/events`, and optional `--web-assets-dir`.
pub fn build_router(state: LoomState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/leases", get(list_leases).post(issue_lease))
        .route("/v1/leases/{id}", delete(revoke_lease))
        .route("/v1/leases/verify", post(verify_lease))
        .route("/v1/governor", get(get_governor))
        .route("/v1/governor/sample", post(post_governor_sample))
        .route("/v1/telemetry", get(get_telemetry))
        .route("/v1/events", get(events_sse))
        .fallback(get(serve_web_asset))
        .with_state(state)
}

async fn health(State(state): State<LoomState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        node: state.node_name.clone(),
        generation: state.governor.generation(),
        headless: state.web_assets_dir.is_none(),
    })
}

async fn list_leases(State(state): State<LoomState>) -> Result<Json<Vec<LeaseRecord>>, StatusCode> {
    state
        .leases
        .list_active()
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn issue_lease(
    State(state): State<LoomState>,
    Json(req): Json<IssueLeaseRequest>,
) -> Result<(StatusCode, Json<LeaseIssuedResponse>), (StatusCode, String)> {
    if req.scope == "batch" && !state.governor.status().admit_batch {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "batch lease rejected by PSI governor pressure shedding".to_owned(),
        ));
    }
    let issued = state
        .leases
        .issue(&req)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state.governor.emit_event(
        "lease_issued",
        format!(
            "id={} subject={} scope={}",
            issued.record.id, issued.record.subject, issued.record.scope
        ),
    );
    let token = issued.token.expose().clone();
    Ok((
        StatusCode::CREATED,
        Json(LeaseIssuedResponse {
            lease: issued.record,
            token,
        }),
    ))
}

async fn revoke_lease(
    State(state): State<LoomState>,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, StatusCode> {
    match state.leases.revoke(id) {
        Ok(true) => {
            state
                .governor
                .emit_event("lease_revoked", format!("id={id}"));
            Ok(StatusCode::NO_CONTENT)
        }
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn verify_lease(
    State(state): State<LoomState>,
    Json(req): Json<VerifyLeaseRequest>,
) -> Result<Json<VerifyLeaseResponse>, StatusCode> {
    let wrapped = Secret::new(req.token);
    let record = state
        .leases
        .verify_token(&wrapped)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(VerifyLeaseResponse {
        valid: record.is_some(),
        lease: record,
    }))
}

async fn get_governor(State(state): State<LoomState>) -> Json<GovernorStatusDto> {
    Json(state.governor.status())
}

async fn post_governor_sample(
    State(state): State<LoomState>,
    Json(sample): Json<SampleInputDto>,
) -> Result<Json<GovernorStatusDto>, (StatusCode, String)> {
    state
        .governor
        .observe_dto(&sample)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn get_telemetry(State(state): State<LoomState>) -> Json<TelemetryResponse> {
    let generation = state.governor.generation();
    let active_leases = state.leases.list_active().map(|l| l.len()).unwrap_or(0);

    let secrets = state.vault_dir.as_ref().and_then(|dir| {
        let vault = AgeVtpmVault::new(dir);
        let policy = SecretPolicy::default();
        let transport = MemoryTargetTransport::new();
        vault.posture(&policy, &transport, None).ok()
    });
    let mesh = state
        .mesh
        .as_ref()
        .and_then(|m| m.posture(Generation(generation)).ok());
    let cloud_auth = if let Some(cloud) = &state.cloud {
        let posture = cloud.auth_posture(Generation(generation)).await.ok();
        if let Ok(mut guard) = state.cached_cloud_auth.lock() {
            *guard = posture.clone();
        }
        posture
    } else {
        state.cached_cloud_auth.lock().ok().and_then(|g| g.clone())
    };

    Json(TelemetryResponse {
        node: state.node_name.clone(),
        generation,
        active_leases,
        secrets,
        mesh,
        quic: (state.quic_stats)(),
        cloud_auth,
        governor: state.governor.status(),
        staged_credentials: staged_credential_names(),
    })
}

async fn events_sse(
    State(state): State<LoomState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.governor.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(ev) => Some(Ok(Event::default()
            .event(ev.kind.clone())
            .json_data(&ev)
            .unwrap_or_else(|_| Event::default().data("event")))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn serve_web_asset(State(state): State<LoomState>, uri: Uri) -> Response {
    let Some(root) = &state.web_assets_dir else {
        return (
            StatusCode::NOT_FOUND,
            "headless nomos-loom: no --web-assets-dir configured",
        )
            .into_response();
    };
    let rel = uri.path().trim_start_matches('/');
    let Some(candidate) = resolve_safe_asset(root, if rel.is_empty() { "index.html" } else { rel })
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let target = if candidate.is_file() {
        candidate
    } else {
        let Some(index) = resolve_safe_asset(root, "index.html") else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if !index.is_file() {
            return StatusCode::NOT_FOUND.into_response();
        }
        index
    };
    match std::fs::read(&target) {
        Ok(bytes) => {
            let mime = mime_for_path(&target);
            let mut resp = Response::new(axum::body::Body::from(bytes));
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            resp
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn resolve_safe_asset(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut joined = root.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(seg) => joined.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(joined)
}

fn mime_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
