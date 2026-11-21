//! Spot Batch runner (`e2-standard-4`, terminal state polling, scale-to-zero cleanup)
//! over GCP Batch REST v1.

use nomos_cloud::{BatchJobHandle, CloudError, CloudResult, SpotBatchConfig};
use nomos_core::cipher::Secret;
use serde::{Deserialize, Serialize};

use crate::lro::jittered_backoff;
use crate::rest::GcpEndpointConfig;

#[derive(Serialize)]
struct BatchJobRequest<'a> {
    #[serde(rename = "taskGroups")]
    task_groups: [TaskGroup<'a>; 1],
    #[serde(rename = "allocationPolicy")]
    allocation_policy: AllocationPolicy<'a>,
}

#[derive(Serialize)]
struct TaskGroup<'a> {
    #[serde(rename = "taskSpec")]
    task_spec: TaskSpec<'a>,
    #[serde(rename = "taskCount")]
    task_count: u32,
}

#[derive(Serialize)]
struct TaskSpec<'a> {
    runnables: [Runnable<'a>; 1],
    #[serde(rename = "maxRunDuration")]
    max_run_duration: String,
}

#[derive(Serialize)]
struct Runnable<'a> {
    container: ContainerSpec<'a>,
}

#[derive(Serialize)]
struct ContainerSpec<'a> {
    #[serde(rename = "imageUri")]
    image_uri: &'a str,
    commands: &'a [String],
}

#[derive(Serialize)]
struct AllocationPolicy<'a> {
    instances: [InstancePolicyOrTemplate<'a>; 1],
    network: NetworkPolicy<'a>,
}

#[derive(Serialize)]
struct InstancePolicyOrTemplate<'a> {
    policy: InstancePolicy<'a>,
}

#[derive(Serialize)]
struct InstancePolicy<'a> {
    #[serde(rename = "machineType")]
    machine_type: &'a str,
    #[serde(rename = "provisioningModel")]
    provisioning_model: &'static str,
}

#[derive(Serialize)]
struct NetworkPolicy<'a> {
    #[serde(rename = "networkInterfaces")]
    network_interfaces: [BatchNetworkInterface<'a>; 1],
}

#[derive(Serialize)]
struct BatchNetworkInterface<'a> {
    network: &'a str,
    subnetwork: &'a str,
}

#[derive(Deserialize)]
struct BatchJobResponse {
    name: String,
    uid: Option<String>,
    status: Option<BatchStatus>,
}

#[derive(Deserialize)]
struct BatchStatus {
    state: Option<String>,
}

fn is_terminal_batch_state(state: &str) -> bool {
    matches!(state, "SUCCEEDED" | "FAILED" | "DELETION_IN_PROGRESS")
}

/// Polls a Batch job resource until it reaches a terminal state (`SUCCEEDED`/`FAILED`).
pub async fn wait_batch_job_terminal(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    project_id: &str,
    region: &str,
    job_id: &str,
) -> CloudResult<BatchJobHandle> {
    let url = format!(
        "{}/projects/{project_id}/locations/{region}/jobs/{job_id}",
        cfg.batch_base_url
    );
    for poll_idx in 0..cfg.lro_max_polls {
        let resp = client
            .get(&url)
            .bearer_auth(token.expose())
            .send()
            .await
            .map_err(|e| CloudError::Http(format!("GET Batch job `{job_id}`: {e}")))?;
        if matches!(resp.status().as_u16(), 429 | 503) {
            tokio::time::sleep(jittered_backoff(
                cfg.lro_poll_interval,
                poll_idx as u32,
                &url,
            ))
            .await;
            continue;
        }
        if !resp.status().is_success() {
            return Err(CloudError::Http(format!(
                "GET Batch job `{job_id}` HTTP {}",
                resp.status()
            )));
        }
        let job: BatchJobResponse = resp
            .json()
            .await
            .map_err(|e| CloudError::Serialization(format!("decode Batch status JSON: {e}")))?;
        let state = job
            .status
            .and_then(|s| s.state)
            .unwrap_or_else(|| "RUNNING".to_owned());
        if is_terminal_batch_state(&state) {
            return Ok(BatchJobHandle {
                name: job.name,
                uid: job.uid.unwrap_or_else(|| job_id.to_owned()),
                state,
            });
        }
        tokio::time::sleep(jittered_backoff(
            cfg.lro_poll_interval,
            poll_idx as u32,
            &url,
        ))
        .await;
    }
    Err(CloudError::Timeout(format!(
        "Batch job `{job_id}` did not reach terminal state"
    )))
}

