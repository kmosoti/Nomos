//! Integration tests for Noise_IKpsk2 handshake, Cookie DoS, rekey/expiry, IPV6_V6ONLY, and MeshPort.

use std::net::{Ipv6Addr, SocketAddrV6};

use nomos_core::plan::Generation;
use nomos_mesh::{MeshError, MeshIdentity, MeshPeerSpec, MeshPort, TunnelConfig};
use nomos_mesh_wireguard::{
    CookieGenerator, CoreEngine, CoreOutput, EcnCodepoint, FixedRing, InnerEndpoint,
    REJECT_AFTER_TIME_NS, REKEY_AFTER_TIME_NS, ResponderOutcome, StaticKeypair, UdpShell,
    WireGuardMeshAdapter, consume_cookie_reply, initiate_handshake, respond_handshake,
};

#[test]
fn noise_ikpsk2_handshake_cookie_dos_and_replay_rejection() {
    let init_static = StaticKeypair::generate().unwrap();
    let resp_static = StaticKeypair::generate().unwrap();
    let psk = [0x42u8; 32];
    let peer_ip: Ipv6Addr = "fd00:db8::77".parse().unwrap();
    let mut cookies = CookieGenerator::new(1_000).unwrap();
    let mut last_ts = 0u64;

    // 1. Under DoS load without mac2 -> Type 3 Cookie Reply.
    let (init_hs, type1) =
        initiate_handshake(&init_static, &resp_static.public_bytes(), 1001, 5_000, None).unwrap();
    let outcome = respond_handshake(
        &resp_static,
        &init_static.public_bytes(),
        &psk,
        &type1,
        peer_ip,
        2002,
        1,
        5_000,
        true,
        &mut cookies,
        &mut last_ts,
    )
    .unwrap();
    let ResponderOutcome::CookieRequired(cookie_reply) = outcome else {
        panic!("expected Type 3 CookieRequired when responder is under load");
    };

    // 2. Initiator decrypts cookie, retries Type 1 with mac2 -> Type 2 + matching keys.
    let cookie = consume_cookie_reply(
        &cookie_reply,
        &resp_static.public_bytes(),
        &init_hs.last_mac1(),
    )
    .unwrap();
    let (init_hs2, type1_with_cookie) = initiate_handshake(
        &init_static,
        &resp_static.public_bytes(),
        1002,
        6_000,
        Some(cookie),
    )
    .unwrap();
    let outcome2 = respond_handshake(
        &resp_static,
        &init_static.public_bytes(),
        &psk,
        &type1_with_cookie,
        peer_ip,
        2003,
        1,
        6_000,
        true,
        &mut cookies,
        &mut last_ts,
    )
    .unwrap();
    let ResponderOutcome::Established {
        response,
        keys: resp_keys,
    } = outcome2
    else {
        panic!("expected Established after supplying valid DoS cookie");
    };
    let init_keys = init_hs2.consume_response(&response, &psk, 1).unwrap();

    // Derived transport keys must match symmetrically (init TX == resp RX and vice versa).
    assert_eq!(init_keys.tx_key, resp_keys.rx_key);
    assert_eq!(init_keys.rx_key, resp_keys.tx_key);
    assert_eq!(init_keys.sender_index, 2003);
    assert_eq!(resp_keys.sender_index, 1002);

    // 3. Replaying the same Type 1 timestamp must be rejected.
    let replay_err = respond_handshake(
        &resp_static,
        &init_static.public_bytes(),
        &psk,
        &type1_with_cookie,
        peer_ip,
        2004,
        2,
        6_500,
        false,
        &mut cookies,
        &mut last_ts,
    )
    .unwrap_err();
    assert!(matches!(replay_err, MeshError::ReplayDetected(6_000)));
}

