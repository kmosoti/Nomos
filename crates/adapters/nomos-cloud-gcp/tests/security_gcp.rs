//! Security tests for GCP identifier grammars, origin-pinned LRO URLs, and repair requestIds (#2, #9).

mod common;

use std::time::Duration;

use common::{MockGcpState, spawn_mock_server};
use nomos_cloud::{
    CloudError, CloudFirewallRule, CloudNodeSpec, CloudSubstrate, SpotBatchConfig, TopologySpec,
};
use nomos_cloud_gcp::batch::{delete_batch_job, submit_spot_batch, wait_batch_job_terminal};
use nomos_cloud_gcp::lro::post_and_wait_lro;
use nomos_cloud_gcp::{GcpCloudAdapter, GcpEndpointConfig, GcpTokenProvider};
use nomos_core::cipher::Secret;

fn valid_topology() -> TopologySpec {
    TopologySpec {
        project_id: "moiric".to_owned(),
        region: "us-central1".to_owned(),
        network_name: "nomos-net".to_owned(),
        subnet_name: "nomos-v6-subnet".to_owned(),
        subnet_ipv6_cidr: "2001:db8:1::/64".to_owned(),
        nodes: vec![CloudNodeSpec {
            name: "cell-1".to_owned(),
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
async fn strict_gcp_grammar_and_url_builders_reject_injection_and_ssrf() {
    let base = valid_topology();
    assert!(base.validate().is_ok());

    for bad_project in [
        "../evil",
        "proj?x=1",
        "proj#frag",
        "ab",
        "BadUpper",
        "a%2fb123",
    ] {
        let mut t = base.clone();
        t.project_id = bad_project.to_owned();
        assert!(
            matches!(t.validate(), Err(CloudError::InvalidSpec(_))),
            "project_id `{bad_project}` must be rejected"
        );
    }

    for bad_seg in ["../x", "name?q=1", "a/b", "name%20x", "BAD-NAME"] {
        let mut t = base.clone();
        t.subnet_name = bad_seg.to_owned();
        assert!(matches!(t.validate(), Err(CloudError::InvalidSpec(_))));

        let mut b = SpotBatchConfig::e2_standard_4(
            "example-project",
            "us-central1",
            bad_seg,
            "ghcr.io/example/worker:v1",
            vec![],
            "projects/example-project/global/networks/nomos-net",
            "projects/example-project/regions/us-central1/subnetworks/nomos-v6-subnet",
        );
        assert!(matches!(b.validate(), Err(CloudError::InvalidSpec(_))));
        b.job_id = "ok-job-1".to_owned();
        b.network_uri = "http://evil.invalid/steal".to_owned();
        assert!(matches!(b.validate(), Err(CloudError::InvalidSpec(_))));
    }

    // Direct Batch URL helpers must also reject raw path/query traversal arguments.
    let client = reqwest::Client::new();
    let cfg = GcpEndpointConfig {
        compute_base_url: "https://compute.googleapis.com/compute/v1".to_owned(),
        batch_base_url: "https://batch.googleapis.com/v1".to_owned(),
        lro_poll_interval: Duration::from_millis(1),
        lro_max_polls: 1,
    };
    let tok = Secret::new("tok".to_owned());
    assert!(
        wait_batch_job_terminal(
            &client,
            &cfg,
            &tok,
            "example-project",
            "us-central1",
            "../evil"
        )
        .await
        .is_err()
    );
    assert!(
        delete_batch_job(
            &client,
            &cfg,
            &tok,
            "example-project",
            "us-central1?x=1",
            "job-1"
        )
        .await
        .is_err()
    );
    assert!(
        submit_spot_batch(
            &client,
            &GcpEndpointConfig {
                batch_base_url: "http://remote.attacker.example/v1".to_owned(),
                ..cfg.clone()
            },
            &tok,
            &SpotBatchConfig::e2_standard_4(
                "example-project",
                "us-central1",
                "job-1",
                "ghcr.io/example/worker:v1",
                vec![],
                "projects/example-project/global/networks/nomos-net",
                "projects/example-project/regions/us-central1/subnetworks/nomos-v6-subnet",
            ),
        )
        .await
        .is_err()
    );

    // Off-origin LRO selfLink must never be followed with bearer auth.
    let rogue_lro_state = spawn_rogue_lro_mock().await;
    let rogue_cfg = GcpEndpointConfig {
        compute_base_url: rogue_lro_state.clone(),
        batch_base_url: rogue_lro_state.clone(),
        lro_poll_interval: Duration::from_millis(1),
        lro_max_polls: 2,
    };
    let err = post_and_wait_lro(
        &client,
        &rogue_cfg,
        &tok,
        &format!("{rogue_lro_state}/projects/moiric/global/firewalls"),
        "00000000-0000-5000-8000-000000000001",
        &serde_json::json!({}),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CloudError::InvalidSpec(_)));
}

#[tokio::test]
async fn repeated_repair_of_same_spec_after_deletion_uses_fresh_request_id() {
    let state = MockGcpState::default();
    let addr = spawn_mock_server(state.clone()).await;
    let base = format!("http://[::1]:{}", addr.port());

    let auth = GcpTokenProvider::from_metadata_endpoint(format!("{base}/metadata/token"), None);
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

    let spec = valid_topology();
    adapter.apply_topology(&spec, true).await.unwrap();
    let count_after_first = state.request_ids.lock().unwrap().len();
    assert_eq!(count_after_first, 3);

    // Simulate out-of-band deletion of firewall rule without changing TopologySpec.
    state.firewalls.lock().unwrap().clear();
    let repair_plan = adapter.apply_topology(&spec, true).await.unwrap();
    assert_eq!(repair_plan.pending_changes(), 1);

    let ids = state.request_ids.lock().unwrap().clone();
    assert_eq!(ids.len(), 4);
    assert_ne!(
        ids[1], ids[3],
        "repair requestId must differ from initial creation requestId even for identical spec"
    );
}

async fn spawn_rogue_lro_mock() -> String {
    use axum::{Json, Router, routing::post};
    let app = Router::new().route(
        "/projects/moiric/global/firewalls",
        post(|| async {
            Json(serde_json::json!({
                "name": "op-rogue",
                "selfLink": "https://evil.example.invalid/steal-oauth-token",
                "status": "RUNNING"
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://[::1]:{}", addr.port())
}
