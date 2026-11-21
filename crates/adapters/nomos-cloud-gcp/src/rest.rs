//! Pure-IPv6 GCP Compute REST v1 client and topology diff planner.

use std::time::Duration;

use nomos_cloud::{CloudDriftItem, CloudError, CloudPlan, CloudResult, DriftStatus, TopologySpec};
use nomos_core::cipher::Secret;
use nomos_core::resource::Digest;

use crate::lro::post_and_wait_lro;
use crate::rest_types::{
    CreateFirewallBody, CreateInstanceBody, CreateSubnetBody, DiskInitParams, FirewallAllowed,
    GceInstanceResource, GceSubnetResource, InstanceDisk, InstanceNetIf,
};

/// Configurable API base URLs and LRO poll limits (defaults to live GCP endpoints).
#[derive(Clone, Debug)]
pub struct GcpEndpointConfig {
    /// Compute Engine REST v1 base URL (no trailing slash).
    pub compute_base_url: String,
    /// Cloud Batch REST v1 base URL (no trailing slash).
    pub batch_base_url: String,
    /// Delay between LRO status polls.
    pub lro_poll_interval: Duration,
    /// Maximum LRO polls before returning [`CloudError::Timeout`].
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
        items.push(check_exists(client, token, &fw_url, "firewall", &fw.name).await?);
    }

    for node in &spec.nodes {
        let vm_url = format!(
            "{}/projects/{}/zones/{}/instances/{}",
            cfg.compute_base_url, spec.project_id, node.zone, node.name
        );
        items.push(check_instance(client, token, &vm_url, &node.name, &node.machine_type).await?);
    }

    Ok(CloudPlan {
        project_id: spec.project_id.clone(),
        spec_digest_hex: digest_hex(&digest),
        dry_run: true,
        items,
    })
}

/// Applies missing resources from `spec` and polls every returned LRO to `DONE`.
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
        if item.status != DriftStatus::Create {
            continue;
        }
        match item.kind.as_str() {
            "subnetwork" => {
                let url = format!(
                    "{}/projects/{}/regions/{}/subnetworks",
                    cfg.compute_base_url, spec.project_id, spec.region
                );
                let body = CreateSubnetBody {
                    name: &spec.subnet_name,
                    network: format!(
                        "projects/{}/global/networks/{}",
                        spec.project_id, spec.network_name
                    ),
                    stack_type: "IPV6_ONLY",
                    ipv6_access_type: "EXTERNAL",
                };
                post_and_wait_lro(client, cfg, token, &url, &body).await?;
            }
            "firewall" => {
                if let Some(fw) = spec.firewall_rules.iter().find(|r| r.name == item.name) {
                    let url = format!(
                        "{}/projects/{}/global/firewalls",
                        cfg.compute_base_url, spec.project_id
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
                    post_and_wait_lro(client, cfg, token, &url, &body).await?;
                }
            }
            "instance" => {
                if let Some(node) = spec.nodes.iter().find(|n| n.name == item.name) {
                    let url = format!(
                        "{}/projects/{}/zones/{}/instances",
                        cfg.compute_base_url, spec.project_id, node.zone
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
                    };
                    post_and_wait_lro(client, cfg, token, &url, &body).await?;
                }
            }
            _ => {}
        }
    }

    Ok(plan)
}

async fn check_subnet(
    client: &reqwest::Client,
    token: &Secret<String>,
    url: &str,
    spec: &TopologySpec,
) -> CloudResult<CloudDriftItem> {
    let resp = client
        .get(url)
        .bearer_auth(token.expose())
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("GET subnet `{}`: {e}", spec.subnet_name)))?;
    if resp.status().as_u16() == 404 {
        return Ok(CloudDriftItem {
            kind: "subnetwork".into(),
            name: spec.subnet_name.clone(),
            status: DriftStatus::Create,
            detail: "subnetwork absent".into(),
        });
    }
    if !resp.status().is_success() {
        return Err(CloudError::Http(format!(
            "GET subnet `{}` HTTP {}",
            spec.subnet_name,
            resp.status()
        )));
    }
    let body: GceSubnetResource = resp
        .json()
        .await
        .map_err(|e| CloudError::Serialization(format!("decode subnet: {e}")))?;
    let status = if body.ipv6_cidr_range.as_deref() == Some(spec.subnet_ipv6_cidr.as_str()) {
        DriftStatus::Match
    } else {
        DriftStatus::Update
    };
    Ok(CloudDriftItem {
        kind: "subnetwork".into(),
        name: spec.subnet_name.clone(),
        status,
        detail: "evaluated subnetwork IPv6 CIDR".into(),
    })
}

async fn check_exists(
    client: &reqwest::Client,
    token: &Secret<String>,
    url: &str,
    kind: &str,
    name: &str,
) -> CloudResult<CloudDriftItem> {
    let resp = client
        .get(url)
        .bearer_auth(token.expose())
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("GET {kind} `{name}`: {e}")))?;
    if resp.status().as_u16() == 404 {
        Ok(CloudDriftItem {
            kind: kind.into(),
            name: name.into(),
            status: DriftStatus::Create,
            detail: format!("{kind} absent"),
        })
    } else if resp.status().is_success() {
        Ok(CloudDriftItem {
            kind: kind.into(),
            name: name.into(),
            status: DriftStatus::Match,
            detail: format!("{kind} present"),
        })
    } else {
        Err(CloudError::Http(format!(
            "GET {kind} `{name}` HTTP {}",
            resp.status()
        )))
    }
}

async fn check_instance(
    client: &reqwest::Client,
    token: &Secret<String>,
    url: &str,
    name: &str,
    desired_machine_type: &str,
) -> CloudResult<CloudDriftItem> {
    let resp = client
        .get(url)
        .bearer_auth(token.expose())
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("GET instance `{name}`: {e}")))?;
    if resp.status().as_u16() == 404 {
        return Ok(CloudDriftItem {
            kind: "instance".into(),
            name: name.into(),
            status: DriftStatus::Create,
            detail: "instance absent".into(),
        });
    }
    if !resp.status().is_success() {
        return Err(CloudError::Http(format!(
            "GET instance `{name}` HTTP {}",
            resp.status()
        )));
    }
    let vm: GceInstanceResource = resp
        .json()
        .await
        .map_err(|e| CloudError::Serialization(format!("decode instance: {e}")))?;
    let matches_type = vm.machine_type.as_deref().is_some_and(|mt| {
        mt == desired_machine_type || mt.ends_with(&format!("/{desired_machine_type}"))
    });
    Ok(CloudDriftItem {
        kind: "instance".into(),
        name: name.into(),
        status: if matches_type {
            DriftStatus::Match
        } else {
            DriftStatus::Update
        },
        detail: "evaluated instance machine type".into(),
    })
}

fn digest_hex(digest: &Digest) -> String {
    let mut out = String::with_capacity(64);
    for b in digest.as_bytes() {
        use core::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}
