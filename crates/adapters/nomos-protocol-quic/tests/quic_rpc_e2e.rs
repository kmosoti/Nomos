//! Integration tests for Ed25519 SHA-256 pinned QUIC RPC streams and RFC 9221 datagrams.

use std::future::Future;
use std::net::{Ipv6Addr, SocketAddrV6};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

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
async fn pinned_ipv6_quic_rpc_and_datagram_roundtrip_and_negative_pin_rejection() {
    let server_id = PinnedIdentity::generate(&["localhost", "::1"]).unwrap();
    let server_pin = server_id.fingerprint_hex().to_owned();

    let server_bind = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0);
    let server = QuicControlNode::bind_ipv6(server_bind, server_id, None).unwrap();
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

    let client_id = PinnedIdentity::generate(&["localhost"]).unwrap();
    let client_bind = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0);
    let client = QuicControlNode::bind_ipv6(client_bind, client_id, None).unwrap();

    // Negative control: mismatched SHA-256 fingerprint must reject connection.
    let bad_pin = "0".repeat(64);
    let err = client
        .connect_ipv6(server_addr, "localhost", &bad_pin)
        .await
        .unwrap_err();
    assert!(matches!(err, ProtocolError::FingerprintMismatch { .. }));

    // Positive: matching SHA-256 pin succeeds for RPC + unreliable datagrams.
    let conn = client
        .connect_ipv6(server_addr, "localhost", &server_pin)
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
    let reply = conn.rpc(&req).await.unwrap();
    assert!(reply.ok);
    assert_eq!(reply.generation, 42);
    assert_eq!(
        reply.digest_hex.as_deref(),
        Some(req.canon_digest_hex.as_str())
    );

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

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let dgrams = received_dgrams.lock().unwrap();
    assert_eq!(dgrams.len(), 1);
    assert_eq!(dgrams[0], dgram);
    assert!(client.stats().rpc_calls >= 1);

    server.close();
    srv_handle.abort();
}
