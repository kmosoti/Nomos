//! Cloud substrate port: config-driven topology convergence, auth posture, and Spot Batch.

pub mod spec;
pub mod validation;

use core::fmt;
use std::future::Future;
use std::pin::Pin;

use nomos_core::plan::Generation;
use serde::{Deserialize, Serialize};

pub use spec::{
    BatchJobHandle, CloudDriftItem, CloudFirewallRule, CloudNodeSpec, CloudPlan, DriftStatus,
    SpotBatchConfig, TopologySpec,
};
pub use validation::{
    validate_firewall_rule, validate_gcp_path_or_uri, validate_gcp_project_id,
    validate_gcp_resource_name, validate_ipv6_cidr,
};

/// Result alias for cloud port operations.
pub type CloudResult<T> = Result<T, CloudError>;

/// Errors returned by cloud substrate adapters without exposing bearer tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudError {
    /// Topology or Batch specification failed validation.
    InvalidSpec(String),
    /// Cloud authentication failed (`ADC` or impersonation).
    Auth(String),
    /// HTTP/REST transport failure or non-2xx API response.
    Http(String),
    /// Long-running operation reported provider error or failed status.
    OperationFailed(String),
    /// Long-running operation exceeded configured poll limit.
    Timeout(String),
    /// Request or response JSON serialization error.
    Serialization(String),
    /// Unsupported cloud operation.
    Unsupported(String),
}

impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloudError::InvalidSpec(msg) => write!(f, "invalid cloud topology spec: {msg}"),
            CloudError::Auth(msg) => write!(f, "cloud auth error: {msg}"),
            CloudError::Http(msg) => write!(f, "cloud HTTP error: {msg}"),
            CloudError::OperationFailed(msg) => write!(f, "cloud LRO failed: {msg}"),
            CloudError::Timeout(msg) => write!(f, "cloud LRO timeout: {msg}"),
            CloudError::Serialization(msg) => write!(f, "cloud JSON error: {msg}"),
            CloudError::Unsupported(msg) => write!(f, "unsupported cloud operation: {msg}"),
        }
    }
}

impl std::error::Error for CloudError {}

/// Cloud credential acquisition mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMode {
    /// Application Default Credentials (`ADC`).
    Adc,
    /// Service account impersonation chained from ADC source credentials.
    ServiceAccountImpersonation {
        /// Target IAM service account email.
        target_service_account: String,
    },
}

/// Caller configuration for cloud credential acquisition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudAuthConfig {
    /// Credential strategy (`Adc` or `ServiceAccountImpersonation`).
    pub mode: AuthMode,
    /// Requested OAuth scopes.
    pub scopes: Vec<String>,
}

impl Default for CloudAuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::Adc,
            scopes: vec!["https://www.googleapis.com/auth/cloud-platform".to_owned()],
        }
    }
}

/// Metadata-only cloud credential posture (never embeds OAuth bearer tokens).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPosture {
    /// True if credential resolution succeeded.
    pub authenticated: bool,
    /// Active mode description (`adc`, `impersonation`, or `mock`).
    pub mode: String,
    /// Optional non-secret principal label or service account email.
    pub principal_hint: Option<String>,
    /// Authority generation when posture was checked.
    pub generation: u64,
    /// Diagnostic summary (never contains raw tokens).
    pub detail: String,
}

impl AuthPosture {
    /// Typed generation watermark from [`nomos_core::plan::Generation`].
    pub const fn typed_generation(&self) -> Generation {
        Generation(self.generation)
    }
}

/// Driven port abstracting cloud topology convergence and ephemeral Spot Batch compute.
pub trait CloudSubstrate: Send + Sync {
    /// Resolves credentials and returns metadata-only [`AuthPosture`].
    fn auth_posture<'a>(
        &'a self,
        generation: Generation,
    ) -> Pin<Box<dyn Future<Output = CloudResult<AuthPosture>> + Send + 'a>>;

    /// Plans topology drift against live cloud resources without mutating state.
    fn plan_topology<'a>(
        &'a self,
        spec: &'a TopologySpec,
    ) -> Pin<Box<dyn Future<Output = CloudResult<CloudPlan>> + Send + 'a>>;

    /// Applies missing or drifted topology resources and polls LRO completion when `confirm_apply` is true.
    fn apply_topology<'a>(
        &'a self,
        spec: &'a TopologySpec,
        confirm_apply: bool,
    ) -> Pin<Box<dyn Future<Output = CloudResult<CloudPlan>> + Send + 'a>>;

    /// Submits a Spot Batch workload (`e2-standard-4`, optional `scale_to_zero` cleanup).
    fn submit_spot_batch<'a>(
        &'a self,
        config: &'a SpotBatchConfig,
    ) -> Pin<Box<dyn Future<Output = CloudResult<BatchJobHandle>> + Send + 'a>>;

    /// Deletes an idle or completed Batch job to maintain scale-to-zero hygiene.
    fn delete_batch_job<'a>(
        &'a self,
        project_id: &'a str,
        region: &'a str,
        job_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = CloudResult<()>> + Send + 'a>>;
}
