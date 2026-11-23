//! Headless IPv6 Axum HTTP API with scoped bearer auth, off-worker TTL-cached posture
//! probes, SSE event streaming, and optional `--web-assets-dir` hosting.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Response;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use nomos_cipher::SecretsPosture;
use nomos_cloud::{AuthPosture, CloudSubstrate};
use nomos_core::cipher::Secret;
use nomos_core::plan::Generation;
use nomos_mesh::MeshPort;
use nomos_protocol::QuicStats;
use tokio::sync::Semaphore;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::governor_svc::GovernorService;
use crate::lease::{IssueLeaseRequest, LeaseBroker, LeaseRecord, staged_credential_names};
use crate::posture_cache::{
    GuardedSseStream, HealthResponse, LeaseIssuedResponse, PostureCache, VerifyLeaseRequest,
    VerifyLeaseResponse, resolve_cloud_auth_posture, resolve_secrets_posture,
};
use crate::server_auth::{authorize_bearer, authorize_control_admin, serve_web_asset_impl};
use crate::telemetry::{GovernorStatusDto, SampleInputDto, TelemetryResponse};

const DEFAULT_POSTURE_CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_SSE_SUBSCRIBERS: usize = 16;

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
    /// Explicit `--allow-plaintext-key` opt-in for local dev/CI vaults.
    pub allow_plaintext_key: bool,
    /// Optional mesh adapter for posture reporting.
    pub mesh: Option<Arc<dyn MeshPort + Send + Sync>>,
    /// Optional cloud adapter for auth posture reporting.
    pub cloud: Option<Arc<dyn CloudSubstrate>>,
    /// Snapshot callback returning current QUIC transport counters.
    pub quic_stats: Arc<dyn Fn() -> QuicStats + Send + Sync>,
    /// Optional prebuilt web assets directory (`--web-assets-dir`).
    pub web_assets_dir: Option<PathBuf>,
    /// BLAKE3 digest of operator admin control token (never stored in plaintext).
    control_token_hash: Option<blake3::Hash>,
    /// TTL for off-worker vTPM and GCP posture caches.
    pub posture_cache_ttl: Duration,
    cached_secrets: PostureCache<SecretsPosture>,
    cached_cloud_auth: PostureCache<AuthPosture>,
    sse_permits: Arc<Semaphore>,
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
            allow_plaintext_key: false,
            mesh: None,
            cloud: None,
            quic_stats: Arc::new(QuicStats::default),
            web_assets_dir,
            control_token_hash: None,
            posture_cache_ttl: DEFAULT_POSTURE_CACHE_TTL,
            cached_secrets: PostureCache::default(),
            cached_cloud_auth: PostureCache::default(),
            sse_permits: Arc::new(Semaphore::new(MAX_SSE_SUBSCRIBERS)),
        }
    }

    /// Configures the operator admin bearer token (hashed immediately with BLAKE3; N8).
    pub fn with_control_token(mut self, token: &Secret<String>) -> Self {
        self.control_token_hash = Some(blake3::hash(token.expose().as_bytes()));
        self
    }

    /// Overrides TTL for off-worker vTPM and GCP posture caches.
    pub fn with_posture_cache_ttl(mut self, ttl: Duration) -> Self {
        self.posture_cache_ttl = ttl;
        self
    }
}

/// Builds the Axum router with scoped bearer auth on control/observability routes,
/// minimal public `/health`, bounded SSE `/v1/events`, and safe `--web-assets-dir`.
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
        headless: state.web_assets_dir.is_none(),
    })
}

