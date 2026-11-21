//! Integration tests for mandatory-mTLS Ed25519 SPKI SHA-256 pinned QUIC RPC (JSON & CBOR).

use std::future::Future;
use std::net::{Ipv6Addr, SocketAddrV6};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::Digest;
use nomos_protocol::{
    ControlEnvelope, ControlOp, ControlReply, ControlTransportMetrics, ProtocolError,
    TelemetryDatagram,
};
use nomos_protocol_quic::{ControlHandler, PinnedIdentity, QuicControlNode};

struct EchoHandler;

impl ControlHandler for EchoHandler {
    fn handle<'a>(
        &'a self,
        request: ControlEnvelope,
    ) -> Pin<Box<dyn Future<Output = ControlReply> + Send + 'a>> {
        Box::pin(async move {
            ControlReply {
                ok: true,
                generation: request.generation + 1,
                digest_hex: Some(request.canon_digest_hex.clone()),
                message: format!("accepted:{:?}", request.op),
                payload_json: request.payload_json,
            }
        })
    }
}

#[tokio::test]
async fn mandatory_mtls_spki_pinned_ipv6_quic_json_and_cbor_roundtrip() {
    let server_id = PinnedIdentity::generate(&["localhost", "::1"]).unwrap();
    let server_spki_pin = server_id.spki_fingerprint_hex().to_owned();
    let client_id = PinnedIdentity::generate(&["localhost"]).unwrap();
    let client_spki_bytes = client_id.spki_sha256_bytes();

    let server_bind = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0);

    // Mandatory mTLS: empty allowed_client_pins MUST fail at bind time.
    let empty_err = QuicControlNode::bind_ipv6(server_bind, server_id.clone(), Vec::new());
    assert!(matches!(empty_err, Err(ProtocolError::Certificate(_))));

    // Server pins client by constant-time SPKI SHA-256.
    let server = QuicControlNode::bind_ipv6(server_bind, server_id, vec![client_spki_bytes])
        .unwrap()
        .with_max_concurrent_streams(16)
        .with_rpc_timeout(Duration::from_secs(5));
    let server_addr = server.local_addr().unwrap();

    let received_dgrams = Arc::new(Mutex::new(Vec::<TelemetryDatagram>::new()));
    let dgrams_sink = Arc::clone(&received_dgrams);
    let srv_handle = tokio::spawn({
        let srv = server.clone();
        async move {
            srv.serve(Arc::new(EchoHandler), move |dgram| {
                dgrams_sink.lock().unwrap().push(dgram);
            })
            .await;
        }
    });

    let client_bind = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0);
    let client = QuicControlNode::client_ipv6(client_bind, client_id)
        .unwrap()
        .with_rpc_timeout(Duration::from_secs(5));

    // Negative control 1: mismatched server SPKI SHA-256 pin rejected.
    let bad_pin = "0".repeat(64);
    let err = client
        .connect_ipv6(server_addr, "localhost", &bad_pin)
        .await
        .unwrap_err();
    assert!(matches!(err, ProtocolError::FingerprintMismatch { .. }));

    // Negative control 2: unpinned rogue client identity rejected by mandatory mTLS server.
    let rogue_id = PinnedIdentity::generate(&["localhost"]).unwrap();
    let rogue = QuicControlNode::client_ipv6(client_bind, rogue_id).unwrap();
    let rogue_conn = rogue
        .connect_ipv6(server_addr, "localhost", &server_spki_pin)
        .await;
    if let Ok(c) = rogue_conn {
        let plan = PlanId::new("plan-rogue").unwrap();
        let req = ControlEnvelope::new(
            ControlOp::Heartbeat,
            Some(&plan),
            Generation(1),
            &Digest::from_bytes([0x01; 32]),
            "{}",
        );
        assert!(c.rpc(&req).await.is_err());
    }

    // Positive: matching SPKI SHA-256 mTLS succeeds for JSON + canonical CBOR RPC & datagrams.
    let conn = client
        .connect_ipv6(server_addr, "localhost", &server_spki_pin)
        .await
        .unwrap();

    let plan = PlanId::new("plan-alpha-1").unwrap();
    let req = ControlEnvelope::new(
        ControlOp::SubmitPlan,
        Some(&plan),
        Generation(41),
        &Digest::from_bytes([0xABu8; 32]),
        r#"{"unit":"nomos-cell.service"}"#,
    );

    let reply_json = conn.rpc(&req).await.unwrap();
    assert!(reply_json.ok);
    assert_eq!(reply_json.generation, 42);

    let reply_cbor = conn.rpc_cbor(&req).await.unwrap();
    assert!(reply_cbor.ok);
    assert_eq!(reply_cbor.generation, 42);
    assert_eq!(reply_cbor.digest_hex, reply_json.digest_hex);

    let dgram = TelemetryDatagram {
        node: "example-node".to_owned(),
        generation: 42,
        seq: 7,
        cpu_pressure_bps: 250,
        memory_pressure_bps: 120,
        io_pressure_bps: 40,
        active_leases: 2,
    };
    conn.send_datagram(&dgram).unwrap();
    conn.send_datagram_cbor(&dgram).unwrap();

    tokio::time::sleep(Duration::from_millis(60)).await;
    let dgrams = received_dgrams.lock().unwrap();
    assert_eq!(dgrams.len(), 2);
    assert_eq!(dgrams[0], dgram);
    assert_eq!(dgrams[1], dgram);
    assert!(client.stats().rpc_calls >= 2);

    server.close();
    srv_handle.abort();
}
