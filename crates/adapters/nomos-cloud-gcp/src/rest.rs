//! Pure-IPv6 GCP Compute REST v1 client, drift planner, and `PATCH` reconciliation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nomos_cloud::{CloudPlan, CloudResult, DriftStatus, TopologySpec};
use nomos_core::cipher::Secret;
use nomos_core::resource::Digest;

use crate::lro::{deterministic_request_id, patch_and_wait_lro, post_and_wait_lro};
use crate::rest_diff::{ObservedDrift, check_firewall, check_instance, check_subnet};
use crate::rest_types::{
    CreateFirewallBody, CreateInstanceBody, CreateSubnetBody, DiskInitParams, FirewallAllowed,
    InstanceDisk, InstanceNetIf, InstanceTags, PatchFirewallBody, SetInstanceTagsBody,
};
use crate::url_guard::build_gcp_url;

static APPLY_EXECUTION_SEQ: AtomicU64 = AtomicU64::new(1);

/// Configurable API base URLs and LRO poll limits (defaults to live GCP endpoints).
#[derive(Clone, Debug)]
pub struct GcpEndpointConfig {
    /// Compute Engine REST v1 base URL (no trailing slash).
    pub compute_base_url: String,
    /// Cloud Batch REST v1 base URL (no trailing slash).
    pub batch_base_url: String,
    /// Base delay between LRO / Batch status polls.
    pub lro_poll_interval: Duration,
    /// Maximum LRO / Batch polls before returning [`nomos_cloud::CloudError::Timeout`].
    pub lro_max_polls: usize,
}

impl Default for GcpEndpointConfig {
    fn default() -> Self {
        Self {
            compute_base_url: "https://compute.googleapis.com/compute/v1".to_owned(),
            batch_base_url: "https://batch.googleapis.com/v1".to_owned(),
            lro_poll_interval: Duration::from_millis(250),
            lro_max_polls: 120,
        }
    }
}

fn compute_url(cfg: &GcpEndpointConfig, segs: &[&str]) -> CloudResult<String> {
    build_gcp_url(&cfg.compute_base_url, "compute_base_url", segs, &[])
}

fn repair_request_id(
    op_kind: &str,
    project_id: &str,
    resource_name: &str,
    spec_digest_hex: &str,
    state_token: &str,
    epoch: u64,
) -> String {
    deterministic_request_id(
        op_kind,
        &format!("{project_id}/{resource_name}/{spec_digest_hex}/{state_token}/{epoch}"),
    )
}

async fn evaluate_topology(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    spec: &TopologySpec,
) -> CloudResult<(CloudPlan, Vec<ObservedDrift>)> {
    let digest = spec.validate()?;
    let mut observed = Vec::new();

    let subnet_url = compute_url(
        cfg,
        &[
            "projects",
            &spec.project_id,
            "regions",
            &spec.region,
            "subnetworks",
            &spec.subnet_name,
        ],
    )?;
    observed.push(check_subnet(client, token, &subnet_url, spec).await?);

    for fw in &spec.firewall_rules {
        let fw_url = compute_url(
            cfg,
            &[
                "projects",
                &spec.project_id,
                "global",
                "firewalls",
                &fw.name,
            ],
        )?;
        observed.push(check_firewall(client, token, &fw_url, fw).await?);
    }

    for node in &spec.nodes {
        let vm_url = compute_url(
            cfg,
            &[
                "projects",
                &spec.project_id,
                "zones",
                &node.zone,
                "instances",
                &node.name,
            ],
        )?;
        observed.push(check_instance(client, token, &vm_url, node).await?);
    }

    let items = observed.iter().map(|o| o.item.clone()).collect();
    let plan = CloudPlan {
        project_id: spec.project_id.clone(),
        spec_digest_hex: digest_hex(&digest),
        dry_run: true,
        items,
    };
    Ok((plan, observed))
}

/// Evaluates [`TopologySpec`] against live GCE resources without issuing mutations.
pub async fn plan_topology(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    spec: &TopologySpec,
) -> CloudResult<CloudPlan> {
    let (plan, _) = evaluate_topology(client, cfg, token, spec).await?;
    Ok(plan)
}

/// Applies `Create` and `Update` (`PATCH` / `setTags`) changes from `spec` with
/// state- and execution-scoped UUIDv5 `?requestId=` and polls every returned LRO to `DONE`.
pub async fn apply_topology(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    spec: &TopologySpec,
    confirm_apply: bool,
) -> CloudResult<CloudPlan> {
    let epoch = APPLY_EXECUTION_SEQ.fetch_add(1, Ordering::Relaxed);
    apply_topology_with_epoch(client, cfg, token, spec, confirm_apply, epoch).await
}

