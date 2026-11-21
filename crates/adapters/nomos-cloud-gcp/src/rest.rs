//! Pure-IPv6 GCP Compute REST v1 client, drift planner, and `PATCH` reconciliation.

use std::time::Duration;

use nomos_cloud::{CloudPlan, CloudResult, DriftStatus, TopologySpec};
use nomos_core::cipher::Secret;
use nomos_core::resource::Digest;

use crate::lro::{deterministic_request_id, patch_and_wait_lro, post_and_wait_lro};
use crate::rest_diff::{check_firewall, check_instance, check_subnet};
use crate::rest_types::{
    CreateFirewallBody, CreateInstanceBody, CreateSubnetBody, DiskInitParams, FirewallAllowed,
    InstanceDisk, InstanceNetIf, InstanceTags, PatchFirewallBody, SetInstanceTagsBody,
};

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

/// Evaluates [`TopologySpec`] against live GCE resources without issuing mutations.
pub async fn plan_topology(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    spec: &TopologySpec,
) -> CloudResult<CloudPlan> {
    let digest = spec.validate()?;
    let mut items = Vec::new();

    let subnet_url = format!(
        "{}/projects/{}/regions/{}/subnetworks/{}",
        cfg.compute_base_url, spec.project_id, spec.region, spec.subnet_name
    );
    items.push(check_subnet(client, token, &subnet_url, spec).await?);

    for fw in &spec.firewall_rules {
        let fw_url = format!(
            "{}/projects/{}/global/firewalls/{}",
            cfg.compute_base_url, spec.project_id, fw.name
        );
        items.push(check_firewall(client, token, &fw_url, fw).await?);
    }

    for node in &spec.nodes {
        let vm_url = format!(
            "{}/projects/{}/zones/{}/instances/{}",
            cfg.compute_base_url, spec.project_id, node.zone, node.name
        );
        items.push(check_instance(client, token, &vm_url, node).await?.0);
    }

    Ok(CloudPlan {
        project_id: spec.project_id.clone(),
        spec_digest_hex: digest_hex(&digest),
        dry_run: true,
        items,
    })
}

/// Applies `Create` and `Update` (`PATCH` / `setTags`) changes from `spec` with deterministic
/// `?requestId=<uuid-v5>` and polls every returned LRO to `DONE`.
pub async fn apply_topology(
    client: &reqwest::Client,
    cfg: &GcpEndpointConfig,
    token: &Secret<String>,
    spec: &TopologySpec,
    confirm_apply: bool,
) -> CloudResult<CloudPlan> {
    let mut plan = plan_topology(client, cfg, token, spec).await?;
    if !confirm_apply {
        return Ok(plan);
    }
    plan.dry_run = false;

    for item in &plan.items {
        match (item.kind.as_str(), item.status) {
            ("subnetwork", DriftStatus::Create) => {
                let url = format!(
                    "{}/projects/{}/regions/{}/subnetworks",
                    cfg.compute_base_url, spec.project_id, spec.region
                );
                let req_id = deterministic_request_id(
                    "create-subnet",
                    &format!(
                        "{}/{}/{}",
                        spec.project_id, spec.subnet_name, plan.spec_digest_hex
                    ),
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
                    let url = format!(
                        "{}/projects/{}/global/firewalls",
                        cfg.compute_base_url, spec.project_id
                    );
                    let req_id = deterministic_request_id(
                        "create-firewall",
                        &format!("{}/{}/{}", spec.project_id, fw.name, plan.spec_digest_hex),
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
                    let url = format!(
                        "{}/projects/{}/global/firewalls/{}",
                        cfg.compute_base_url, spec.project_id, fw.name
                    );
                    let req_id = deterministic_request_id(
                        "patch-firewall",
                        &format!("{}/{}/{}", spec.project_id, fw.name, plan.spec_digest_hex),
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
                    let url = format!(
                        "{}/projects/{}/zones/{}/instances",
                        cfg.compute_base_url, spec.project_id, node.zone
                    );
                    let req_id = deterministic_request_id(
                        "create-instance",
                        &format!("{}/{}/{}", spec.project_id, node.name, plan.spec_digest_hex),
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
                    let vm_url = format!(
                        "{}/projects/{}/zones/{}/instances/{}",
                        cfg.compute_base_url, spec.project_id, node.zone, node.name
                    );
                    let (_, fingerprint) = check_instance(client, token, &vm_url, node).await?;
                    let url = format!("{vm_url}/setTags");
                    let req_id = deterministic_request_id(
                        "patch-instance-tags",
                        &format!("{}/{}/{}", spec.project_id, node.name, plan.spec_digest_hex),
                    );
                    let body = SetInstanceTagsBody {
                        items: &node.tags,
                        fingerprint,
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
