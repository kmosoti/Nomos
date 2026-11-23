//! Single-flight TTL-cached off-worker posture probes for `nomos-loom`.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use nomos_cipher::{CipherPort, SecretPolicy, SecretsPosture};
use nomos_cipher_age_vtpm::{AgeVtpmVault, MemoryTargetTransport};
use nomos_cloud::{AuthPosture, CloudSubstrate};
use nomos_core::plan::Generation;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock};

use crate::lease::LeaseRecord;

#[derive(Serialize)]
pub(crate) struct HealthResponse {
    pub(crate) status: &'static str,
    pub(crate) headless: bool,
}

#[derive(Serialize)]
pub(crate) struct LeaseIssuedResponse {
    pub(crate) lease: LeaseRecord,
    pub(crate) token: String,
}

#[derive(Deserialize)]
pub(crate) struct VerifyLeaseRequest {
    pub(crate) token: String,
}

#[derive(Serialize)]
pub(crate) struct VerifyLeaseResponse {
    pub(crate) valid: bool,
    pub(crate) lease: Option<LeaseRecord>,
}

pub(crate) struct GuardedSseStream<S> {
    pub(crate) inner: S,
    pub(crate) permit: OwnedSemaphorePermit,
}

impl<S: tokio_stream::Stream + Unpin> tokio_stream::Stream for GuardedSseStream<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let _keep = &self.permit;
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

const POSTURE_REFRESH_TIMEOUT: Duration = Duration::from_secs(2);

type CachedSlot<T> = Option<(Instant, Option<T>)>;

/// Thread-safe single-flight TTL cache returning stale values during contended refreshes.
pub(crate) struct PostureCache<T> {
    data: Arc<RwLock<CachedSlot<T>>>,
    refresh_lock: Arc<Mutex<()>>,
}

impl<T: Clone> Clone for PostureCache<T> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
            refresh_lock: Arc::clone(&self.refresh_lock),
        }
    }
}

impl<T> Default for PostureCache<T> {
    fn default() -> Self {
        Self {
            data: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(Mutex::new(())),
        }
    }
}

impl<T: Clone> PostureCache<T> {
    async fn read_fresh(&self, ttl: Duration) -> Option<Option<T>> {
        let guard = self.data.read().await;
        let (fetched_at, cached) = guard.as_ref()?;
        if Instant::now().duration_since(*fetched_at) < ttl {
            Some(cached.clone())
        } else {
            None
        }
    }

    async fn read_stale(&self) -> Option<T> {
        let guard = self.data.read().await;
        guard.as_ref().and_then(|(_, cached)| cached.clone())
    }
}

pub(crate) async fn resolve_secrets_posture(
    vault_dir: Option<&PathBuf>,
    allow_plaintext_key: bool,
    ttl: Duration,
    cache: &PostureCache<SecretsPosture>,
) -> Option<SecretsPosture> {
    let dir = vault_dir?.clone();
    if let Some(hit) = cache.read_fresh(ttl).await {
        return hit;
    }
    let Ok(_single_flight) = cache.refresh_lock.try_lock() else {
        return cache.read_stale().await;
    };
    if let Some(hit) = cache.read_fresh(ttl).await {
        return hit;
    }
    let join = tokio::task::spawn_blocking(move || {
        let vault = AgeVtpmVault::new(dir).allow_plaintext_key(allow_plaintext_key);
        let policy = SecretPolicy::default();
        let transport = MemoryTargetTransport::new();
        vault.posture(&policy, &transport, None).ok()
    });
    let fresh = match tokio::time::timeout(POSTURE_REFRESH_TIMEOUT, join).await {
        Ok(Ok(val)) => val,
        _ => return cache.read_stale().await,
    };
    let mut guard = cache.data.write().await;
    *guard = Some((Instant::now(), fresh.clone()));
    fresh
}

pub(crate) async fn resolve_cloud_auth_posture(
    cloud: Option<&Arc<dyn CloudSubstrate>>,
    generation: u64,
    ttl: Duration,
    cache: &PostureCache<AuthPosture>,
) -> Option<AuthPosture> {
    let cloud = cloud?;
    if let Some(hit) = cache.read_fresh(ttl).await {
        return hit;
    }
    let Ok(_single_flight) = cache.refresh_lock.try_lock() else {
        return cache.read_stale().await;
    };
    if let Some(hit) = cache.read_fresh(ttl).await {
        return hit;
    }
    let fresh = match tokio::time::timeout(
        POSTURE_REFRESH_TIMEOUT,
        cloud.auth_posture(Generation(generation)),
    )
    .await
    {
        Ok(Ok(val)) => Some(val),
        Ok(Err(_)) => None,
        Err(_) => return cache.read_stale().await,
    };
    let mut guard = cache.data.write().await;
    *guard = Some((Instant::now(), fresh.clone()));
    fresh
}
