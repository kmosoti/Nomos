//! Config-driven cloud topology specification and Spot Batch request types.

use nomos_core::resource::Digest;
use serde::{Deserialize, Serialize};

use crate::{CloudError, CloudResult};

/// Desired pure-IPv6 cloud VM node specification supplied by operator config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudNodeSpec {
    /// Instance hostname inside the project zone.
    pub name: String,
    /// Compute zone (for example `us-central1-a`).
    pub zone: String,
    /// Machine type (for example `e2-standard-2`).
    pub machine_type: String,
    /// Assigned pure-IPv6 CIDR address (for example `2001:db8:1::10/96`).
    pub ipv6_cidr: String,
    /// Network tags applied for firewall matching.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Source boot image self-link or family path.
    pub source_image: String,
}

/// Desired ingress/egress pure-IPv6 cloud firewall rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudFirewallRule {
    /// Firewall resource name.
    pub name: String,
    /// Traffic direction (`INGRESS` or `EGRESS`).
    pub direction: String,
    /// Source pure-IPv6 CIDR blocks.
    pub source_ranges_v6: Vec<String>,
    /// IP protocol (for example `tcp` or `udp`).
    pub protocol: String,
    /// Allowed ports or port ranges (for example `51820`).
    pub ports: Vec<String>,
}

/// Complete caller-provided cloud infrastructure topology (zero hardcoded project constants).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologySpec {
    /// Cloud project identifier (for example `example-project`).
    pub project_id: String,
    /// Regional deployment identifier (for example `us-central1`).
    pub region: String,
    /// VPC network name.
    pub network_name: String,
    /// IPv6-capable subnetwork name.
    pub subnet_name: String,
    /// Subnetwork pure-IPv6 CIDR allocation (`2001:db8::/64`).
    pub subnet_ipv6_cidr: String,
    /// Managed virtual machine nodes.
    pub nodes: Vec<CloudNodeSpec>,
    /// Managed firewall rules.
    #[serde(default)]
    pub firewall_rules: Vec<CloudFirewallRule>,
}

impl TopologySpec {
    /// Validates project, IPv6 subnet/node fields, and returns deterministic specification digest.
    pub fn validate(&self) -> CloudResult<Digest> {
        if self.project_id.trim().is_empty()
            || self.region.trim().is_empty()
            || self.network_name.trim().is_empty()
            || self.subnet_name.trim().is_empty()
        {
            return Err(CloudError::InvalidSpec(
                "project_id, region, network_name, and subnet_name must be non-empty".into(),
            ));
        }
        if !self.subnet_ipv6_cidr.contains(':') {
            return Err(CloudError::InvalidSpec(format!(
                "subnet_ipv6_cidr `{}` must be IPv6",
                self.subnet_ipv6_cidr
            )));
        }
        for node in &self.nodes {
            if node.name.trim().is_empty() || node.zone.trim().is_empty() {
                return Err(CloudError::InvalidSpec(
                    "node name and zone must be non-empty".into(),
                ));
            }
            if !node.ipv6_cidr.contains(':') {
                return Err(CloudError::InvalidSpec(format!(
                    "node `{}` ipv6_cidr `{}` must be IPv6",
                    node.name, node.ipv6_cidr
                )));
            }
        }
        for fw in &self.firewall_rules {
            if fw.name.trim().is_empty()
                || fw.source_ranges_v6.is_empty()
                || fw.source_ranges_v6.iter().any(|r| !r.contains(':'))
            {
                return Err(CloudError::InvalidSpec(format!(
                    "firewall `{}` must define pure-IPv6 source ranges",
                    fw.name
                )));
            }
        }
        Ok(self.spec_digest())
    }

    /// Computes deterministic 32-byte digest across all topology fields.
    pub fn spec_digest(&self) -> Digest {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        feed(self.project_id.as_bytes());
        feed(self.region.as_bytes());
        feed(self.network_name.as_bytes());
        feed(self.subnet_name.as_bytes());
        feed(self.subnet_ipv6_cidr.as_bytes());
        for node in &self.nodes {
            feed(node.name.as_bytes());
            feed(node.zone.as_bytes());
            feed(node.machine_type.as_bytes());
            feed(node.ipv6_cidr.as_bytes());
        }
        for fw in &self.firewall_rules {
            feed(fw.name.as_bytes());
            feed(fw.protocol.as_bytes());
        }
        let mut out = [0u8; 32];
        for (i, chunk) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let mixed =
                h.rotate_left((i as u32) * 13) ^ ((i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            *chunk = mixed.to_be_bytes();
        }
        Digest::from_bytes(out)
    }
}

/// Convergence action state for a cloud resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftStatus {
    /// Cloud resource is absent and will be created.
    Create,
    /// Cloud resource exists with differing configuration.
    Update,
    /// Cloud resource matches [`TopologySpec`].
    Match,
}

/// Single resource evaluation inside [`CloudPlan`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudDriftItem {
    /// Resource kind (`subnetwork`, `firewall`, or `instance`).
    pub kind: String,
    /// Resource name inside cloud project.
    pub name: String,
    /// Drift classification.
    pub status: DriftStatus,
    /// Non-secret explanation.
    pub detail: String,
}

/// Result of planning or applying [`TopologySpec`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudPlan {
    /// Target cloud project ID.
    pub project_id: String,
    /// Hex representation of [`TopologySpec::spec_digest`].
    pub spec_digest_hex: String,
    /// True when no write mutations ran.
    pub dry_run: bool,
    /// Resource diffs evaluated in deterministic order.
    pub items: Vec<CloudDriftItem>,
}

impl CloudPlan {
    /// Number of non-matching resources (`Create` or `Update`).
    pub fn pending_changes(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.status, DriftStatus::Create | DriftStatus::Update))
            .count()
    }
}

/// Scale-to-zero Spot Batch execution parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotBatchConfig {
    /// Target project ID.
    pub project_id: String,
    /// Region hosting the Batch job.
    pub region: String,
    /// Unique job identifier.
    pub job_id: String,
    /// Compute machine type (defaults to `e2-standard-4`).
    pub machine_type: String,
    /// Maximum runtime in seconds before automatic termination.
    pub max_run_secs: u64,
    /// Container image executed on the Spot worker.
    pub container_image: String,
    /// Entrypoint command arguments.
    pub commands: Vec<String>,
    /// VPC network self-link or short name.
    pub network_uri: String,
    /// IPv6 subnetwork URI.
    pub subnetwork_uri: String,
    /// Automatically delete completed/idle Batch job (`scale_to_zero`).
    pub scale_to_zero: bool,
}

impl SpotBatchConfig {
    /// Builds an `e2-standard-4` Spot Batch configuration defaulting to `scale_to_zero = true`.
    pub fn e2_standard_4(
        project_id: impl Into<String>,
        region: impl Into<String>,
        job_id: impl Into<String>,
        container_image: impl Into<String>,
        commands: Vec<String>,
        network_uri: impl Into<String>,
        subnetwork_uri: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            region: region.into(),
            job_id: job_id.into(),
            machine_type: "e2-standard-4".to_owned(),
            max_run_secs: 1800,
            container_image: container_image.into(),
            commands,
            network_uri: network_uri.into(),
            subnetwork_uri: subnetwork_uri.into(),
            scale_to_zero: true,
        }
    }
}

/// Handle referencing an active or finished cloud Batch job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchJobHandle {
    /// Fully qualified job resource name.
    pub name: String,
    /// Unique job UID assigned by the provider.
    pub uid: String,
    /// Provider status state string (for example `QUEUED`, `RUNNING`, `SUCCEEDED`).
    pub state: String,
}
