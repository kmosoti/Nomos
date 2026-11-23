//! Drift evaluation helpers comparing live GCE resources against [`TopologySpec`].

use nomos_cloud::{
    CloudDriftItem, CloudError, CloudFirewallRule, CloudNodeSpec, CloudResult, DriftStatus,
    TopologySpec,
};
use nomos_core::cipher::Secret;

use crate::rest_types::{GceFirewallResource, GceInstanceResource, GceSubnetResource};

pub(crate) struct ObservedDrift {
    pub item: CloudDriftItem,
    pub state_token: String,
    pub tags_fingerprint: Option<String>,
}

pub(crate) async fn check_subnet(
    client: &reqwest::Client,
    token: &Secret<String>,
    url: &str,
    spec: &TopologySpec,
) -> CloudResult<ObservedDrift> {
    let resp = client
        .get(url)
        .bearer_auth(token.expose())
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("GET subnet `{}`: {e}", spec.subnet_name)))?;
    if resp.status().as_u16() == 404 {
        return Ok(ObservedDrift {
            item: CloudDriftItem {
                kind: "subnetwork".into(),
                name: spec.subnet_name.clone(),
                status: DriftStatus::Create,
                detail: "subnetwork absent".into(),
            },
            state_token: "absent".to_owned(),
            tags_fingerprint: None,
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
    let state_token = state_hash(&[
        body.id.as_deref().unwrap_or(""),
        body.fingerprint.as_deref().unwrap_or(""),
        body.ipv6_cidr_range.as_deref().unwrap_or(""),
    ]);
    Ok(ObservedDrift {
        item: CloudDriftItem {
            kind: "subnetwork".into(),
            name: spec.subnet_name.clone(),
            status,
            detail: "evaluated subnetwork IPv6 CIDR".into(),
        },
        state_token,
        tags_fingerprint: None,
    })
}

pub(crate) async fn check_firewall(
    client: &reqwest::Client,
    token: &Secret<String>,
    url: &str,
    fw: &CloudFirewallRule,
) -> CloudResult<ObservedDrift> {
    let resp = client
        .get(url)
        .bearer_auth(token.expose())
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("GET firewall `{}`: {e}", fw.name)))?;
    if resp.status().as_u16() == 404 {
        return Ok(ObservedDrift {
            item: CloudDriftItem {
                kind: "firewall".into(),
                name: fw.name.clone(),
                status: DriftStatus::Create,
                detail: "firewall absent".into(),
            },
            state_token: "absent".to_owned(),
            tags_fingerprint: None,
        });
    }
    if !resp.status().is_success() {
        return Err(CloudError::Http(format!(
            "GET firewall `{}` HTTP {}",
            fw.name,
            resp.status()
        )));
    }
    let live: GceFirewallResource = resp
        .json()
        .await
        .map_err(|e| CloudError::Serialization(format!("decode firewall: {e}")))?;
    let dir_ok = live.direction.as_deref() == Some(fw.direction.as_str());
    let src_ok = live.source_ranges == fw.source_ranges_v6;
    let allowed_ok = live.allowed.len() == 1
        && live.allowed[0].ip_protocol == fw.protocol
        && live.allowed[0].ports == fw.ports;
    let status = if dir_ok && src_ok && allowed_ok {
        DriftStatus::Match
    } else {
        DriftStatus::Update
    };
    let mut parts = vec![
        live.id.as_deref().unwrap_or("").to_owned(),
        live.fingerprint.as_deref().unwrap_or("").to_owned(),
        live.direction.as_deref().unwrap_or("").to_owned(),
    ];
    parts.extend(live.source_ranges.iter().cloned());
    for a in &live.allowed {
        parts.push(a.ip_protocol.clone());
        parts.extend(a.ports.iter().cloned());
    }
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    Ok(ObservedDrift {
        item: CloudDriftItem {
            kind: "firewall".into(),
            name: fw.name.clone(),
            status,
            detail: "evaluated firewall direction, sourceRanges, and allowed ports".into(),
        },
        state_token: state_hash(&refs),
        tags_fingerprint: None,
    })
}

pub(crate) async fn check_instance(
    client: &reqwest::Client,
    token: &Secret<String>,
    url: &str,
    node: &CloudNodeSpec,
) -> CloudResult<ObservedDrift> {
    let resp = client
        .get(url)
        .bearer_auth(token.expose())
        .send()
        .await
        .map_err(|e| CloudError::Http(format!("GET instance `{}`: {e}", node.name)))?;
    if resp.status().as_u16() == 404 {
        return Ok(ObservedDrift {
            item: CloudDriftItem {
                kind: "instance".into(),
                name: node.name.clone(),
                status: DriftStatus::Create,
                detail: "instance absent".into(),
            },
            state_token: "absent".to_owned(),
            tags_fingerprint: None,
        });
    }
    if !resp.status().is_success() {
        return Err(CloudError::Http(format!(
            "GET instance `{}` HTTP {}",
            node.name,
            resp.status()
        )));
    }
    let vm: GceInstanceResource = resp
        .json()
        .await
        .map_err(|e| CloudError::Serialization(format!("decode instance: {e}")))?;
    let matches_type = vm.machine_type.as_deref().is_some_and(|mt| {
        mt == node.machine_type || mt.ends_with(&format!("/{}", node.machine_type))
    });
    let fingerprint = vm.tags.as_ref().and_then(|t| t.fingerprint.clone());
    let live_tags: &[String] = vm.tags.as_ref().map_or(&[], |t| t.items.as_slice());
    let matches_tags = live_tags == node.tags.as_slice();
    let mut parts = vec![
        vm.id.as_deref().unwrap_or("").to_owned(),
        vm.machine_type.as_deref().unwrap_or("").to_owned(),
        fingerprint.as_deref().unwrap_or("").to_owned(),
    ];
    parts.extend(live_tags.iter().cloned());
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    Ok(ObservedDrift {
        item: CloudDriftItem {
            kind: "instance".into(),
            name: node.name.clone(),
            status: if matches_type && matches_tags {
                DriftStatus::Match
            } else {
                DriftStatus::Update
            },
            detail: "evaluated instance machineType and tags".into(),
        },
        state_token: state_hash(&refs),
        tags_fingerprint: fingerprint,
    })
}

fn state_hash(fields: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for f in fields {
        h.update(&(f.len() as u64).to_le_bytes());
        h.update(f.as_bytes());
    }
    let hex = h.finalize().to_hex();
    hex.as_str()[..24].to_owned()
}
