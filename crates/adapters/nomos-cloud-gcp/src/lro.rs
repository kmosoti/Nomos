//! Long-running operation (LRO) submission and status polling over pure-IPv6 GCP REST.

use nomos_cloud::{CloudError, CloudResult};
use nomos_core::cipher::Secret;
use serde::Serialize;

use crate::rest::GcpEndpointConfig;
use crate::rest_types::GceOperationResponse;

/// Submits a POST mutation returning a GCP LRO and polls until completion or timeout.
pub async fn post_and_wait_lro<B: Serialize>(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    url: &str,
    body: &B,
) -> CloudResult<()> {
    let resp = client
        .post(url)
        .bearer_auth(token.expose())
        .json(body)
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("POST `{url}`: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(CloudError::Http(format!(
            "POST `{url}` HTTP {status}: {text}"
        )));
    }
    let op: GceOperationResponse = resp
        .json()
        .await
        .map_err(|e| CloudError::Serialization(format!("decode LRO response: {e}")))?;
    if op.status.as_deref() == Some("DONE") {
        if let Some(err) = op.error {
            return Err(CloudError::OperationFailed(err.to_string()));
        }
        return Ok(());
    }
    let poll_url = match (op.self_link, op.name) {
        (Some(link), _) if link.starts_with("http") => link,
        (Some(link), _) => format!(
            "{}/{}",
            cfg.compute_base_url.trim_end_matches('/'),
            link.trim_start_matches('/')
        ),
        (None, Some(name)) => format!("{}/{name}", cfg.compute_base_url),
        (None, None) => {
            return Err(CloudError::OperationFailed(
                "LRO missing selfLink and name".into(),
            ));
        }
    };
    wait_lro(client, cfg, token, &poll_url).await
}

/// Polls a GCP Compute LRO resource until `DONE`, rejecting operation errors and timeouts.
pub async fn wait_lro(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    poll_url: &str,
) -> CloudResult<()> {
    for _ in 0..cfg.lro_max_polls {
        let resp = client
            .get(poll_url)
            .bearer_auth(token.expose())
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("GET LRO `{poll_url}`: {e}")))?;
        if !resp.status().is_success() {
            return Err(CloudError::Http(format!(
                "GET LRO `{poll_url}` HTTP {}",
                resp.status()
            )));
        }
        let op: GceOperationResponse = resp
            .json()
            .await
            .map_err(|e| CloudError::Serialization(format!("decode LRO status: {e}")))?;
        if let Some(err) = op.error {
            return Err(CloudError::OperationFailed(err.to_string()));
        }
        if op.status.as_deref() == Some("DONE") {
            return Ok(());
        }
        tokio::time::sleep(cfg.lro_poll_interval).await;
    }
    Err(CloudError::Timeout(format!(
        "LRO `{poll_url}` did not complete"
    )))
}