/// Executes [`apply_topology`] with an explicit monotonic `execution_epoch` mixed into `?requestId=`.
pub async fn apply_topology_with_epoch(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    spec: &TopologySpec,
    confirm_apply: bool,
    execution_epoch: u64,
) -> CloudResult<CloudPlan> {
    let (mut plan, observed) = evaluate_topology(client, cfg, token, spec).await?;
    if !confirm_apply {
        return Ok(plan);
    }
    plan.dry_run = false;

    for obs in &observed {
        let item = &obs.item;
        match (item.kind.as_str(), item.status) {
            ("subnetwork", DriftStatus::Create) => {
                let url = compute_url(
                    cfg,
                    &[
                        "projects",
                        &spec.project_id,
                        "regions",
                        &spec.region,
                        "subnetworks",
                    ],
                )?;
                let req_id = repair_request_id(
                    "create-subnet",
                    &spec.project_id,
                    &spec.subnet_name,
                    &plan.spec_digest_hex,
                    &obs.state_token,
                    execution_epoch,
                );
                let body = CreateSubnetBody {
                    name: &spec.subnet_name,
                    network: format!(
                        "projects/{}/global/networks/{}",
                        spec.project_id, spec.network_name
                    ),
                    stack_type: "IPV6_ONLY",
                    ipv6_access_type: "EXTERNAL",
                    ipv6_cidr_range: &spec.subnet_ipv6_cidr,
                };
                post_and_wait_lro(client, cfg, token, &url, &req_id, &body).await?;
            }
            ("firewall", DriftStatus::Create) => {
                if let Some(fw) = spec.firewall_rules.iter().find(|r| r.name == item.name) {
                    let url =
                        compute_url(cfg, &["projects", &spec.project_id, "global", "firewalls"])?;
                    let req_id = repair_request_id(
                        "create-firewall",
                        &spec.project_id,
                        &fw.name,
                        &plan.spec_digest_hex,
                        &obs.state_token,
                        execution_epoch,
                    );
                    let body = CreateFirewallBody {
                        name: &fw.name,
                        network: format!(
                            "projects/{}/global/networks/{}",
                            spec.project_id, spec.network_name
                        ),
                        direction: &fw.direction,
                        source_ranges: &fw.source_ranges_v6,
                        allowed: [FirewallAllowed {
                            ip_protocol: &fw.protocol,
                            ports: &fw.ports,
                        }],
                    };
                    post_and_wait_lro(client, cfg, token, &url, &req_id, &body).await?;
                }
            }
            ("firewall", DriftStatus::Update) => {
                if let Some(fw) = spec.firewall_rules.iter().find(|r| r.name == item.name) {
                    let url = compute_url(
                        cfg,
                        &[
                            "projects",
                            &spec.project_id,
                            "global",
                            "firewalls",
                            &fw.name,
                        ],
                    )?;
                    let req_id = repair_request_id(
                        "patch-firewall",
                        &spec.project_id,
                        &fw.name,
                        &plan.spec_digest_hex,
                        &obs.state_token,
                        execution_epoch,
                    );
                    let body = PatchFirewallBody {
                        direction: &fw.direction,
                        source_ranges: &fw.source_ranges_v6,
                        allowed: [FirewallAllowed {
                            ip_protocol: &fw.protocol,
                            ports: &fw.ports,
                        }],
                    };
                    patch_and_wait_lro(client, cfg, token, &url, &req_id, &body).await?;
                }
            }
            ("instance", DriftStatus::Create) => {
                if let Some(node) = spec.nodes.iter().find(|n| n.name == item.name) {
                    let url = compute_url(
                        cfg,
                        &[
                            "projects",
                            &spec.project_id,
                            "zones",
                            &node.zone,
                            "instances",
                        ],
                    )?;
                    let req_id = repair_request_id(
                        "create-instance",
                        &spec.project_id,
                        &node.name,
                        &plan.spec_digest_hex,
                        &obs.state_token,
                        execution_epoch,
                    );
                    let ipv6_only = node.ipv6_cidr.split('/').next().unwrap_or(&node.ipv6_cidr);
                    let body = CreateInstanceBody {
                        name: &node.name,
                        machine_type: format!(
                            "zones/{}/machineTypes/{}",
                            node.zone, node.machine_type
                        ),
                        network_interfaces: [InstanceNetIf {
                            subnetwork: format!(
                                "projects/{}/regions/{}/subnetworks/{}",
                                spec.project_id, spec.region, spec.subnet_name
                            ),
                            stack_type: "IPV6_ONLY",
                            ipv6_address: ipv6_only.to_owned(),
                        }],
                        disks: [InstanceDisk {
                            boot: true,
                            auto_delete: true,
                            initialize_params: DiskInitParams {
                                source_image: &node.source_image,
                            },
                        }],
                        tags: InstanceTags { items: &node.tags },
                    };
                    post_and_wait_lro(client, cfg, token, &url, &req_id, &body).await?;
                }
            }
            ("instance", DriftStatus::Update) => {
                if let Some(node) = spec.nodes.iter().find(|n| n.name == item.name) {
                    let url = compute_url(
                        cfg,
                        &[
                            "projects",
                            &spec.project_id,
                            "zones",
                            &node.zone,
                            "instances",
                            &node.name,
                            "setTags",
                        ],
                    )?;
                    let req_id = repair_request_id(
                        "patch-instance-tags",
                        &spec.project_id,
                        &node.name,
                        &plan.spec_digest_hex,
                        &obs.state_token,
                        execution_epoch,
                    );
                    let body = SetInstanceTagsBody {
                        items: &node.tags,
                        fingerprint: obs.tags_fingerprint.clone(),
                    };
                    post_and_wait_lro(client, cfg, token, &url, &req_id, &body).await?;
                }
            }
            _ => {}
        }
    }

    Ok(plan)
}

fn digest_hex(digest: &Digest) -> String {
    let mut out = String::with_capacity(64);
    for b in digest.as_bytes() {
        use core::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}
