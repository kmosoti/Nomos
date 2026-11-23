//! Config-driven cloud topology specification and Spot Batch request types.

use nomos_core::resource::Digest;
use serde::{Deserialize, Serialize};

use crate::validation::{
    validate_firewall_rule, validate_gcp_path_or_uri, validate_gcp_project_id,
    validate_gcp_resource_name, validate_ipv6_cidr,
};
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
    /// Validates project, GCP resource naming grammars, IPv6 CIDRs, paths, and returns digest.
    pub fn validate(&self) -> CloudResult<Digest> {
        validate_gcp_project_id(&self.project_id)?;
        validate_gcp_resource_name("region", &self.region)?;
        validate_gcp_resource_name("network_name", &self.network_name)?;
        validate_gcp_resource_name("subnet_name", &self.subnet_name)?;
        validate_ipv6_cidr("subnet_ipv6_cidr", &self.subnet_ipv6_cidr)?;
        for node in &self.nodes {
            validate_gcp_resource_name("node.name", &node.name)?;
            validate_gcp_resource_name("node.zone", &node.zone)?;
            validate_gcp_resource_name("node.machine_type", &node.machine_type)?;
            validate_ipv6_cidr("node.ipv6_cidr", &node.ipv6_cidr)?;
            validate_gcp_path_or_uri("node.source_image", &node.source_image)?;
            for tag in &node.tags {
                validate_gcp_resource_name("node.tag", tag)?;
            }
        }
        for fw in &self.firewall_rules {
            validate_firewall_rule(fw)?;
        }
        Ok(self.spec_digest())
    }

    /// Computes deterministic 32-byte BLAKE3 digest across ALL topology fields.
    pub fn spec_digest(&self) -> Digest {
        let mut hasher = blake3::Hasher::new();
        let feed_str = |h: &mut blake3::Hasher, s: &str| {
            h.update(&(s.len() as u64).to_le_bytes());
            h.update(s.as_bytes());
        };
        hasher.update(b"nomos/topology-spec/v1");
        feed_str(&mut hasher, &self.project_id);
        feed_str(&mut hasher, &self.region);
        feed_str(&mut hasher, &self.network_name);
        feed_str(&mut hasher, &self.subnet_name);
        feed_str(&mut hasher, &self.subnet_ipv6_cidr);

        hasher.update(&(self.nodes.len() as u64).to_le_bytes());
        for node in &self.nodes {
            feed_str(&mut hasher, &node.name);
            feed_str(&mut hasher, &node.zone);
            feed_str(&mut hasher, &node.machine_type);
            feed_str(&mut hasher, &node.ipv6_cidr);
            feed_str(&mut hasher, &node.source_image);
            hasher.update(&(node.tags.len() as u64).to_le_bytes());
            for tag in &node.tags {
                feed_str(&mut hasher, tag);
            }
        }

        hasher.update(&(self.firewall_rules.len() as u64).to_le_bytes());
        for fw in &self.firewall_rules {
            feed_str(&mut hasher, &fw.name);
            feed_str(&mut hasher, &fw.direction);
            feed_str(&mut hasher, &fw.protocol);
            hasher.update(&(fw.source_ranges_v6.len() as u64).to_le_bytes());
            for range in &fw.source_ranges_v6 {
                feed_str(&mut hasher, range);
            }
            hasher.update(&(fw.ports.len() as u64).to_le_bytes());
            for port in &fw.ports {
                feed_str(&mut hasher, port);
            }
        }

        Digest::from_bytes(*hasher.finalize().as_bytes())
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
    /// Validates Spot Batch project/region/job identifiers and network/container specs.
    pub fn validate(&self) -> CloudResult<()> {
        validate_gcp_project_id(&self.project_id)?;
        validate_gcp_resource_name("region", &self.region)?;
        validate_gcp_resource_name("job_id", &self.job_id)?;
        validate_gcp_resource_name("machine_type", &self.machine_type)?;
        validate_gcp_path_or_uri("network_uri", &self.network_uri)?;
        validate_gcp_path_or_uri("subnetwork_uri", &self.subnetwork_uri)?;
        if self.container_image.is_empty()
            || self
                .container_image
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        {
            return Err(CloudError::InvalidSpec(format!(
                "invalid container_image `{}`",
                self.container_image
            )));
        }
        if self.max_run_secs == 0 {
            return Err(CloudError::InvalidSpec(
                "max_run_secs must be greater than zero".into(),
            ));
        }
        Ok(())
    }

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
