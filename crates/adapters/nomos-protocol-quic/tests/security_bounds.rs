//! QUIC concurrency bounds and SPKI identity forwarding tests (Finding #12).

use std::future::Future;
use std::net::{Ipv6Addr, SocketAddrV6};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomos_core::plan::Generation;
use nomos_core::resource::Digest;
use nomos_protocol::{ControlEnvelope, ControlOp, ControlReply, TelemetryDatagram};
use nomos_protocol_quic::{ControlHandler, PinnedIdentity, QuicControlNode};

struct RecordingHandler {
    seen_spki: Arc<Mutex<Vec<[u8; 32]>>>,
    seen_dg_spki: Arc<Mutex<Vec<[u8; 32]>>>,
}

impl ControlHandler for RecordingHandler {
    fn handle<'a>(
        &'a self,
        _request: ControlEnvelope,
    ) -> Pin<Box<dyn Future<Output = ControlReply> + Send + 'a>> {
        Box::pin(async {
            ControlReply {
                ok: false,
                generation: 0,
                digest_hex: None,
                message: "unauthenticated".to_owned(),
                payload_json: "{}".to_owned(),
            }
        })
    }

    fn handle_authenticated<'a>(
        &'a self,
        peer_spki_sha256: [u8; 32],
        request: ControlEnvelope,
    ) -> Pin<Box<dyn Future<Output = ControlReply> + Send + 'a>> {
        Box::pin(async move {
            self.seen_spki.lock().unwrap().push(peer_spki_sha256);
            ControlReply {
                ok: true,
                generation: request.generation,
                digest_hex: Some(request.canon_digest_hex),
                message: "ok".to_owned(),
                payload_json: "{}".to_owned(),
            }
        })
    }

    fn handle_telemetry_authenticated(&self, peer_spki_sha256: [u8; 32], _dg: TelemetryDatagram) {
        self.seen_dg_spki.lock().unwrap().push(peer_spki_sha256);
    }
}

#[tokio::test]
async fn quic_forwards_verified_peer_spki_and_caps_concurrent_connections() {
    let server_id = PinnedIdentity::generate(&["localhost", "::1"]).unwrap();
    let server_pin = server_id.spki_fingerprint_hex().to_owned();
    let client_a = PinnedIdentity::generate(&["localhost"]).unwrap();
    let client_b = PinnedIdentity::generate(&["localhost"]).unwrap();
    let pin_a = client_a.spki_sha256_bytes();
    let pin_b = client_b.spki_sha256_bytes();

    let server = QuicControlNode::bind_ipv6(
        SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0),
        server_id,
        vec![pin_a, pin_b],
    )
    .unwrap()
    .with_max_concurrent_connections(1)
    .with_per_peer_limits(2, 2)
    .with_rpc_timeout(Duration::from_secs(2))
    .with_handshake_timeout(Duration::from_millis(500));
    let addr = server.local_addr().unwrap();

    let seen_spki = Arc::new(Mutex::new(Vec::new()));
    let seen_dg_spki = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(RecordingHandler {
        seen_spki: Arc::clone(&seen_spki),
        seen_dg_spki: Arc::clone(&seen_dg_spki),
    });
    let srv_task = tokio::spawn({
        let s = server.clone();
        async move { s.serve(handler, |_| {}).await }
    });

    let node_a =
        QuicControlNode::client_ipv6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0), client_a)
            .unwrap();
    let conn_a = node_a
        .connect_ipv6(addr, "localhost", &server_pin)
        .await
        .unwrap();
    let req = ControlEnvelope::new(
        ControlOp::Heartbeat,
        None,
        Generation(1),
        &Digest::from_bytes([0u8; 32]),
        "{}",
    );
    assert!(conn_a.rpc(&req).await.unwrap().ok);
    conn_a
        .send_datagram(&TelemetryDatagram {
            node: "node-a".to_owned(),
            generation: 1,
            seq: 1,
            cpu_pressure_bps: 10,
            memory_pressure_bps: 20,
            io_pressure_bps: 5,
            active_leases: 0,
        })
        .unwrap();

    // Second simultaneous connection exceeds max_concurrent_connections(1) and MUST be refused.
    let node_b =
        QuicControlNode::client_ipv6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0), client_b)
            .unwrap()
            .with_handshake_timeout(Duration::from_millis(400));
    let rejected = node_b.connect_ipv6(addr, "localhost", &server_pin).await;
    if let Ok(conn_b) = rejected {
        assert!(conn_b.rpc(&req).await.is_err());
    }

    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(seen_spki.lock().unwrap().as_slice(), &[pin_a]);
    assert_eq!(seen_dg_spki.lock().unwrap().as_slice(), &[pin_a]);

    server.close();
    srv_task.abort();
}
