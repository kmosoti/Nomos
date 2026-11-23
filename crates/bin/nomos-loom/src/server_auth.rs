//! Scoped bearer auth verification and symlink-safe `--web-assets-dir` fallback helpers.

use std::fs::{self, File};
use std::io::Read as _;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};

use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use nomos_core::cipher::Secret;

use crate::lease::LeaseBroker;

/// Maximum single static web asset size read into memory (16 MiB).
const MAX_STATIC_ASSET_BYTES: u64 = 16 * 1024 * 1024;

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, (StatusCode, String)> {
    let raw_hdr = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "missing Authorization: Bearer token".to_owned(),
            )
        })?;
    raw_hdr
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Authorization header must use Bearer scheme".to_owned(),
            )
        })
}

fn matches_control_token(control_token_hash: Option<blake3::Hash>, token: &str) -> bool {
    let Some(expected) = control_token_hash else {
        return false;
    };
    let provided = blake3::hash(token.as_bytes());
    provided == expected
}

/// Requires the root operator control token (non-delegable admin endpoints).
///
/// Child lease tokens are strictly rejected with `403 FORBIDDEN`.
pub(crate) fn authorize_control_admin(
    control_token_hash: Option<blake3::Hash>,
    leases: &LeaseBroker,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, String)> {
    let token_str = extract_bearer_token(headers)?;
    if matches_control_token(control_token_hash, token_str) {
        return Ok(());
    }
    let wrapped = Secret::new(token_str.to_owned());
    match leases.verify_token(&wrapped) {
        Ok(Some(_child_lease)) => Err((
            StatusCode::FORBIDDEN,
            "child lease tokens cannot perform non-delegable admin lease operations".to_owned(),
        )),
        Ok(None) => Err((
            StatusCode::UNAUTHORIZED,
            "invalid or unauthorized admin bearer token".to_owned(),
        )),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err)),
    }
}

/// Verifies `Authorization: Bearer <token>` against either the hashed operator
/// control token (constant-time BLAKE3 comparison) or an active scoped lease.
pub(crate) fn authorize_bearer(
    control_token_hash: Option<blake3::Hash>,
    leases: &LeaseBroker,
    headers: &HeaderMap,
    allowed_scopes: &[&str],
) -> Result<(), (StatusCode, String)> {
    let token_str = extract_bearer_token(headers)?;
    if matches_control_token(control_token_hash, token_str) {
        return Ok(());
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
    let root = root.clone();
    let rel = uri.path().trim_start_matches('/').to_owned();
    let res = tokio::task::spawn_blocking(move || read_static_or_spa(&root, &rel)).await;
    match res {
        Ok(AssetRead::Ok { mime, bytes }) => {
            let mut resp = Response::new(axum::body::Body::from(bytes));
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            resp.headers_mut().insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            resp
        }
        Ok(AssetRead::Forbidden) => StatusCode::FORBIDDEN.into_response(),
        Ok(AssetRead::NotFound) | Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

enum AssetRead {
    Ok { mime: &'static str, bytes: Vec<u8> },
    Forbidden,
    NotFound,
}

fn read_static_or_spa(root: &Path, rel: &str) -> AssetRead {
    let Ok(canon_root) = fs::canonicalize(root) else {
        return AssetRead::NotFound;
    };
    let requested = if rel.is_empty() { "index.html" } else { rel };
    match read_confined_regular_file(&canon_root, requested) {
        AssetRead::NotFound if requested != "index.html" => {
            read_confined_regular_file(&canon_root, "index.html")
        }
        other => other,
    }
}

fn read_confined_regular_file(canon_root: &Path, rel: &str) -> AssetRead {
    if rel.contains('%') || rel.contains('\\') || rel.contains('\0') {
        return AssetRead::Forbidden;
    }
    let mut cur = canon_root.to_path_buf();
    let mut segments = Vec::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(seg) => segments.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return AssetRead::Forbidden;
            }
        }
    }
    if segments.is_empty() {
        return AssetRead::Forbidden;
    }
    for (idx, seg) in segments.iter().enumerate() {
        cur.push(seg);
        let Ok(lmeta) = fs::symlink_metadata(&cur) else {
            return AssetRead::NotFound;
        };
        if lmeta.file_type().is_symlink() {
            return AssetRead::Forbidden;
        }
        let is_last = idx + 1 == segments.len();
        if !is_last && !lmeta.file_type().is_dir() {
            return AssetRead::Forbidden;
        }
    }

    let Ok(lmeta) = fs::symlink_metadata(&cur) else {
        return AssetRead::NotFound;
    };
    if lmeta.file_type().is_symlink() || !lmeta.file_type().is_file() {
        return if lmeta.file_type().is_symlink() {
            AssetRead::Forbidden
        } else {
            AssetRead::NotFound
        };
    }
    let Ok(canon_target) = fs::canonicalize(&cur) else {
        return AssetRead::NotFound;
    };
    if !canon_target.starts_with(canon_root) {
        return AssetRead::Forbidden;
    }

    let Ok(file) = File::open(&canon_target) else {
        return AssetRead::NotFound;
    };
    let Ok(fmeta) = file.metadata() else {
        return AssetRead::NotFound;
    };
    if !fmeta.file_type().is_file() || fmeta.len() > MAX_STATIC_ASSET_BYTES {
        return AssetRead::Forbidden;
    }
    #[cfg(unix)]
    if (fmeta.dev(), fmeta.ino()) != (lmeta.dev(), lmeta.ino()) {
        return AssetRead::Forbidden;
    }

    let mut buf = Vec::with_capacity(fmeta.len().min(64 * 1024) as usize);
    if file
        .take(MAX_STATIC_ASSET_BYTES + 1)
        .read_to_end(&mut buf)
        .is_err()
        || (buf.len() as u64) > MAX_STATIC_ASSET_BYTES
    {
        return AssetRead::Forbidden;
    }

    AssetRead::Ok {
        mime: mime_for_path(&canon_target),
        bytes: buf,
    }
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
