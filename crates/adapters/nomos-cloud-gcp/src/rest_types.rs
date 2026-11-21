//! JSON wire DTOs for GCP Compute Engine REST v1 requests and operations.

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub(crate) struct GceOperationResponse {
    pub name: Option<String>,
    #[serde(rename = "selfLink")]
    pub self_link: Option<String>,
    pub status: Option<String>,
    pub error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub(crate) struct GceSubnetResource {
    #[serde(rename = "ipv6CidrRange")]
    pub ipv6_cidr_range: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct GceInstanceResource {
    #[serde(rename = "machineType")]
    pub machine_type: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct CreateSubnetBody<'a> {
    pub name: &'a str,
    pub network: String,
    #[serde(rename = "stackType")]
    pub stack_type: &'static str,
    #[serde(rename = "ipv6AccessType")]
    pub ipv6_access_type: &'static str,
}

#[derive(Serialize)]
pub(crate) struct CreateFirewallBody<'a> {
    pub name: &'a str,
    pub network: String,
    pub direction: &'a str,
    #[serde(rename = "sourceRanges")]
    pub source_ranges: &'a [String],
    pub allowed: [FirewallAllowed<'a>; 1],
}

#[derive(Serialize)]
pub(crate) struct FirewallAllowed<'a> {
    #[serde(rename = "IPProtocol")]
    pub ip_protocol: &'a str,
    pub ports: &'a [String],
}

#[derive(Serialize)]
pub(crate) struct CreateInstanceBody<'a> {
    pub name: &'a str,
    #[serde(rename = "machineType")]
    pub machine_type: String,
    #[serde(rename = "networkInterfaces")]
    pub network_interfaces: [InstanceNetIf; 1],
    pub disks: [InstanceDisk<'a>; 1],
}

#[derive(Serialize)]
pub(crate) struct InstanceNetIf {
    pub subnetwork: String,
    #[serde(rename = "stackType")]
    pub stack_type: &'static str,
    #[serde(rename = "ipv6Address")]
    pub ipv6_address: String,
}

#[derive(Serialize)]
pub(crate) struct InstanceDisk<'a> {
    pub boot: bool,
    #[serde(rename = "autoDelete")]
    pub auto_delete: bool,
    #[serde(rename = "initializeParams")]
    pub initialize_params: DiskInitParams<'a>,
}

#[derive(Serialize)]
pub(crate) struct DiskInitParams<'a> {
    #[serde(rename = "sourceImage")]
    pub source_image: &'a str,
}
