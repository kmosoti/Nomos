//! Pure-IPv6 `[::1]` mock GCP Compute / LRO / Batch / Metadata HTTP server for tests.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub struct MockGcpState {
    pub subnets: Arc<Mutex<BTreeMap<String, Value>>>,
    pub firewalls: Arc<Mutex<BTreeMap<String, Value>>>,
    pub instances: Arc<Mutex<BTreeMap<String, Value>>>,
    pub jobs: Arc<Mutex<BTreeMap<String, Value>>>,
    pub request_ids: Arc<Mutex<Vec<String>>>,
    pub token_fetches: Arc<AtomicUsize>,
    pub lro_polls: Arc<AtomicUsize>,
    pub batch_polls: Arc<AtomicUsize>,
    pub fail_lro: Arc<AtomicBool>,
    pub deleted_jobs: Arc<AtomicUsize>,
    pub fw_patches: Arc<AtomicUsize>,
    pub tag_updates: Arc<AtomicUsize>,
}

#[derive(Deserialize)]
struct ReqIdQuery {
    #[serde(rename = "requestId")]
    request_id: Option<String>,
}

#[derive(Deserialize)]
struct JobQuery {
    #[serde(rename = "jobId")]
    job_id: String,
}

async fn metadata_token(State(st): State<MockGcpState>) -> Json<Value> {
    st.token_fetches.fetch_add(1, Ordering::SeqCst);
    Json(json!({ "access_token": "mock-secret-oauth-token-do-not-leak", "expires_in": 3600 }))
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
    Query(q): Query<ReqIdQuery>,
    Json(body): Json<Value>,
) -> Json<Value> {
    st.request_ids
        .lock()
        .unwrap()
        .push(q.request_id.expect("requestId required on POST subnet"));
    let cidr = body["ipv6CidrRange"]
        .as_str()
        .expect("ipv6CidrRange required")
        .to_owned();
    let name = body["name"].as_str().unwrap_or("sub").to_owned();
    st.subnets
        .lock()
        .unwrap()
        .insert(name, json!({ "ipv6CidrRange": cidr }));
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
    Query(q): Query<ReqIdQuery>,
    Json(body): Json<Value>,
) -> Json<Value> {
    st.request_ids
        .lock()
        .unwrap()
        .push(q.request_id.expect("requestId required on POST firewall"));
    let name = body["name"].as_str().unwrap_or("fw").to_owned();
    st.firewalls.lock().unwrap().insert(name, body);
    Json(json!({
        "name": "op-fw",
        "selfLink": format!("projects/{proj}/global/operations/op-fw"),
        "status": "DONE"
    }))
}

async fn patch_firewall(
    State(st): State<MockGcpState>,
    Path((proj, name)): Path<(String, String)>,
    Query(q): Query<ReqIdQuery>,
    Json(body): Json<Value>,
) -> Json<Value> {
    st.request_ids
        .lock()
        .unwrap()
        .push(q.request_id.expect("requestId required on PATCH firewall"));
    st.fw_patches.fetch_add(1, Ordering::SeqCst);
    st.firewalls.lock().unwrap().insert(name, body);
    Json(json!({
        "name": "op-fw-patch",
        "selfLink": format!("projects/{proj}/global/operations/op-fw-patch"),
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
    Query(q): Query<ReqIdQuery>,
    Json(body): Json<Value>,
) -> Json<Value> {
    st.request_ids
        .lock()
        .unwrap()
        .push(q.request_id.expect("requestId required on POST instance"));
    let name = body["name"].as_str().unwrap_or("vm").to_owned();
    let tags = body["tags"]["items"].clone();
    assert!(tags.is_array(), "CreateInstanceBody must include tags");
    st.instances.lock().unwrap().insert(
        name,
        json!({
            "machineType": format!("zones/{zone}/machineTypes/e2-standard-2"),
            "tags": { "items": tags, "fingerprint": "fp-1" }
        }),
    );
    Json(json!({
        "name": "op-vm",
        "selfLink": format!("projects/{proj}/zones/{zone}/operations/op-vm"),
        "status": "RUNNING"
    }))
}

async fn set_instance_tags(
    State(st): State<MockGcpState>,
    Path((proj, zone, name)): Path<(String, String, String)>,
    Query(q): Query<ReqIdQuery>,
    Json(body): Json<Value>,
) -> Json<Value> {
    st.request_ids
        .lock()
        .unwrap()
        .push(q.request_id.expect("requestId required on setTags"));
    st.tag_updates.fetch_add(1, Ordering::SeqCst);
    let mut map = st.instances.lock().unwrap();
    if let Some(vm) = map.get_mut(&name) {
        vm["tags"] = json!({ "items": body["items"], "fingerprint": "fp-2" });
    }
    Json(json!({
        "name": "op-tags",
        "selfLink": format!("projects/{proj}/zones/{zone}/operations/op-tags"),
        "status": "DONE"
    }))
}

async fn get_lro(State(st): State<MockGcpState>) -> Result<Json<Value>, StatusCode> {
    let count = st.lro_polls.fetch_add(1, Ordering::SeqCst);
    if st.fail_lro.load(Ordering::SeqCst) {
        return Ok(Json(json!({
            "status": "DONE",
            "error": { "errors": [{ "code": "QUOTA_EXCEEDED" }] }
        })));
    }
    if count == 0 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    if count == 1 {
        Ok(Json(json!({ "status": "RUNNING" })))
    } else {
        Ok(Json(json!({ "status": "DONE" })))
    }
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
    let full_name = format!("projects/{proj}/locations/{reg}/jobs/{}", q.job_id);
    st.jobs.lock().unwrap().insert(q.job_id.clone(), body);
    Json(json!({
        "name": full_name,
        "uid": "uid-batch-1",
        "status": { "state": "QUEUED" }
    }))
}

async fn get_batch(
    State(st): State<MockGcpState>,
    Path((proj, reg, job_id)): Path<(String, String, String)>,
) -> Json<Value> {
    let poll = st.batch_polls.fetch_add(1, Ordering::SeqCst);
    let state = if poll == 0 { "RUNNING" } else { "SUCCEEDED" };
    Json(json!({
        "name": format!("projects/{proj}/locations/{reg}/jobs/{job_id}"),
        "uid": "uid-batch-1",
        "status": { "state": state }
    }))
}

async fn delete_batch(
    State(st): State<MockGcpState>,
    Path((_proj, _reg, job_id)): Path<(String, String, String)>,
) -> StatusCode {
    assert!(
        st.batch_polls.load(Ordering::SeqCst) >= 2,
        "scale_to_zero must wait for terminal state before DELETE"
    );
    st.jobs.lock().unwrap().remove(&job_id);
    st.deleted_jobs.fetch_add(1, Ordering::SeqCst);
    StatusCode::OK
}

pub async fn spawn_mock_server(state: MockGcpState) -> SocketAddr {
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
            get(get_firewall).patch(patch_firewall),
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
            "/compute/v1/projects/{proj}/zones/{zone}/instances/{name}/setTags",
            post(set_instance_tags),
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
            get(get_batch).delete(delete_batch),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    addr
}
