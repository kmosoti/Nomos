//! Security regression suite covering Daybreak QUIC & CLI findings (#1, #6, #11, #13).

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nomos_canon::artifact::{Profile, canon_id, encode};
use nomos_canon::model::{Canon, RawCanon, RawResource, RawSpec};
use nomos_core::governor::GovernorConfig;
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::Digest;
use nomos_loom::cli::{LoomCommand, ServeArgs, parse_args, validate_serve_args};
use nomos_loom::governor_svc::GovernorService;
use nomos_loom::kernel_svc::{KernelService, MAX_GENERATION_LEAP};
use nomos_loom::lease::LeaseBroker;
use nomos_loom::peer_policy::{PeerRole, parse_client_pin_spec};
use nomos_loom::quic_svc::LoomQuicHandler;
use nomos_protocol::{ControlEnvelope, ControlOp, TelemetryDatagram};
use nomos_protocol_quic::ControlHandler;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "nomos-loom-qsec-{label}-{}-{nanos}",
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

fn canon_payload(path: &str) -> (String, Digest) {
    let raw = RawCanon {
        name: "loom-sec".to_owned(),
        resources: vec![RawResource {
            path: path.to_owned(),
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
    (
        String::from_utf8(bytes).unwrap(),
        Digest::from_bytes(*cid.as_bytes()),
    )
}

#[test]
fn non_loopback_http_bind_requires_tls_proxy_flag_and_pin_roles_default_least_privilege() {
    // #6: non-loopback HTTP bind fails without --behind-tls-proxy.
    let bad_args = ServeArgs {
        listen: "[::]:7440".parse().unwrap(),
        ..Default::default()
    };
    assert!(validate_serve_args(&bad_args).is_err());

    let cmd = parse_args([
        "serve".to_owned(),
        "--listen".to_owned(),
        "[::]:7440".to_owned(),
        "--behind-tls-proxy".to_owned(),
        "--authority-pin".to_owned(),
        "aa".repeat(32),
        "--cell-pin".to_owned(),
        format!("cell-1:{}", "bb".repeat(32)),
    ])
    .unwrap();
    let LoomCommand::Serve(ok_args) = cmd else {
        panic!("expected Serve command");
    };
    assert!(ok_args.behind_tls_proxy);
    assert!(validate_serve_args(&ok_args).is_ok());

    // #1: bare --allowed-client-pin parses as Cell role, never Authority.
    let bare = parse_client_pin_spec(&"cc".repeat(32)).unwrap();
    assert_eq!(bare.role, PeerRole::Cell);
    let explicit = parse_client_pin_spec(&format!("authority:{}", "aa".repeat(32))).unwrap();
    assert_eq!(explicit.role, PeerRole::Authority);
}

#[tokio::test]
async fn quic_role_separation_replay_leap_guard_and_telemetry_isolation() {
    let tmp = TempDir::new("quic-auth");
    let kernel = KernelService::open(&tmp.0.join("state")).unwrap();
    let leases = LeaseBroker::open(tmp.0.join("leases.sqlite"), 4).unwrap();
    let governor = GovernorService::new(GovernorConfig::default());

    let auth_pin = [0x11u8; 32];
    let cell_pin = [0x22u8; 32];
    let cell_b_pin = [0x33u8; 32];
    let handler = LoomQuicHandler::new("coord-sec".to_owned(), governor.clone(), leases)
        .with_kernel(kernel.clone())
        .with_authority_pin(auth_pin)
        .with_cell_pin(Some("cell-a".to_owned()), cell_pin)
        .with_cell_pin(None, cell_b_pin);

    let (canon_a, dig_a) = canon_payload("/etc/nomos/a.conf");
    let (canon_b, dig_b) = canon_payload("/etc/nomos/b.conf");
    let plan = PlanId::new("plan-1").unwrap();

    // #1: Cell pin MUST be denied SubmitPlan, while Status succeeds.
    let submit_from_cell = ControlEnvelope::new(
        ControlOp::SubmitPlan,
        Some(&plan),
        Generation(1),
        &dig_a,
        canon_a.clone(),
    );
    let cell_reply = handler
        .handle_authenticated(cell_pin, submit_from_cell.clone())
        .await;
    assert!(!cell_reply.ok);
    assert_eq!(kernel.journal_len(), 0);

    let hb_from_cell = ControlEnvelope::new(
        ControlOp::Status,
        None,
        Generation(99_999),
        &Digest::from_bytes([0u8; 32]),
        "{}",
    );
    let status_reply = handler.handle_authenticated(cell_pin, hb_from_cell).await;
    assert!(status_reply.ok);
    assert_eq!(
        status_reply.generation,
        governor.generation(),
        "server generation must not reflect client request.generation"
    );

    // #11: Authority u64::MAX / extreme generation leap MUST be rejected without poisoning Fence.
    let poison_req = ControlEnvelope::new(
        ControlOp::SubmitPlan,
        Some(&plan),
        Generation(u64::MAX),
        &dig_a,
        canon_a.clone(),
    );
    assert!(!handler.handle_authenticated(auth_pin, poison_req).await.ok);
    let leap_req = ControlEnvelope::new(
        ControlOp::SubmitPlan,
        Some(&plan),
        Generation(MAX_GENERATION_LEAP + 10),
        &dig_a,
        canon_a.clone(),
    );
    assert!(!handler.handle_authenticated(auth_pin, leap_req).await.ok);
    assert_eq!(kernel.current_generation(), 0);
    assert_eq!(kernel.journal_len(), 0);

    // Valid Authority SubmitPlan succeeds at gen 1, then replay & divergent redelivery fail.
    let valid_req = ControlEnvelope::new(
        ControlOp::SubmitPlan,
        Some(&plan),
        Generation(1),
        &dig_a,
        canon_a,
    );
    assert!(
        handler
            .handle_authenticated(auth_pin, valid_req.clone())
            .await
            .ok
    );
    assert_eq!(kernel.journal_len(), 1);
    assert!(!handler.handle_authenticated(auth_pin, valid_req).await.ok);
    let divergent_req = ControlEnvelope::new(
        ControlOp::SubmitPlan,
        Some(&plan),
        Generation(1),
        &dig_b,
        canon_b,
    );
    assert!(
        !handler
            .handle_authenticated(auth_pin, divergent_req)
            .await
            .ok
    );
    assert_eq!(kernel.journal_len(), 1);

    // #13: Telemetry binds node_id to peer SPKI, rejects replay/overflow, isolates local governor.
    let bad_node_dg = TelemetryDatagram {
        node: "spoofed-other-node".to_owned(),
        generation: 1,
        seq: 1,
        cpu_pressure_bps: 100,
        memory_pressure_bps: 100,
        io_pressure_bps: 100,
        active_leases: 0,
    };
    handler.handle_telemetry_authenticated(cell_pin, bad_node_dg);
    assert!(handler.remote_telemetry_snapshot().is_empty());

    let overload_dg = TelemetryDatagram {
        node: "cell-a".to_owned(),
        generation: 1,
        seq: 1,
        cpu_pressure_bps: 15_000,
        memory_pressure_bps: 9_999,
        io_pressure_bps: 100,
        active_leases: 0,
    };
    handler.handle_telemetry_authenticated(cell_pin, overload_dg);
    assert!(handler.remote_telemetry_snapshot().is_empty());

    let ok_dg = TelemetryDatagram {
        node: "cell-a".to_owned(),
        generation: 1,
        seq: 2,
        cpu_pressure_bps: 9_500,
        memory_pressure_bps: 9_500,
        io_pressure_bps: 100,
        active_leases: 1,
    };
    handler.handle_telemetry_authenticated(cell_pin, ok_dg.clone());
    assert_eq!(handler.remote_telemetry_snapshot().len(), 1);
    assert!(governor.status().admit_batch);
    handler.handle_telemetry_authenticated(cell_pin, ok_dg.clone());
    handler.handle_telemetry_authenticated(cell_b_pin, ok_dg);
    assert_eq!(handler.remote_telemetry_snapshot().len(), 1);
}
