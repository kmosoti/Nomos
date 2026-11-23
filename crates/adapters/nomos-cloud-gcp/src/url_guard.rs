//! Origin-pinned GCP REST URL construction and LRO `selfLink` validation.

use nomos_cloud::{CloudError, CloudResult};
use reqwest::Url;

/// Parses a GCP API base URL and rejects query strings, fragments, and plaintext non-loopback HTTP.
pub(crate) fn parse_api_base(raw: &str, label: &str) -> CloudResult<Url> {
    let url = Url::parse(raw)
        .map_err(|e| CloudError::InvalidSpec(format!("invalid {label} URL `{raw}`: {e}")))?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(CloudError::InvalidSpec(format!(
            "{label} base URL `{raw}` must not contain query or fragment"
        )));
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(url.host_str()) => {}
        other => {
            return Err(CloudError::InvalidSpec(format!(
                "{label} URL `{raw}` has disallowed scheme `{other}` (use https or loopback http)"
            )));
        }
    }
    if url.host_str().is_none() {
        return Err(CloudError::InvalidSpec(format!(
            "{label} URL `{raw}` missing host"
        )));
    }
    Ok(url)
}

/// Validates every dynamic segment and appends path + query safely onto `base_url`.
pub(crate) fn build_gcp_url(
    base_url: &str,
    label: &str,
    segments: &[&str],
    query: &[(&str, &str)],
) -> CloudResult<String> {
    let base = parse_api_base(base_url, label)?;
    let base_prefix = normalized_path_prefix(base.path());

    for seg in segments {
        validate_path_segment(seg, label)?;
    }

    let mut url = base.clone();
    {
        let mut path_mut = url.path_segments_mut().map_err(|()| {
            CloudError::InvalidSpec(format!("{label} base URL `{base_url}` cannot be a base"))
        })?;
        path_mut.pop_if_empty();
        for seg in segments {
            path_mut.push(seg);
        }
    }
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            validate_query_component(k, label)?;
            validate_query_component(v, label)?;
            pairs.append_pair(k, v);
        }
    }
    verify_same_origin_and_prefix(&base, base_prefix, &url, label)?;
    Ok(url.into())
}

/// Resolves and pins an LRO `selfLink` or `name` to the configured Compute Engine base origin/prefix.
pub(crate) fn resolve_lro_poll_url(
    compute_base_url: &str,
    self_link: Option<&str>,
    name: Option<&str>,
) -> CloudResult<String> {
    let base = parse_api_base(compute_base_url, "compute_base_url")?;
    let base_prefix = normalized_path_prefix(base.path());

    let resolved = match (self_link, name) {
        (Some(link), _) if link.contains("://") => Url::parse(link)
            .map_err(|e| CloudError::OperationFailed(format!("invalid LRO selfLink: {e}")))?,
        (Some(link), _) if link.starts_with('/') => base
            .join(link)
            .map_err(|e| CloudError::OperationFailed(format!("invalid relative selfLink: {e}")))?,
        (Some(rel), _) => join_segments(&base, rel, "LRO selfLink")?,
        (None, Some(op_name)) => join_segments(&base, op_name, "LRO name")?,
        (None, None) => {
            return Err(CloudError::OperationFailed(
                "LRO missing selfLink and name".into(),
            ));
        }
    };

    verify_same_origin_and_prefix(&base, base_prefix, &resolved, "LRO poll")?;
    Ok(resolved.into())
}

fn join_segments(base: &Url, rel: &str, label: &str) -> CloudResult<Url> {
    if rel.is_empty() || rel.contains('?') || rel.contains('#') || rel.contains('\\') {
        return Err(CloudError::OperationFailed(format!(
            "invalid {label} `{rel}`"
        )));
    }
    let mut url = base.clone();
    {
        let mut path_mut = url
            .path_segments_mut()
            .map_err(|()| CloudError::OperationFailed("invalid compute_base_url".into()))?;
        path_mut.pop_if_empty();
        for seg in rel.trim_start_matches('/').split('/') {
            if seg.is_empty() || seg == "." || seg == ".." || seg.contains('%') {
                return Err(CloudError::OperationFailed(format!(
                    "unsafe segment `{seg}` in {label}"
                )));
            }
            path_mut.push(seg);
        }
    }
    Ok(url)
}

fn verify_same_origin_and_prefix(
    base: &Url,
    base_prefix: &str,
    candidate: &Url,
    label: &str,
) -> CloudResult<()> {
    let same_origin = candidate.scheme() == base.scheme()
        && candidate.host_str() == base.host_str()
        && candidate.port_or_known_default() == base.port_or_known_default();
    if !same_origin {
        return Err(CloudError::InvalidSpec(format!(
            "{label} URL `{candidate}` escapes configured origin `{base}`"
        )));
    }
    let cand_path = candidate.path();
    let prefix_ok = cand_path == base_prefix
        || cand_path
            .strip_prefix(base_prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
    if !prefix_ok || cand_path.split('/').any(|s| s == "." || s == "..") {
        return Err(CloudError::InvalidSpec(format!(
            "{label} path `{cand_path}` escapes base prefix `{base_prefix}`"
        )));
    }
    Ok(())
}

fn normalized_path_prefix(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() { "/" } else { trimmed }
}

fn is_loopback_host(host: Option<&str>) -> bool {
    matches!(host, Some("127.0.0.1" | "::1" | "[::1]" | "localhost"))
}

fn validate_path_segment(seg: &str, label: &str) -> CloudResult<()> {
    if seg.is_empty()
        || seg == "."
        || seg == ".."
        || seg.bytes().any(|b| {
            b.is_ascii_whitespace()
                || b.is_ascii_control()
                || matches!(b, b'/' | b'\\' | b'?' | b'#' | b'%')
        })
    {
        return Err(CloudError::InvalidSpec(format!(
            "unsafe {label} path segment `{seg}`"
        )));
    }
    Ok(())
}

fn validate_query_component(val: &str, label: &str) -> CloudResult<()> {
    if val.is_empty()
        || val.bytes().any(|b| {
            b.is_ascii_whitespace() || b.is_ascii_control() || matches!(b, b'?' | b'#' | b'%')
        })
    {
        return Err(CloudError::InvalidSpec(format!(
            "unsafe {label} query component `{val}`"
        )));
    }
    Ok(())
}