#[test]
fn core_engine_120s_rekey_byte_limit_and_180s_session_expiry() {
    let src = InnerEndpoint {
        addr: "fd00:db8::1".parse().unwrap(),
        port: 51820,
    };
    let dst = InnerEndpoint {
        addr: "fd00:db8::2".parse().unwrap(),
        port: 51820,
    };
    let mut engine = CoreEngine::new(2, 1460, src, dst).unwrap();
    engine.set_rekey_after_bytes(64);
    let mut pool = nomos_mesh_wireguard::BufPool::new(16, 0).unwrap();
    let mut key_updates = FixedRing::with_capacity(8);

    let init_static = StaticKeypair::generate().unwrap();
    let resp_static = StaticKeypair::generate().unwrap();
    let psk = [0x11u8; 32];
    let mut cookies = CookieGenerator::new(0).unwrap();
    let mut last_ts = 0;
    let (hs, t1) =
        initiate_handshake(&init_static, &resp_static.public_bytes(), 11, 100, None).unwrap();
    let ResponderOutcome::Established { response, .. } = respond_handshake(
        &resp_static,
        &init_static.public_bytes(),
        &psk,
        &t1,
        dst.addr,
        22,
        1,
        100,
        false,
        &mut cookies,
        &mut last_ts,
    )
    .unwrap() else {
        panic!("expected established");
    };
    let keys = hs.consume_response(&response, &psk, 1).unwrap();
    engine
        .install_peer_session(0, keys, 1_000, &mut key_updates)
        .unwrap();

    // Crossing byte threshold triggers proactive RekeyRequired.
    let task = engine
        .encapsulate_quic_tx(0, &[0xABu8; 80], src, dst, EcnCodepoint::None, &mut pool)
        .unwrap();
    pool.release(task.handle);
    let mut out = FixedRing::with_capacity(16);
    let mut cipher_out = FixedRing::with_capacity(16);
    engine
        .poll(2_000, &[], &mut pool, &mut out, &mut cipher_out)
        .unwrap();
    assert_eq!(out.pop(), Some(CoreOutput::RekeyRequired { peer_slot: 0 }));

    // Crossing 120s + 180s without rotating retires the expired session.
    engine
        .poll(
            1_000 + REKEY_AFTER_TIME_NS,
            &[],
            &mut pool,
            &mut out,
            &mut cipher_out,
        )
        .unwrap();
    while out.pop().is_some() {}
    while let Some(t) = cipher_out.pop() {
        pool.release(t.handle);
    }

    engine
        .poll(
            1_000 + REJECT_AFTER_TIME_NS + 1,
            &[],
            &mut pool,
            &mut out,
            &mut cipher_out,
        )
        .unwrap();
    let mut retired = false;
    while let Some(item) = out.pop() {
        if item == (CoreOutput::RetirePeer { peer_slot: 0 }) {
            retired = true;
        }
    }
    assert!(retired, "expired session must emit RetirePeer");
    assert!(!engine.peers()[0].established);
}

#[tokio::test]
async fn udp_shell_enforces_ipv6_v6only_and_mesh_adapter_drives_engine() {
    let shell = UdpShell::bind_ipv6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0))
        .await
        .unwrap();
    assert!(shell.is_only_v6(), "UdpShell must enforce IPV6_V6ONLY");

    let adapter = WireGuardMeshAdapter::new(MeshIdentity {
        node_name: "coord-0".to_owned(),
        public_key_hex: "aa".repeat(32),
        mesh_ipv6: "fd00:db8::1/64".to_owned(),
        key_digest_hex: "bb".repeat(32),
    });
    let cfg = TunnelConfig {
        interface: "wg0".to_owned(),
        local_ipv6: "fd00:db8::1/64".to_owned(),
        listen_port: 51820,
        outer_pmtu: 1460,
        accept_ra: 2,
        peers: vec![MeshPeerSpec {
            name: "peer-1".to_owned(),
            public_key_hex: "cc".repeat(32),
            allowed_ips_v6: vec!["fd00:db8::2/128".to_owned()],
            endpoint_v6: Some("[::1]:51820".to_owned()),
            persistent_keepalive_secs: 25,
        }],
    };
    assert_eq!(adapter.configure_tunnel(&cfg).unwrap(), 1380);
    adapter
        .bind_udp_ipv6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0))
        .await
        .unwrap();

    let init_static = StaticKeypair::generate().unwrap();
    let resp_static = StaticKeypair::generate().unwrap();
    let psk = [0x77u8; 32];
    let mut cookies = CookieGenerator::new(0).unwrap();
    let mut ts = 0;
    let (hs, t1) =
        initiate_handshake(&init_static, &resp_static.public_bytes(), 1, 10, None).unwrap();
    let ResponderOutcome::Established { response, .. } = respond_handshake(
        &resp_static,
        &init_static.public_bytes(),
        &psk,
        &t1,
        Ipv6Addr::LOCALHOST,
        2,
        1,
        10,
        false,
        &mut cookies,
        &mut ts,
    )
    .unwrap() else {
        panic!("expected established");
    };
    let keys = hs.consume_response(&response, &psk, 1).unwrap();
    adapter.install_session(0, keys, 5_000).unwrap();

    let wire = adapter
        .encrypt_outbound_payload(
            0,
            b"mesh-control-ping",
            InnerEndpoint {
                addr: "fd00:db8::1".parse().unwrap(),
                port: 51820,
            },
            InnerEndpoint {
                addr: "fd00:db8::2".parse().unwrap(),
                port: 51820,
            },
            EcnCodepoint::None,
        )
        .unwrap();
    assert_eq!(wire.len(), 80 + b"mesh-control-ping".len());

    let posture = adapter.posture(Generation(9)).unwrap();
    assert_eq!(posture.active_peers, 1);
    assert_eq!(posture.peers[0].tx_packets, 1);
    assert_eq!(posture.peers[0].tx_bytes, b"mesh-control-ping".len() as u64);

    // Advance past 180s expiry and verify KeyStore retires the expired session.
    let events = adapter
        .poll_once(5_000 + REJECT_AFTER_TIME_NS + 1, &[])
        .unwrap();
    assert!(events.contains(&CoreOutput::RetirePeer { peer_slot: 0 }));
    assert_eq!(adapter.key_store().load().active_count(), 0);
}
