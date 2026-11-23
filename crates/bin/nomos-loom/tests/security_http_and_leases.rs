//! Security regression suite covering Daybreak HTTP, static asset, and SQLite findings (#4, #5, #7, #16).

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use nomos_core::cipher::Secret;
use nomos_core::governor::GovernorConfig;
use nomos_loom::governor_svc::GovernorService;
use nomos_loom::lease::{IssueLeaseRequest, LeaseBroker};
use nomos_loom::server::{LoomState, build_router};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "nomos-loom-hsec-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn http_auth_non_delegable_leases_symlink_assets_and_sqlite_pruning() {
    let tmp = TempDir::new("http-sec");
    let db_path = tmp.0.join("leases.sqlite");
    let assets_dir = tmp.0.join("web");
    let outside_secret = tmp.0.join("secret.txt");
    fs::create_dir_all(&assets_dir).unwrap();
    fs::write(assets_dir.join("index.html"), "<html>ok</html>").unwrap();
    fs::write(&outside_secret, "TOP-SECRET-KEY").unwrap();
    symlink(&outside_secret, assets_dir.join("leak.txt")).unwrap();

    // #7: SQLite pruning + scope allowlist + subject checks.
    let leases = LeaseBroker::open(&db_path, 3).unwrap();
    assert!(
        leases
            .issue(&IssueLeaseRequest {
                subject: "attacker".to_owned(),
                scope: "admin".to_owned(),
                ttl_secs: 60,
            })
            .is_err(),
        "admin scope must never be delegable via lease issuance"
    );
    for idx in 0..15 {
        let issued = leases
            .issue(&IssueLeaseRequest {
                subject: format!("worker-{idx}"),
                scope: "batch".to_owned(),
                ttl_secs: 60,
            })
            .unwrap();
        assert!(leases.revoke(issued.record.id).unwrap());
    }
    assert!(
        leases.total_row_count().unwrap() <= 3,
        "revoked/expired rows must be pruned to keep DB bounded"
    );

    let admin_token = "root-admin-control-token-99".to_owned();
    let state = LoomState::new(
        "coord-http",
        leases.clone(),
        GovernorService::new(GovernorConfig::default()),
        None,
        Some(assets_dir),
    )
    .with_control_token(&Secret::new(admin_token.clone()));

    let listener = TcpListener::bind("[::1]:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, build_router(state)).await;
    });
    let base = format!("http://[::1]:{}", addr.port());
    let client = reqwest::Client::new();

    // #16: /health stays minimal; observability endpoints require bearer auth.
    let health: Value = client
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["status"], "ok");
    assert!(health.get("node").is_none());
    assert!(health.get("generation").is_none());

    for path in ["/v1/leases", "/v1/governor", "/v1/telemetry", "/v1/events"] {
        let resp = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "GET {path} must 401"
        );
    }

    // #5: Even a "control"-scoped child lease MUST receive 403 on POST /v1/leases & DELETE.
    let issued_ctrl = leases
        .issue(&IssueLeaseRequest {
            subject: "sub-controller".to_owned(),
            scope: "control".to_owned(),
            ttl_secs: 120,
        })
        .unwrap();
    let delegate_resp = client
        .post(format!("{base}/v1/leases"))
        .bearer_auth(issued_ctrl.token.expose())
        .json(&IssueLeaseRequest {
            subject: "escalated".to_owned(),
            scope: "control".to_owned(),
            ttl_secs: 60,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(delegate_resp.status(), StatusCode::FORBIDDEN);

    let observer_ok = client
        .get(format!("{base}/v1/telemetry"))
        .bearer_auth(issued_ctrl.token.expose())
        .send()
        .await
        .unwrap();
    assert_eq!(observer_ok.status(), StatusCode::OK);

    // #4: Symlink and raw traversal asset paths rejected with 403 FORBIDDEN without reading outside root.
    let sym = client.get(format!("{base}/leak.txt")).send().await.unwrap();
    assert_eq!(sym.status(), StatusCode::FORBIDDEN);
    for raw_path in [
        "/%2e%2e/secret.txt",
        "/../secret.txt",
        "/%2e%2e%5csecret.txt",
    ] {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let req =
            format!("GET {raw_path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut resp_bytes = Vec::new();
        stream.read_to_end(&mut resp_bytes).await.unwrap();
        let head = String::from_utf8_lossy(&resp_bytes);
        assert!(
            head.starts_with("HTTP/1.1 403"),
            "raw traversal `{raw_path}` must return 403, got: {head}"
        );
    }

    let root_ok = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(root_ok.status(), StatusCode::OK);
    assert_eq!(root_ok.text().await.unwrap(), "<html>ok</html>");
}
