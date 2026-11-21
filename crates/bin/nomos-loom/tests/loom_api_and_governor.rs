//! Integration tests for headless `nomos-loom`: scoped bearer auth, SQLite WAL leases,
//! Holt-Winters/OLS/Gradient2 PSI governor, real KernelService/Warp/Fence QUIC RPC, and web assets.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use nomos_canon::artifact::{Profile, canon_id, encode};
use nomos_canon::model::{Canon, RawCanon, RawResource, RawSpec};
use nomos_core::cipher::Secret;
use nomos_core::governor::GovernorConfig;
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::Digest;
use nomos_loom::cli::{LoomCommand, parse_args};
use nomos_loom::governor_svc::{GovernorService, parse_meminfo_bytes, parse_psi_avg10_bps};
use nomos_loom::kernel_svc::KernelService;
use nomos_loom::lease::{IssueLeaseRequest, LeaseBroker};
use nomos_loom::quic_svc::LoomQuicHandler;
use nomos_loom::server::{LoomState, build_router};
use nomos_protocol::{ControlEnvelope, ControlOp};
use nomos_protocol_quic::ControlHandler;
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

fn sample_canon_jcs() -> (Vec<u8>, Digest) {
    let raw = RawCanon {
        name: "loom-demo".to_owned(),
        resources: vec![RawResource {
            path: "/etc/nomos/demo.conf".to_owned(),
            spec: RawSpec {
                kind: "file".to_owned(),
                state: "present-any".to_owned(),
                digest: None,
            },
            keys: Vec::new(),
            disrupts: Vec::new(),
        }],
        relations: Vec::new(),
    };
    let canon = Canon::try_from(raw).unwrap();
    let bytes = encode(&canon, Profile::Jcs);
    let cid = canon_id(&canon, Profile::Jcs);
    (bytes, Digest::from_bytes(*cid.as_bytes()))
}

#[test]
fn cli_psi_and_meminfo_parsers_handle_new_flags() {
    let cmd = parse_args([
        "serve".to_owned(),
        "--listen".to_owned(),
        "[::1]:0".to_owned(),
        "--web-assets-dir".to_owned(),
        "/srv/nomos-web".to_owned(),
        "--max-leases".to_owned(),
        "12".to_owned(),
        "--allowed-client-pin".to_owned(),
        "ab".repeat(32),
        "--allow-plaintext-key".to_owned(),
    ])
    .unwrap();
    match cmd {
        LoomCommand::Serve(args) => {
            assert_eq!(args.max_active_leases, 12);
            assert_eq!(args.web_assets_dir, Some(PathBuf::from("/srv/nomos-web")));
            assert_eq!(args.allowed_client_pins.len(), 1);
            assert!(args.allow_plaintext_key);
        }
        other => panic!("unexpected parsed command: {other:?}"),
    }

    let psi = "some avg10=18.42 avg60=12.00 avg300=4.50 total=100\nfull avg10=1.25 avg60=0.50 avg300=0.10 total=10\n";
    assert_eq!(parse_psi_avg10_bps(psi, "some "), Some(1842));
    assert_eq!(parse_psi_avg10_bps(psi, "full "), Some(125));

    let meminfo =
        "MemTotal:       16384000 kB\nMemFree:         2000000 kB\nMemAvailable:    4096000 kB\n";
    let (used, total) = parse_meminfo_bytes(meminfo).unwrap();
    assert_eq!(total, 16_384_000 * 1024);
    assert_eq!(used, (16_384_000 - 4_096_000) * 1024);
}

