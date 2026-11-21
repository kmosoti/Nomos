//! Google Cloud authentication provider with `expiry - 60s` OAuth2 token caching.
//!
//! Raw OAuth bearer tokens exist solely inside [`nomos_core::cipher::Secret`] and
//! never appear in [`AuthPosture`] or error diagnostics (invariant N8).

use std::sync::Arc;
use std::time::{Duration, Instant};

use google_cloud_auth::credentials::{Builder as AdcBuilder, impersonated::Builder as ImpBuilder};
use nomos_cloud::{AuthMode, AuthPosture, CloudAuthConfig, CloudError, CloudResult};
use nomos_core::cipher::Secret;
use nomos_core::plan::Generation;
use serde::Deserialize;
use tokio::sync::Mutex;

const CACHE_SAFETY_MARGIN: Duration = Duration::from_secs(60);
const DEFAULT_TOKEN_TTL_SECS: u64 = 3600;

/// Authentication mode backed by `google-cloud-auth` or an explicit test token source.
#[derive(Clone)]
pub enum GcpAuthMode {
    /// Uses standard Application Default Credentials or IAM impersonation.
    Cloud(CloudAuthConfig),
    /// Queries a local metadata-compatible HTTP endpoint returning `{"access_token":"..."}`.
    MetadataEndpoint {
        /// Full HTTP URL queried via GET with `Metadata-Flavor: Google`.
        url: String,
        /// Non-secret principal hint for [`AuthPosture`].
        principal_hint: Option<String>,
    },
    /// Injects an in-memory bearer token wrapped inside [`Secret`] for offline tests.
    InMemorySecret {
        /// Bearer token redacted against accidental display/serialization.
        token: Arc<Secret<String>>,
        /// Non-secret principal hint for [`AuthPosture`].
        principal_hint: Option<String>,
    },
}

impl core::fmt::Debug for GcpAuthMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Cloud(cfg) => f.debug_tuple("Cloud").field(cfg).finish(),
            Self::MetadataEndpoint {
                url,
                principal_hint,
            } => f
                .debug_struct("MetadataEndpoint")
                .field("url", url)
                .field("principal_hint", principal_hint)
                .finish(),
            Self::InMemorySecret { principal_hint, .. } => f
                .debug_struct("InMemorySecret")
                .field("token", &"[redacted]")
                .field("principal_hint", principal_hint)
                .finish(),
        }
    }
}

struct CachedToken {
    token: Secret<String>,
    refresh_after: Instant,
}

/// Token provider acquiring redacted OAuth2 bearer strings with `expiry - 60s` caching.
#[derive(Clone)]
pub struct GcpTokenProvider {
    mode: GcpAuthMode,
    cache: Arc<Mutex<Option<CachedToken>>>,
}

