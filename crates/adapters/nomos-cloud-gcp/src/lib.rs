//! Google Cloud Platform adapter implementing [`nomos_cloud::CloudSubstrate`] ([ADR 0021](../../../docs/adr/0021-cloud-substrate-port-and-gcp-adapter.md)).

pub mod auth;
pub mod batch;
pub mod lro;
pub mod rest;
mod rest_diff;
mod rest_types;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use nomos_cloud::{
    AuthPosture, BatchJobHandle, CloudPlan, CloudResult, CloudSubstrate, SpotBatchConfig,
    TopologySpec,
};
use nomos_core::plan::Generation;

pub use auth::{GcpAuthMode, GcpTokenProvider};
pub use rest::GcpEndpointConfig;

/// Google Cloud substrate adapter driving Compute Engine and Batch over IPv6 REST APIs.
#[derive(Clone, Debug)]
pub struct GcpCloudAdapter {
    auth: GcpTokenProvider,
    client: reqwest::Client,
    endpoints: GcpEndpointConfig,
}

impl GcpCloudAdapter {
    /// Creates an adapter targeting standard Google Cloud API endpoints.
    pub fn new(auth: GcpTokenProvider) -> CloudResult<Self> {
        Self::with_endpoints(auth, GcpEndpointConfig::default())
    }

    /// Creates an adapter targeting explicit base URLs (for example a mock `[::1]` REST server).
    pub fn with_endpoints(
        auth: GcpTokenProvider,
        endpoints: GcpEndpointConfig,
    ) -> CloudResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| nomos_cloud::CloudError::Http(format!("build HTTP client: {e}")))?;
        Ok(Self {
            auth,
            client,
            endpoints,
        })
    }
}

impl CloudSubstrate for GcpCloudAdapter {
    fn auth_posture<'a>(
        &'a self,
        generation: Generation,
    ) -> Pin<Box<dyn Future<Output = CloudResult<AuthPosture>> + Send + 'a>> {
        Box::pin(async move { self.auth.posture(&self.client, generation).await })
    }

    fn plan_topology<'a>(
        &'a self,
        spec: &'a TopologySpec,
    ) -> Pin<Box<dyn Future<Output = CloudResult<CloudPlan>> + Send + 'a>> {
        Box::pin(async move {
            let token = self.auth.bearer_token(&self.client).await?;
            rest::plan_topology(&self.client, &self.endpoints, &token, spec).await
        })
    }

    fn apply_topology<'a>(
        &'a self,
        spec: &'a TopologySpec,
        confirm_apply: bool,
    ) -> Pin<Box<dyn Future<Output = CloudResult<CloudPlan>> + Send + 'a>> {
        Box::pin(async move {
            let token = self.auth.bearer_token(&self.client).await?;
            rest::apply_topology(&self.client, &self.endpoints, &token, spec, confirm_apply).await
        })
    }

    fn submit_spot_batch<'a>(
        &'a self,
        config: &'a SpotBatchConfig,
    ) -> Pin<Box<dyn Future<Output = CloudResult<BatchJobHandle>> + Send + 'a>> {
        Box::pin(async move {
            let token = self.auth.bearer_token(&self.client).await?;
            batch::submit_spot_batch(&self.client, &self.endpoints, &token, config).await
        })
    }

    fn delete_batch_job<'a>(
        &'a self,
        project_id: &'a str,
        region: &'a str,
        job_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = CloudResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let token = self.auth.bearer_token(&self.client).await?;
            batch::delete_batch_job(
                &self.client,
                &self.endpoints,
                &token,
                project_id,
                region,
                job_id,
            )
            .await
        })
    }
}
