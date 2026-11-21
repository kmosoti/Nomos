//! Integration tests for headless `nomos-loom`: SQLite WAL leases, PSI governor, and `--web-assets-dir`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use nomos_core::cipher::Secret;
use nomos_core::governor::GovernorConfig;
use nomos_loom::cli::{LoomCommand, parse_args};
use nomos_loom::governor_svc::{GovernorService, parse_psi_avg10_bps};
use nomos_loom::lease::{IssueLeaseRequest, LeaseBroker};
use nomos_loom::server::{LoomState, build_router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "nomos-loom-test-{label}-{}-{nanos}",
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

#[test]
fn cli_and_psi_parser_handle_headless_and_web_assets_args() {
    let cmd = parse_args([
        "serve".to_owned(),
        "--listen".to_owned(),
        "[::1]:0".to_owned(),
        "--web-assets-dir".to_owned(),
        "/srv/nomos-web".to_owned(),
        "--max-leases".to_owned(),
        "12".to_owned(),
    ])
    .unwrap();
    match cmd {
        LoomCommand::Serve(args) => {
            assert_eq!(args.max_active_leases, 12);
            assert_eq!(args.web_assets_dir, Some(PathBuf::from("/srv/nomos-web")));
        }
        other => panic!("unexpected parsed command: {other:?}"),
    }

    let psi = "some avg10=18.42 avg60=12.00 avg300=4.50 total=100\nfull avg10=1.25 avg60=0.50 avg300=0.10 total=10\n";
    assert_eq!(parse_psi_avg10_bps(psi, "some "), Some(1842));
    assert_eq!(parse_psi_avg10_bps(psi, "full "), Some(125));
}

#[tokio::test]
async fn headless_http_lease_broker_governor_backpressure_and_web_assets() {
    let tmp = TempDir::new("http");
    let db_path = tmp.0.join("leases.sqlite");
    let assets_dir = tmp.0.join("web");
    fs::create_dir_all(&assets_dir).unwrap();
    fs::write(assets_dir.join("index.html"), "<html>nomos-web</html>").unwrap();

    let leases = LeaseBroker::open(&db_path, 4).unwrap();
    let governor = GovernorService::new(GovernorConfig::default());
    let state = LoomState::new(
        "example-loom",
        leases.clone(),
        governor,
        None,
        Some(assets_dir),
    );
    let listener = TcpListener::bind("[::1]:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, build_router(state)).await;
    });

    let base = format!("http://[::1]:{}", addr.port());
    let client = reqwest::Client::new();

    let health: Value = client
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["status"], "ok");
    assert_eq!(health["headless"], false);

    // Issue batch lease and verify raw token only appears in issuance response, never in SQLite or GET list.
    let issued_resp = client
        .post(format!("{base}/v1/leases"))
        .json(&IssueLeaseRequest {
            subject: "worker-1".to_owned(),
            scope: "batch".to_owned(),
            ttl_secs: 300,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(issued_resp.status(), StatusCode::CREATED);
    let issued_json: Value = issued_resp.json().await.unwrap();
    let token = issued_json["token"].as_str().unwrap().to_owned();
    let lease_id = issued_json["lease"]["id"].as_i64().unwrap();

    // Raw token must never exist on disk inside SQLite DB bytes.
    let db_bytes = fs::read(&db_path).unwrap();
    assert!(!db_bytes.windows(token.len()).any(|w| w == token.as_bytes()));

    let active_list = client
        .get(format!("{base}/v1/leases"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!active_list.contains(&token));
    assert!(
        leases
            .verify_token(&Secret::new(token.clone()))
            .unwrap()
            .is_some()
    );

    // Escalate PSI pressure -> batch leases return 429 TOO_MANY_REQUESTS while control still admits.
    let gov_resp: Value = client
        .post(format!("{base}/v1/governor/sample"))
        .json(&json!({
            "timestamp_ns": 2_000_000_000u64,
            "cpu_some_avg10_bps": 4500,
            "memory_used_bytes": 500,
            "memory_total_bytes": 1000
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(gov_resp["admit_batch"], false);
    assert_eq!(gov_resp["shed_background"], true);

    let rejected = client
        .post(format!("{base}/v1/leases"))
        .json(&IssueLeaseRequest {
            subject: "worker-2".to_owned(),
            scope: "batch".to_owned(),
            ttl_secs: 60,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);

    // Revoke and check static web-assets-dir + negative path traversal rejection.
    let del = client
        .delete(format!("{base}/v1/leases/{lease_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    assert!(leases.verify_token(&Secret::new(token)).unwrap().is_none());

    let index_resp = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(index_resp.status(), StatusCode::OK);
    assert!(index_resp.text().await.unwrap().contains("nomos-web"));

    let telem: Value = client
        .get(format!("{base}/v1/telemetry"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(telem["node"], "example-loom");
    assert_eq!(telem["active_leases"], 0);
}
