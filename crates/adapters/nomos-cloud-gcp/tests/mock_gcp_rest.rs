//! Integration tests for `nomos-cloud-gcp` against a pure-IPv6 `[::1]` mock REST/LRO/Batch server.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use nomos_cloud::{
    CloudError, CloudFirewallRule, CloudNodeSpec, CloudSubstrate, DriftStatus, SpotBatchConfig,
    TopologySpec,
};
use nomos_cloud_gcp::{GcpCloudAdapter, GcpEndpointConfig, GcpTokenProvider};
use nomos_core::plan::Generation;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct MockGcpState {
    subnets: Arc<Mutex<BTreeMap<String, Value>>>,
    firewalls: Arc<Mutex<BTreeMap<String, Value>>>,
    instances: Arc<Mutex<BTreeMap<String, Value>>>,
    jobs: Arc<Mutex<BTreeMap<String, Value>>>,
    lro_polls: Arc<AtomicUsize>,
    fail_lro: Arc<AtomicBool>,
    deleted_jobs: Arc<AtomicUsize>,
}

async fn metadata_token() -> Json<Value> {
    Json(json!({ "access_token": "mock-secret-oauth-token-do-not-leak" }))
}

async fn get_subnet(
    State(st): State<MockGcpState>,
    Path((_proj, _reg, name)): Path<(String, String, String)>,
) -> Result<Json<Value>, StatusCode> {
    st.subnets
        .lock()
        .unwrap()
        .get(&name)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create_subnet(
    State(st): State<MockGcpState>,
    Path((proj, reg)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let name = body["name"].as_str().unwrap_or("sub").to_owned();
    st.subnets
        .lock()
        .unwrap()
        .insert(name, json!({ "ipv6CidrRange": "2001:db8:1::/64" }));
    Json(json!({
        "name": "op-subnet",
        "selfLink": format!("projects/{proj}/regions/{reg}/operations/op-subnet"),
        "status": "RUNNING"
    }))
}

async fn get_firewall(
    State(st): State<MockGcpState>,
    Path((_proj, name)): Path<(String, String)>,
) -> Result<Json<Value>, StatusCode> {
    st.firewalls
        .lock()
        .unwrap()
        .get(&name)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create_firewall(
    State(st): State<MockGcpState>,
    Path(proj): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let name = body["name"].as_str().unwrap_or("fw").to_owned();
    st.firewalls.lock().unwrap().insert(name, body);
    Json(json!({
        "name": "op-fw",
        "selfLink": format!("projects/{proj}/global/operations/op-fw"),
        "status": "DONE"
    }))
}

async fn get_instance(
    State(st): State<MockGcpState>,
    Path((_proj, _zone, name)): Path<(String, String, String)>,
) -> Result<Json<Value>, StatusCode> {
    st.instances
        .lock()
        .unwrap()
        .get(&name)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create_instance(
    State(st): State<MockGcpState>,
    Path((proj, zone)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let name = body["name"].as_str().unwrap_or("vm").to_owned();
    st.instances.lock().unwrap().insert(
        name,
        json!({ "machineType": format!("zones/{zone}/machineTypes/e2-standard-2") }),
    );
    Json(json!({
        "name": "op-vm",
        "selfLink": format!("projects/{proj}/zones/{zone}/operations/op-vm"),
        "status": "RUNNING"
    }))
}

async fn get_lro(State(st): State<MockGcpState>) -> Json<Value> {
    let count = st.lro_polls.fetch_add(1, Ordering::SeqCst);
    if st.fail_lro.load(Ordering::SeqCst) {
        return Json(json!({
            "status": "DONE",
            "error": { "errors": [{ "code": "QUOTA_EXCEEDED" }] }
        }));
    }
    if count == 0 {
        Json(json!({ "status": "RUNNING" }))
    } else {
        Json(json!({ "status": "DONE" }))
    }
}

#[derive(Deserialize)]
struct JobQuery {
    #[serde(rename = "jobId")]
    job_id: String,
}

async fn submit_batch(
    State(st): State<MockGcpState>,
    Path((proj, reg)): Path<(String, String)>,
    Query(q): Query<JobQuery>,
    Json(body): Json<Value>,
) -> Json<Value> {
    assert_eq!(
        body["allocationPolicy"]["instances"][0]["policy"]["provisioningModel"],
        "SPOT"
    );
    assert_eq!(
        body["allocationPolicy"]["instances"][0]["policy"]["machineType"],
        "e2-standard-4"
    );
    let full_name = format!("projects/{proj}/locations/{reg}/jobs/{}", q.job_id);
    st.jobs.lock().unwrap().insert(q.job_id.clone(), body);
    Json(json!({
        "name": full_name,
        "uid": "uid-batch-1",
        "status": { "state": "SUCCEEDED" }
    }))
}

async fn delete_batch(
    State(st): State<MockGcpState>,
    Path((_proj, _reg, job_id)): Path<(String, String, String)>,
) -> StatusCode {
    st.jobs.lock().unwrap().remove(&job_id);
    st.deleted_jobs.fetch_add(1, Ordering::SeqCst);
    StatusCode::OK
}

async fn spawn_mock_server(state: MockGcpState) -> SocketAddr {
    let router = Router::new()
        .route("/metadata/token", get(metadata_token))
        .route(
            "/compute/v1/projects/{proj}/regions/{reg}/subnetworks",
            post(create_subnet),
        )
        .route(
            "/compute/v1/projects/{proj}/regions/{reg}/subnetworks/{name}",
            get(get_subnet),
        )
        .route(
            "/compute/v1/projects/{proj}/regions/{reg}/operations/{op}",
            get(get_lro),
        )
        .route(
            "/compute/v1/projects/{proj}/global/firewalls",
            post(create_firewall),
        )
        .route(
            "/compute/v1/projects/{proj}/global/firewalls/{name}",
            get(get_firewall),
        )
        .route(
            "/compute/v1/projects/{proj}/global/operations/{op}",
            get(get_lro),
        )
        .route(
            "/compute/v1/projects/{proj}/zones/{zone}/instances",
            post(create_instance),
        )
        .route(
            "/compute/v1/projects/{proj}/zones/{zone}/instances/{name}",
            get(get_instance),
        )
        .route(
            "/compute/v1/projects/{proj}/zones/{zone}/operations/{op}",
            get(get_lro),
        )
        .route(
            "/batch/v1/projects/{proj}/locations/{reg}/jobs",
            post(submit_batch),
        )
        .route(
            "/batch/v1/projects/{proj}/locations/{reg}/jobs/{job_id}",
            delete(delete_batch),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    addr
}

fn sample_topology() -> TopologySpec {
    TopologySpec {
        project_id: "example-project".to_owned(),
        region: "us-central1".to_owned(),
        network_name: "nomos-net".to_owned(),
        subnet_name: "nomos-v6-subnet".to_owned(),
        subnet_ipv6_cidr: "2001:db8:1::/64".to_owned(),
        nodes: vec![CloudNodeSpec {
            name: "example-cell-1".to_owned(),
            zone: "us-central1-a".to_owned(),
            machine_type: "e2-standard-2".to_owned(),
            ipv6_cidr: "2001:db8:1::10/96".to_owned(),
            tags: vec!["nomos-mesh".to_owned()],
            source_image: "projects/debian-cloud/global/images/family/debian-12".to_owned(),
        }],
        firewall_rules: vec![CloudFirewallRule {
            name: "allow-wg-ipv6".to_owned(),
            direction: "INGRESS".to_owned(),
            source_ranges_v6: vec!["2001:db8::/32".to_owned()],
            protocol: "udp".to_owned(),
            ports: vec!["51820".to_owned()],
        }],
    }
}

#[tokio::test]
async fn pure_ipv6_gcp_topology_lro_spot_batch_and_token_redaction() {
    let state = MockGcpState::default();
    let addr = spawn_mock_server(state.clone()).await;
    let base = format!("http://[::1]:{}", addr.port());

    let auth = GcpTokenProvider::from_metadata_endpoint(
        format!("{base}/metadata/token"),
        Some("nomos-deployer@example-project.iam.gserviceaccount.com".to_owned()),
    );
    let adapter = GcpCloudAdapter::with_endpoints(
        auth,
        GcpEndpointConfig {
            compute_base_url: format!("{base}/compute/v1"),
            batch_base_url: format!("{base}/batch/v1"),
            lro_poll_interval: Duration::from_millis(5),
            lro_max_polls: 10,
        },
    )
    .unwrap();

    let posture = adapter.auth_posture(Generation(3)).await.unwrap();
    assert!(posture.authenticated);
    assert!(
        !serde_json::to_string(&posture)
            .unwrap()
            .contains("mock-secret-oauth-token")
    );

    let spec = sample_topology();
    let dry = adapter.plan_topology(&spec).await.unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.pending_changes(), 3);

    let applied = adapter.apply_topology(&spec, true).await.unwrap();
    assert!(!applied.dry_run);
    assert_eq!(applied.pending_changes(), 3);
    assert!(state.lro_polls.load(Ordering::SeqCst) >= 2);

    // Second plan converges with 0 pending diffs.
    let clean = adapter.plan_topology(&spec).await.unwrap();
    assert_eq!(clean.pending_changes(), 0);
    assert!(clean.items.iter().all(|i| i.status == DriftStatus::Match));

    // Spot Batch e2-standard-4 scale-to-zero executes and deletes finished job.
    let job = SpotBatchConfig::e2_standard_4(
        "example-project",
        "us-central1",
        "spot-job-1",
        "ghcr.io/example/worker:v1",
        vec!["--check".to_owned()],
        "projects/example-project/global/networks/nomos-net",
        "projects/example-project/regions/us-central1/subnetworks/nomos-v6-subnet",
    );
    let handle = adapter.submit_spot_batch(&job).await.unwrap();
    assert_eq!(handle.state, "SUCCEEDED");
    assert_eq!(state.deleted_jobs.load(Ordering::SeqCst), 1);

    // Negative control: LRO error fails closed.
    state.fail_lro.store(true, Ordering::SeqCst);
    let mut spec2 = spec;
    spec2.nodes[0].name = "example-cell-2".to_owned();
    let err = adapter.apply_topology(&spec2, true).await.unwrap_err();
    assert!(matches!(err, CloudError::OperationFailed(_)));
}
