//! Integration tests for `nomos-cloud-gcp` against a pure-IPv6 `[::1]` mock REST/PATCH/LRO/Batch server.

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{MockGcpState, spawn_mock_server};
use nomos_cloud::{
    CloudError, CloudFirewallRule, CloudNodeSpec, CloudSubstrate, DriftStatus, SpotBatchConfig,
    TopologySpec,
};
use nomos_cloud_gcp::{GcpCloudAdapter, GcpEndpointConfig, GcpTokenProvider};
use nomos_core::plan::Generation;

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

#[test]
fn blake3_spec_digest_covers_every_node_and_firewall_field() {
    let base = sample_topology();
    let base_digest = base.spec_digest();

    let mut t1 = base.clone();
    t1.nodes[0].tags.push("extra-tag".to_owned());
    assert_ne!(t1.spec_digest(), base_digest);

    let mut t2 = base.clone();
    t2.nodes[0].source_image = "projects/debian-cloud/global/images/family/debian-13".to_owned();
    assert_ne!(t2.spec_digest(), base_digest);

    let mut t3 = base.clone();
    t3.firewall_rules[0].direction = "EGRESS".to_owned();
    assert_ne!(t3.spec_digest(), base_digest);

    let mut t4 = base.clone();
    t4.firewall_rules[0].ports.push("7443".to_owned());
    assert_ne!(t4.spec_digest(), base_digest);
}

#[tokio::test]
async fn pure_ipv6_gcp_patch_reconcile_cached_oauth_lro_retry_and_batch_polling() {
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

    let mut spec = sample_topology();
    let applied = adapter.apply_topology(&spec, true).await.unwrap();
    assert_eq!(applied.pending_changes(), 3);
    assert!(state.lro_polls.load(Ordering::SeqCst) >= 3);

    // Token cache must reuse the single cached token across posture + plan + apply!
    assert_eq!(state.token_fetches.load(Ordering::SeqCst), 1);

    // Mutate firewall port + instance tags -> PATCH & setTags reconcile DriftStatus::Update.
    spec.firewall_rules[0].ports.push("7443".to_owned());
    spec.nodes[0].tags.push("nomos-loom".to_owned());
    let drift = adapter.plan_topology(&spec).await.unwrap();
    assert_eq!(drift.pending_changes(), 2);
    assert_eq!(
        drift
            .items
            .iter()
            .filter(|i| i.status == DriftStatus::Update)
            .count(),
        2
    );

    adapter.apply_topology(&spec, true).await.unwrap();
    assert_eq!(state.fw_patches.load(Ordering::SeqCst), 1);
    assert_eq!(state.tag_updates.load(Ordering::SeqCst), 1);
    assert_eq!(
        adapter
            .plan_topology(&spec)
            .await
            .unwrap()
            .pending_changes(),
        0
    );

    // Every GCE mutation must carry a UUIDv5 requestId (version nibble '5').
    let ids = state.request_ids.lock().unwrap().clone();
    assert!(ids.len() >= 5);
    assert!(
        ids.iter()
            .all(|id| id.len() == 36 && id.as_bytes()[14] == b'5')
    );

    // Spot Batch polls QUEUED -> RUNNING -> SUCCEEDED before scale_to_zero DELETE.
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
    assert_eq!(state.token_fetches.load(Ordering::SeqCst), 1);

    // Negative control: LRO error fails closed.
    state.fail_lro.store(true, Ordering::SeqCst);
    spec.nodes[0].name = "example-cell-2".to_owned();
    let err = adapter.apply_topology(&spec, true).await.unwrap_err();
    assert!(matches!(err, CloudError::OperationFailed(_)));
}