impl core::fmt::Debug for GcpTokenProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GcpTokenProvider")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct MetadataTokenPayload {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl GcpTokenProvider {
    /// Configures provider from generic [`CloudAuthConfig`].
    pub fn from_config(config: CloudAuthConfig) -> Self {
        Self {
            mode: GcpAuthMode::Cloud(config),
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Configures provider using an explicit metadata HTTP endpoint (for mock testing).
    pub fn from_metadata_endpoint(url: impl Into<String>, principal_hint: Option<String>) -> Self {
        Self {
            mode: GcpAuthMode::MetadataEndpoint {
                url: url.into(),
                principal_hint,
            },
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Configures provider from an already-acquired [`Secret`] bearer string.
    pub fn from_secret_bearer(token: Secret<String>, principal_hint: Option<String>) -> Self {
        Self {
            mode: GcpAuthMode::InMemorySecret {
                token: Arc::new(token),
                principal_hint,
            },
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Clears any cached OAuth2 bearer token, forcing a fresh fetch on next use.
    pub async fn invalidate_cache(&self) {
        let mut guard = self.cache.lock().await;
        *guard = None;
    }

    /// Resolves a bearer token inside [`Secret<String>`], caching until `expiry - 60s`.
    pub async fn bearer_token(&self, client: &reqwest::Client) -> CloudResult<Secret<String>> {
        if let GcpAuthMode::InMemorySecret { token, .. } = &self.mode {
            return Ok(Secret::new(token.expose().clone()));
        }

        let now = Instant::now();
        {
            let guard = self.cache.lock().await;
            if let Some(cached) = guard.as_ref()
                && now < cached.refresh_after
            {
                return Ok(Secret::new(cached.token.expose().clone()));
            }
        }

        let (fresh_token, ttl) = self.fetch_uncached(client).await?;
        let effective_ttl = ttl.saturating_sub(CACHE_SAFETY_MARGIN);
        if !effective_ttl.is_zero() {
            let mut guard = self.cache.lock().await;
            *guard = Some(CachedToken {
                token: Secret::new(fresh_token.expose().clone()),
                refresh_after: Instant::now() + effective_ttl,
            });
        }
        Ok(fresh_token)
    }

    async fn fetch_uncached(
        &self,
        client: &reqwest::Client,
    ) -> CloudResult<(Secret<String>, Duration)> {
        match &self.mode {
            GcpAuthMode::InMemorySecret { token, .. } => Ok((
                Secret::new(token.expose().clone()),
                Duration::from_secs(DEFAULT_TOKEN_TTL_SECS),
            )),
            GcpAuthMode::MetadataEndpoint { url, .. } => {
                let resp = client
                    .get(url)
                    .header("Metadata-Flavor", "Google")
                    .send()
                    .await
                    .map_err(|e| CloudError::Auth(format!("metadata token request: {e}")))?;
                if !resp.status().is_success() {
                    return Err(CloudError::Auth(format!(
                        "metadata endpoint returned HTTP {}",
                        resp.status()
                    )));
                }
                let parsed: MetadataTokenPayload = resp
                    .json()
                    .await
                    .map_err(|e| CloudError::Auth(format!("parse metadata token JSON: {e}")))?;
                if parsed.access_token.trim().is_empty() {
                    return Err(CloudError::Auth(
                        "empty access_token in metadata payload".into(),
                    ));
                }
                let ttl = Duration::from_secs(parsed.expires_in.unwrap_or(DEFAULT_TOKEN_TTL_SECS));
                Ok((Secret::new(parsed.access_token), ttl))
            }
            GcpAuthMode::Cloud(cfg) => match &cfg.mode {
                AuthMode::Adc => {
                    let creds = AdcBuilder::default()
                        .with_scopes(cfg.scopes.iter().map(String::as_str))
                        .build_access_token_credentials()
                        .map_err(|e| CloudError::Auth(format!("build ADC credentials: {e}")))?;
                    let tok = creds
                        .access_token()
                        .await
                        .map_err(|e| CloudError::Auth(format!("acquire ADC access token: {e}")))?;
                    let ttl = Duration::from_secs(DEFAULT_TOKEN_TTL_SECS);
                    Ok((Secret::new(tok.token), ttl))
                }
                AuthMode::ServiceAccountImpersonation {
                    target_service_account,
                } => {
                    let source = AdcBuilder::default()
                        .build()
                        .map_err(|e| CloudError::Auth(format!("build ADC source creds: {e}")))?;
                    let creds = ImpBuilder::from_source_credentials(source)
                        .with_target_principal(target_service_account.as_str())
                        .with_scopes(cfg.scopes.iter().map(String::as_str))
                        .build_access_token_credentials()
                        .map_err(|e| {
                            CloudError::Auth(format!("build impersonated credentials: {e}"))
                        })?;
                    let tok = creds.access_token().await.map_err(|e| {
                        CloudError::Auth(format!("acquire impersonated token: {e}"))
                    })?;
                    let ttl = Duration::from_secs(DEFAULT_TOKEN_TTL_SECS);
                    Ok((Secret::new(tok.token), ttl))
                }
            },
        }
    }

    /// Probes credential resolution and returns metadata-only [`AuthPosture`].
    pub async fn posture(
        &self,
        client: &reqwest::Client,
        generation: Generation,
    ) -> CloudResult<AuthPosture> {
        let (mode_label, hint) = match &self.mode {
            GcpAuthMode::Cloud(cfg) => match &cfg.mode {
                AuthMode::Adc => ("adc".to_owned(), None),
                AuthMode::ServiceAccountImpersonation {
                    target_service_account,
                } => (
                    "impersonation".to_owned(),
                    Some(target_service_account.clone()),
                ),
            },
            GcpAuthMode::MetadataEndpoint { principal_hint, .. } => {
                ("metadata".to_owned(), principal_hint.clone())
            }
            GcpAuthMode::InMemorySecret { principal_hint, .. } => {
                ("secret_bearer".to_owned(), principal_hint.clone())
            }
        };

        match self.bearer_token(client).await {
            Ok(_) => Ok(AuthPosture {
                authenticated: true,
                mode: mode_label,
                principal_hint: hint,
                generation: generation.0,
                detail: "token acquisition succeeded (value redacted)".to_owned(),
            }),
            Err(err) => Ok(AuthPosture {
                authenticated: false,
                mode: mode_label,
                principal_hint: hint,
                generation: generation.0,
                detail: err.to_string(),
            }),
        }
    }
}
