//! Scoped bearer auth verification and static `--web-assets-dir` fallback helpers.

use std::path::{Component, Path, PathBuf};

use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use nomos_core::cipher::Secret;

use crate::lease::LeaseBroker;

/// Verifies `Authorization: Bearer <token>` against either the hashed operator
/// control token (constant-time BLAKE3 comparison) or an active scoped lease.
pub(crate) fn authorize_bearer(
    control_token_hash: Option<blake3::Hash>,
    leases: &LeaseBroker,
    headers: &HeaderMap,
    allowed_scopes: &[&str],
) -> Result<(), (StatusCode, String)> {
    let raw_hdr = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "missing Authorization: Bearer token".to_owned(),
            )
        })?;
    let token_str = raw_hdr
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Authorization header must use Bearer scheme".to_owned(),
            )
        })?;

    if let Some(expected_hash) = control_token_hash {
        let provided_hash = blake3::hash(token_str.as_bytes());
        if provided_hash == expected_hash {
            return Ok(());
        }
    }

    let wrapped = Secret::new(token_str.to_owned());
    match leases.verify_token(&wrapped) {
        Ok(Some(lease)) if allowed_scopes.contains(&lease.scope.as_str()) => Ok(()),
        Ok(Some(lease)) => Err((
            StatusCode::FORBIDDEN,
            format!("lease scope `{}` insufficient for endpoint", lease.scope),
        )),
        Ok(None) => Err((
            StatusCode::UNAUTHORIZED,
            "invalid or expired bearer token".to_owned(),
        )),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err)),
    }
}

pub(crate) async fn serve_web_asset_impl(web_assets_dir: Option<&PathBuf>, uri: Uri) -> Response {
    let Some(root) = web_assets_dir else {
        return (
            StatusCode::NOT_FOUND,
            "headless nomos-loom: no --web-assets-dir configured",
        )
            .into_response();
    };
    let rel = uri.path().trim_start_matches('/');
    let Some(candidate) = resolve_safe_asset(root, if rel.is_empty() { "index.html" } else { rel })
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let target = if candidate.is_file() {
        candidate
    } else {
        let Some(index) = resolve_safe_asset(root, "index.html") else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if !index.is_file() {
            return StatusCode::NOT_FOUND.into_response();
        }
        index
    };
    match tokio::fs::read(&target).await {
        Ok(bytes) => {
            let mime = mime_for_path(&target);
            let mut resp = Response::new(axum::body::Body::from(bytes));
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            resp
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn resolve_safe_asset(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut joined = root.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(seg) => joined.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(joined)
}

fn mime_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