/// Submits a Spot Batch job, polls until terminal state when `scale_to_zero` is enabled,
/// and deletes the completed job cleanly.
pub async fn submit_spot_batch(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    job: &SpotBatchConfig,
) -> CloudResult<BatchJobHandle> {
    if job.project_id.trim().is_empty()
        || job.region.trim().is_empty()
        || job.job_id.trim().is_empty()
    {
        return Err(CloudError::InvalidSpec(
            "Batch job project_id, region, and job_id must be non-empty".into(),
        ));
    }
    let url = format!(
        "{}/projects/{}/locations/{}/jobs?jobId={}",
        cfg.batch_base_url, job.project_id, job.region, job.job_id
    );
    let body = BatchJobRequest {
        task_groups: [TaskGroup {
            task_spec: TaskSpec {
                runnables: [Runnable {
                    container: ContainerSpec {
                        image_uri: &job.container_image,
                        commands: &job.commands,
                    },
                }],
                max_run_duration: format!("{}s", job.max_run_secs),
            },
            task_count: 1,
        }],
        allocation_policy: AllocationPolicy {
            instances: [InstancePolicyOrTemplate {
                policy: InstancePolicy {
                    machine_type: &job.machine_type,
                    provisioning_model: "SPOT",
                },
            }],
            network: NetworkPolicy {
                network_interfaces: [BatchNetworkInterface {
                    network: &job.network_uri,
                    subnetwork: &job.subnetwork_uri,
                }],
            },
        },
    };

    let resp = client
        .post(&url)
        .bearer_auth(token.expose())
        .json(&body)
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("POST Spot Batch `{}`: {e}", job.job_id)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(CloudError::Http(format!(
            "POST Spot Batch `{}` HTTP {status}: {text}",
            job.job_id
        )));
    }
    let created: BatchJobResponse = resp
        .json()
        .await
        .map_err(|e| CloudError::Serialization(format!("decode Batch job JSON: {e}")))?;

    let initial_state = created
        .status
        .and_then(|s| s.state)
        .unwrap_or_else(|| "QUEUED".to_owned());
    let mut handle = BatchJobHandle {
        name: created.name,
        uid: created.uid.unwrap_or_else(|| job.job_id.clone()),
        state: initial_state,
    };

    if job.scale_to_zero {
        if !is_terminal_batch_state(&handle.state) {
            handle = wait_batch_job_terminal(
                client,
                cfg,
                token,
                &job.project_id,
                &job.region,
                &job.job_id,
            )
            .await?;
        }
        delete_batch_job(
            client,
            cfg,
            token,
            &job.project_id,
            &job.region,
            &job.job_id,
        )
        .await?;
    }
    Ok(handle)
}

/// Deletes a Spot Batch job resource for zero-idle-compute cleanup.
pub async fn delete_batch_job(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    project_id: &str,
    region: &str,
    job_id: &str,
) -> CloudResult<()> {
    let url = format!(
        "{}/projects/{project_id}/locations/{region}/jobs/{job_id}",
        cfg.batch_base_url
    );
    let resp = client
        .delete(&url)
        .bearer_auth(token.expose())
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("DELETE Batch job `{job_id}`: {e}")))?;
    if resp.status().is_success() || resp.status().as_u16() == 404 {
        Ok(())
    } else {
        Err(CloudError::Http(format!(
            "DELETE Batch job `{job_id}` HTTP {}",
            resp.status()
        )))
    }
}
