//! GCP Long-Running Operation (LRO) polling with deterministic UUIDv5 `?requestId=`,
//! jittered exponential backoff, and HTTP `429`/`503` retries.

use std::time::Duration;

use nomos_cloud::{CloudError, CloudResult};
use nomos_core::cipher::Secret;
use reqwest::Method;
use serde::Serialize;

use crate::rest::GcpEndpointConfig;
use crate::rest_types::GceOperationResponse;
use crate::url_guard::resolve_lro_poll_url;

const MAX_TRANSIENT_RETRIES: u32 = 5;

/// Derives a deterministic RFC 4122 UUIDv5 string from `(namespace, key)` via BLAKE3
/// for idempotent GCE `?requestId=` query parameters.
pub fn deterministic_request_id(namespace: &str, key: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"nomos/gcp-request-id/v1");
    h.update(namespace.as_bytes());
    h.update(&[0u8]);
    h.update(key.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h.finalize().as_bytes()[..16]);
    // Set RFC 4122 version 5 and variant 1 bits.
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// Appends `?requestId=<uuid-v5>` (or `&requestId=`) to a GCE mutation URL.
pub fn with_request_id(url: &str, request_id: &str) -> String {
    if let Ok(mut parsed) = reqwest::Url::parse(url) {
        parsed
            .query_pairs_mut()
            .append_pair("requestId", request_id);
        return parsed.into();
    }
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}requestId={request_id}")
}

/// Computes deterministic jittered exponential backoff for attempt `attempt` (`0..`).
pub fn jittered_backoff(base: Duration, attempt: u32, salt: &str) -> Duration {
    let shift = attempt.min(4);
    let exp = base.saturating_mul(1u32 << shift);
    let mut h = blake3::Hasher::new();
    h.update(salt.as_bytes());
    h.update(&attempt.to_le_bytes());
    let jitter_pct = u64::from(h.finalize().as_bytes()[0] % 25); // 0..24% deterministic jitter
    let extra_ms = (exp.as_millis() as u64).saturating_mul(jitter_pct) / 100;
    exp.saturating_add(Duration::from_millis(extra_ms))
        .min(base.saturating_mul(16).max(Duration::from_millis(50)))
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 503)
}

/// Sends a GCE POST mutation with `?requestId=<uuid-v5>` and polls its LRO to `DONE`.
pub async fn post_and_wait_lro<B: Serialize>(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    url: &str,
    request_id: &str,
    body: &B,
) -> CloudResult<()> {
    mutate_and_wait_lro(client, cfg, token, Method::POST, url, request_id, body).await
}

/// Sends a GCE PATCH mutation with `?requestId=<uuid-v5>` and polls its LRO to `DONE`.
pub async fn patch_and_wait_lro<B: Serialize>(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    url: &str,
    request_id: &str,
    body: &B,
) -> CloudResult<()> {
    mutate_and_wait_lro(client, cfg, token, Method::PATCH, url, request_id, body).await
}

async fn mutate_and_wait_lro<B: Serialize>(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    method: Method,
    url: &str,
    request_id: &str,
    body: &B,
) -> CloudResult<()> {
    let full_url = with_request_id(url, request_id);
    let mut attempt = 0u32;
    let resp = loop {
        let r = client
            .request(method.clone(), &full_url)
            .bearer_auth(token.expose())
            .json(body)
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("{method} `{full_url}`: {e}")))?;
        if is_retryable_status(r.status()) && attempt < MAX_TRANSIENT_RETRIES {
            tokio::time::sleep(jittered_backoff(cfg.lro_poll_interval, attempt, &full_url)).await;
            attempt += 1;
            continue;
        }
        break r;
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(CloudError::Http(format!(
            "{method} `{full_url}` HTTP {status}: {text}"
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
    let poll_url = resolve_lro_poll_url(
        &cfg.compute_base_url,
        op.self_link.as_deref(),
        op.name.as_deref(),
    )?;
    wait_lro(client, cfg, token, &poll_url).await
}

/// Polls a GCP Compute LRO until `DONE` with jittered exponential backoff and `429`/`503` retries.
pub async fn wait_lro(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    poll_url: &str,
) -> CloudResult<()> {
    let mut transient_retries = 0u32;
    for poll_idx in 0..cfg.lro_max_polls {
        let resp = client
            .get(poll_url)
            .bearer_auth(token.expose())
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("GET LRO `{poll_url}`: {e}")))?;
        if is_retryable_status(resp.status()) {
            if transient_retries >= MAX_TRANSIENT_RETRIES {
                return Err(CloudError::Http(format!(
                    "GET LRO `{poll_url}` exhausted retries on HTTP {}",
                    resp.status()
                )));
            }
            let delay = jittered_backoff(cfg.lro_poll_interval, transient_retries, poll_url);
            transient_retries += 1;
            tokio::time::sleep(delay).await;
            continue;
        }
        if !resp.status().is_success() {
            return Err(CloudError::Http(format!(
                "GET LRO `{poll_url}` HTTP {}",
                resp.status()
            )));
        }
        transient_retries = 0;
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
        tokio::time::sleep(jittered_backoff(
            cfg.lro_poll_interval,
            poll_idx as u32,
            poll_url,
        ))
        .await;
    }
    Err(CloudError::Timeout(format!(
        "LRO `{poll_url}` did not complete"
    )))
}