#[tokio::test]
async fn scoped_bearer_auth_governor_backpressure_and_web_assets() {
    let tmp = TempDir::new("http");
    let db_path = tmp.0.join("leases.sqlite");
    let assets_dir = tmp.0.join("web");
    fs::create_dir_all(&assets_dir).unwrap();
    fs::write(assets_dir.join("index.html"), "<html>nomos-web</html>").unwrap();

    let admin_token = "loom-admin-bearer-test-token-77".to_owned();
    let leases = LeaseBroker::open(&db_path, 4).unwrap();
    let governor = GovernorService::new(GovernorConfig::default());
    let state = LoomState::new(
        "example-loom",
        leases.clone(),
        governor,
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

    // Unauthenticated mutating endpoints MUST return 401 UNAUTHORIZED.
    let unauth_lease = client
        .post(format!("{base}/v1/leases"))
        .json(&IssueLeaseRequest {
            subject: "anon".to_owned(),
            scope: "batch".to_owned(),
            ttl_secs: 60,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_lease.status(), StatusCode::UNAUTHORIZED);

    let unauth_sample = client
        .post(format!("{base}/v1/governor/sample"))
        .json(&json!({
            "timestamp_ns": 1_000_000_000u64,
            "cpu_some_avg10_bps": 100,
            "memory_used_bytes": 100,
            "memory_total_bytes": 1000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_sample.status(), StatusCode::UNAUTHORIZED);

    // Admin bearer token issues a batch lease; batch token itself MUST be 403 on admin endpoints.
    let issued_resp = client
        .post(format!("{base}/v1/leases"))
        .bearer_auth(&admin_token)
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
    let batch_token = issued_json["token"].as_str().unwrap().to_owned();
    let lease_id = issued_json["lease"]["id"].as_i64().unwrap();

    let forbidden = client
        .delete(format!("{base}/v1/leases/{lease_id}"))
        .bearer_auth(&batch_token)
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    // Raw token must never exist on disk inside SQLite DB bytes.
    let db_bytes = fs::read(&db_path).unwrap();
    assert!(
        !db_bytes
            .windows(batch_token.len())
            .any(|w| w == batch_token.as_bytes())
    );

    // Escalate PSI pressure with authorized admin token -> 429 on new batch leases.
    let gov_resp: Value = client
        .post(format!("{base}/v1/governor/sample"))
        .bearer_auth(&admin_token)
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
        .bearer_auth(&admin_token)
        .json(&IssueLeaseRequest {
            subject: "worker-2".to_owned(),
            scope: "batch".to_owned(),
            ttl_secs: 60,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);

    // Revoke with admin bearer token succeeds and invalidates lease.
    let del = client
        .delete(format!("{base}/v1/leases/{lease_id}"))
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    assert!(
        leases
            .verify_token(&Secret::new(batch_token))
            .unwrap()
            .is_none()
    );

    let index_resp = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(index_resp.status(), StatusCode::OK);
    assert!(index_resp.text().await.unwrap().contains("nomos-web"));
}

#[tokio::test]
async fn quic_handler_executes_real_kernel_warp_fence_and_fsstore_persistence() {
    let tmp = TempDir::new("kernel");
    let state_dir = tmp.0.join("state");
    let db_path = tmp.0.join("leases.sqlite");

    let kernel = KernelService::open(&state_dir).unwrap();
    let leases = LeaseBroker::open(&db_path, 4).unwrap();
    let governor = GovernorService::new(GovernorConfig::default());
    let handler =
        LoomQuicHandler::new("coord-0".to_owned(), governor, leases).with_kernel(kernel.clone());

    let (canon_bytes, digest) = sample_canon_jcs();
    let canon_json = String::from_utf8(canon_bytes).unwrap();
    let plan = PlanId::new("plan-real-1").unwrap();

    // 1. Valid Canon + monotonically advancing generation accepted & persisted to FsStore.
    let req = ControlEnvelope::new(
        ControlOp::SubmitPlan,
        Some(&plan),
        Generation(10),
        &digest,
        canon_json.clone(),
    );
    let reply = handler.handle(req).await;
    assert!(reply.ok, "valid plan must succeed: {}", reply.message);
    assert_eq!(reply.generation, 10);
    assert_eq!(kernel.journal_len(), 1);
    assert_eq!(kernel.current_generation(), 10);

    // 2. Stale generation behind Fence MUST be rejected (ok == false).
    let stale_plan = PlanId::new("plan-stale-9").unwrap();
    let stale_req = ControlEnvelope::new(
        ControlOp::SubmitPlan,
        Some(&stale_plan),
        Generation(9),
        &digest,
        canon_json,
    );
    let stale_reply = handler.handle(stale_req).await;
    assert!(!stale_reply.ok, "stale generation behind Fence must fail");

    // 3. Unknown/bogus VerifyReceipt MUST be rejected (no stub ok: true).
    let bogus_receipt = ControlEnvelope::new(
        ControlOp::VerifyReceipt,
        Some(&plan),
        Generation(10),
        &digest,
        r#"{"resource_path":"/etc/nomos/demo.conf","iteration":0,"outcome":"completed"}"#,
    );
    let receipt_reply = handler.handle(bogus_receipt).await;
    assert!(
        !receipt_reply.ok,
        "unmatched receipt must be rejected by kernel::step"
    );
}
