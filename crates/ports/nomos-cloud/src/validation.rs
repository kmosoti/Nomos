//! Strict GCP identifier, CIDR, firewall, and resource path validation helpers.

use std::net::Ipv6Addr;

use crate::{CloudError, CloudFirewallRule, CloudResult};

/// Validates a GCP `project_id` (`6..=30` lowercase letters, digits, hyphens; starts `[a-z]`, ends `[a-z0-9]`).
pub fn validate_gcp_project_id(project_id: &str) -> CloudResult<()> {
    let bytes = project_id.as_bytes();
    if !(6..=30).contains(&bytes.len())
        || !bytes[0].is_ascii_lowercase()
        || !(bytes[bytes.len() - 1].is_ascii_lowercase() || bytes[bytes.len() - 1].is_ascii_digit())
        || !bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    {
        return Err(CloudError::InvalidSpec(format!(
            "invalid GCP project_id `{project_id}`"
        )));
    }
    Ok(())
}

/// Validates a GCP RFC1035 resource name (`1..=63` chars `[a-z0-9-]`, starts `[a-z]`, ends `[a-z0-9]`).
pub fn validate_gcp_resource_name(kind: &str, value: &str) -> CloudResult<()> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 63
        || !bytes[0].is_ascii_lowercase()
        || !(bytes[bytes.len() - 1].is_ascii_lowercase() || bytes[bytes.len() - 1].is_ascii_digit())
        || !bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    {
        return Err(CloudError::InvalidSpec(format!(
            "invalid GCP {kind} `{value}`"
        )));
    }
    Ok(())
}

/// Validates a pure-IPv6 address or CIDR block (`<ipv6>` or `<ipv6>/<0..=128>`).
pub fn validate_ipv6_cidr(kind: &str, cidr: &str) -> CloudResult<()> {
    if cidr.is_empty() || contains_forbidden_uri_chars(cidr) {
        return Err(CloudError::InvalidSpec(format!(
            "{kind} `{cidr}` contains invalid characters"
        )));
    }
    if let Some((ip_str, prefix_str)) = cidr.split_once('/') {
        ip_str.parse::<Ipv6Addr>().map_err(|_| {
            CloudError::InvalidSpec(format!("{kind} `{cidr}` must contain a valid IPv6 address"))
        })?;
        let prefix: u8 = prefix_str.parse().map_err(|_| {
            CloudError::InvalidSpec(format!("{kind} `{cidr}` has invalid prefix length"))
        })?;
        if prefix > 128 {
            return Err(CloudError::InvalidSpec(format!(
                "{kind} `{cidr}` prefix length must be <= 128"
            )));
        }
        return Ok(());
    }
    cidr.parse::<Ipv6Addr>().map_err(|_| {
        CloudError::InvalidSpec(format!("{kind} `{cidr}` must be a valid IPv6 address/CIDR"))
    })?;
    Ok(())
}

/// Validates a relative GCP resource path or `https://` URI without traversal or query/fragment smuggling.
pub fn validate_gcp_path_or_uri(kind: &str, raw: &str) -> CloudResult<()> {
    if raw.is_empty() || contains_forbidden_uri_chars(raw) {
        return Err(CloudError::InvalidSpec(format!(
            "{kind} `{raw}` is empty or contains disallowed characters"
        )));
    }
    let path = if let Some(rest) = raw.strip_prefix("https://") {
        let Some((host, tail)) = rest.split_once('/') else {
            return Err(CloudError::InvalidSpec(format!(
                "{kind} `{raw}` HTTPS URI must include resource path"
            )));
        };
        if host.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
        {
            return Err(CloudError::InvalidSpec(format!(
                "{kind} `{raw}` contains invalid HTTPS host"
            )));
        }
        tail
    } else if raw.contains("://") {
        return Err(CloudError::InvalidSpec(format!(
            "{kind} `{raw}` scheme must be https or a relative resource path"
        )));
    } else {
        raw
    };

    let mut count = 0usize;
    for seg in path.split('/') {
        count += 1;
        if seg.is_empty()
            || seg == "."
            || seg == ".."
            || !seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(CloudError::InvalidSpec(format!(
                "{kind} `{raw}` contains invalid path segment `{seg}`"
            )));
        }
    }
    if count == 0 {
        return Err(CloudError::InvalidSpec(format!("{kind} must not be empty")));
    }
    Ok(())
}

/// Validates firewall rule direction, protocol, ports, and IPv6 source ranges.
pub fn validate_firewall_rule(fw: &CloudFirewallRule) -> CloudResult<()> {
    validate_gcp_resource_name("firewall_rule.name", &fw.name)?;
    if !matches!(fw.direction.as_str(), "INGRESS" | "EGRESS") {
        return Err(CloudError::InvalidSpec(format!(
            "firewall `{}` direction `{}` must be INGRESS or EGRESS",
            fw.name, fw.direction
        )));
    }
    if fw.source_ranges_v6.is_empty() {
        return Err(CloudError::InvalidSpec(format!(
            "firewall `{}` must define pure-IPv6 source ranges",
            fw.name
        )));
    }
    for range in &fw.source_ranges_v6 {
        validate_ipv6_cidr("firewall source_ranges_v6", range)?;
    }
    let proto_ok = matches!(
        fw.protocol.as_str(),
        "tcp" | "udp" | "icmp" | "icmpv6" | "esp" | "ah" | "sctp" | "all"
    ) || fw.protocol.parse::<u8>().is_ok();
    if !proto_ok {
        return Err(CloudError::InvalidSpec(format!(
            "firewall `{}` has unsupported protocol `{}`",
            fw.name, fw.protocol
        )));
    }
    for port in &fw.ports {
        validate_port_spec(&fw.name, port)?;
    }
    Ok(())
}

fn validate_port_spec(fw_name: &str, spec: &str) -> CloudResult<()> {
    if spec.is_empty() || contains_forbidden_uri_chars(spec) {
        return Err(CloudError::InvalidSpec(format!(
            "firewall `{fw_name}` has invalid port `{spec}`"
        )));
    }
    if let Some((start_s, end_s)) = spec.split_once('-') {
        let start: u16 = start_s.parse().map_err(|_| {
            CloudError::InvalidSpec(format!("firewall `{fw_name}` invalid port range `{spec}`"))
        })?;
        let end: u16 = end_s.parse().map_err(|_| {
            CloudError::InvalidSpec(format!("firewall `{fw_name}` invalid port range `{spec}`"))
        })?;
        if start == 0 || start > end {
            return Err(CloudError::InvalidSpec(format!(
                "firewall `{fw_name}` invalid port bounds `{spec}`"
            )));
        }
        return Ok(());
    }
    let port: u16 = spec.parse().map_err(|_| {
        CloudError::InvalidSpec(format!("firewall `{fw_name}` invalid port `{spec}`"))
    })?;
    if port == 0 {
        return Err(CloudError::InvalidSpec(format!(
            "firewall `{fw_name}` port must be >= 1"
        )));
    }
    Ok(())
}

pub(crate) fn contains_forbidden_uri_chars(s: &str) -> bool {
    s.bytes().any(|b| {
        b.is_ascii_whitespace() || b.is_ascii_control() || matches!(b, b'?' | b'#' | b'%' | b'\\')
    })
}