async fn list_leases(
    State(state): State<LoomState>,
    headers: HeaderMap,
) -> Result<Json<Vec<LeaseRecord>>, (StatusCode, String)> {
    authorize_bearer(
        state.control_token_hash,
        &state.leases,
        &headers,
        &["admin", "control", "observer"],
    )?;
    state
        .leases
        .list_active()
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

async fn issue_lease(
    State(state): State<LoomState>,
    headers: HeaderMap,
    Json(req): Json<IssueLeaseRequest>,
) -> Result<(StatusCode, Json<LeaseIssuedResponse>), (StatusCode, String)> {
    authorize_control_admin(state.control_token_hash, &state.leases, &headers)?;
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
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    authorize_control_admin(state.control_token_hash, &state.leases, &headers)?;
    match state.leases.revoke(id) {
        Ok(true) => {
            state
                .governor
                .emit_event("lease_revoked", format!("id={id}"));
            Ok(StatusCode::NO_CONTENT)
        }
        Ok(false) => Err((StatusCode::NOT_FOUND, "lease not found".to_owned())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn verify_lease(
    State(state): State<LoomState>,
    headers: HeaderMap,
    Json(req): Json<VerifyLeaseRequest>,
) -> Result<Json<VerifyLeaseResponse>, (StatusCode, String)> {
    authorize_bearer(
        state.control_token_hash,
        &state.leases,
        &headers,
        &["admin", "control", "observer"],
    )?;
    let wrapped = Secret::new(req.token);
    let record = state
        .leases
        .verify_token(&wrapped)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(VerifyLeaseResponse {
        valid: record.is_some(),
        lease: record,
    }))
}

async fn get_governor(
    State(state): State<LoomState>,
    headers: HeaderMap,
) -> Result<Json<GovernorStatusDto>, (StatusCode, String)> {
    authorize_bearer(
        state.control_token_hash,
        &state.leases,
        &headers,
        &["admin", "control", "governor", "observer"],
    )?;
    Ok(Json(state.governor.status()))
}

async fn post_governor_sample(
    State(state): State<LoomState>,
    headers: HeaderMap,
    Json(sample): Json<SampleInputDto>,
) -> Result<Json<GovernorStatusDto>, (StatusCode, String)> {
    authorize_bearer(
        state.control_token_hash,
        &state.leases,
        &headers,
        &["admin", "control", "governor"],
    )?;
    state
        .governor
        .observe_dto(&sample)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn get_telemetry(
    State(state): State<LoomState>,
    headers: HeaderMap,
) -> Result<Json<TelemetryResponse>, (StatusCode, String)> {
    authorize_bearer(
        state.control_token_hash,
        &state.leases,
        &headers,
        &["admin", "control", "governor", "observer"],
    )?;
    let generation = state.governor.generation();
    let active_leases = state.leases.list_active().map(|l| l.len()).unwrap_or(0);
    let secrets = resolve_secrets_posture(
        state.vault_dir.as_ref(),
        state.allow_plaintext_key,
        state.posture_cache_ttl,
        &state.cached_secrets,
    )
    .await;
    let mesh = state
        .mesh
        .as_ref()
        .and_then(|m| m.posture(Generation(generation)).ok());
    let cloud_auth = resolve_cloud_auth_posture(
        state.cloud.as_ref(),
        generation,
        state.posture_cache_ttl,
        &state.cached_cloud_auth,
    )
    .await;

    Ok(Json(TelemetryResponse {
        node: state.node_name.clone(),
        generation,
        active_leases,
        secrets,
        mesh,
        quic: (state.quic_stats)(),
        cloud_auth,
        governor: state.governor.status(),
        staged_credentials: staged_credential_names(),
    }))
}

async fn events_sse(
    State(state): State<LoomState>,
    headers: HeaderMap,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    authorize_bearer(
        state.control_token_hash,
        &state.leases,
        &headers,
        &["admin", "control", "governor", "observer"],
    )?;
    let permit = Arc::clone(&state.sse_permits)
        .try_acquire_owned()
        .map_err(|_| {
            (
                StatusCode::TOO_MANY_REQUESTS,
                "too many concurrent SSE subscribers".to_owned(),
            )
        })?;
    let rx = state.governor.subscribe();
    let inner = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(ev) => Some(Ok(Event::default()
            .event(ev.kind.clone())
            .json_data(&ev)
            .unwrap_or_else(|_| Event::default().data("event")))),
        Err(_) => None,
    });
    let stream = GuardedSseStream { inner, permit };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn serve_web_asset(State(state): State<LoomState>, uri: Uri) -> Response {
    serve_web_asset_impl(state.web_assets_dir.as_ref(), uri).await
}
