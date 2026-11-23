//! Regression tests for inner IPv6 destination + port authorization and zero-alloc poll (#14).

use std::alloc::System;
use std::net::Ipv6Addr;

use nomos_mesh::{MeshIdentity, MeshPeerSpec, MeshPort, TunnelConfig};
use nomos_mesh_wireguard::{
    BufPool, CoreEngine, CoreOutput, DecryptedRx, EcnCodepoint, FixedRing, InnerEndpoint,
    SessionKeys, WireGuardMeshAdapter,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[test]
fn core_engine_enforces_inner_destination_ip_port_and_rejects_port_zero_or_multicast() {
    let src = InnerEndpoint {
        addr: "fd00:db8::10".parse::<Ipv6Addr>().unwrap(),
        port: 4000,
    };
    let dst_ok = InnerEndpoint {
        addr: "fd00:db8::1".parse::<Ipv6Addr>().unwrap(),
        port: 7443,
    };
    let dst_bad_port = InnerEndpoint {
        addr: dst_ok.addr,
        port: 9000,
    };
    let dst_bad_ip = InnerEndpoint {
        addr: "fd00:dead::99".parse::<Ipv6Addr>().unwrap(),
        port: 7443,
    };
    let dst_multicast = InnerEndpoint {
        addr: "ff02::1".parse::<Ipv6Addr>().unwrap(),
        port: 7443,
    };
    let dst_port_zero = InnerEndpoint {
        addr: dst_ok.addr,
        port: 0,
    };

    let mut engine = CoreEngine::new(1, 1460, dst_ok, src).unwrap();
    engine.set_peer_allowed_prefix(0, src.addr, 128).unwrap();
    engine
        .set_peer_allowed_destination(0, dst_ok.addr, 128, Some((7440, 7450)))
        .unwrap();

    let mut pool = BufPool::new(16, 0).unwrap();
    let mut updates = FixedRing::with_capacity(8);
    engine
        .install_peer_session(
            0,
            SessionKeys::new(1, 2, [0xAA; 32], [0xBB; 32], 1),
            100,
            &mut updates,
        )
        .unwrap();

    let mut out = FixedRing::with_capacity(8);
    let mut cipher_out = FixedRing::with_capacity(8);

    // 1. Authorized packet must deliver with ZERO heap allocations inside poll.
    let ok_tx = engine
        .encapsulate_quic_tx(0, b"valid", src, dst_ok, EcnCodepoint::None, &mut pool)
        .unwrap();
    let rx_ok = [DecryptedRx {
        handle: ok_tx.handle,
        peer_slot: 0,
        counter: 0,
        wire_len: ok_tx.wire_len,
        ecn: EcnCodepoint::None,
    }];
    let region = Region::new(GLOBAL);
    engine
        .poll(200, &rx_ok, &mut pool, &mut out, &mut cipher_out)
        .unwrap();
    let stats = region.change();
    assert_eq!(
        stats.allocations, 0,
        "CoreEngine::poll must stay zero-alloc"
    );
    let item = out.pop().unwrap();
    assert!(matches!(item, CoreOutput::DeliverInner { .. }));
    pool.release(ok_tx.handle);

    // 2. Unauthorized destination IP, wrong port, port 0, and multicast MUST be dropped.
    for (ctr, bad_dst) in [dst_bad_ip, dst_bad_port, dst_port_zero, dst_multicast]
        .into_iter()
        .enumerate()
    {
        let bad_tx = engine
            .encapsulate_quic_tx(0, b"bad", src, bad_dst, EcnCodepoint::None, &mut pool)
            .unwrap();
        let rx_bad = [DecryptedRx {
            handle: bad_tx.handle,
            peer_slot: 0,
            counter: (ctr + 1) as u64,
            wire_len: bad_tx.wire_len,
            ecn: EcnCodepoint::None,
        }];
        engine
            .poll(
                300 + ctr as u64,
                &rx_bad,
                &mut pool,
                &mut out,
                &mut cipher_out,
            )
            .unwrap();
        assert!(matches!(out.pop(), Some(CoreOutput::RecycleHandle { .. })));
        pool.release(bad_tx.handle);
    }
    assert_eq!(engine.dropped_invalid, 4);

    // Verify invalid port range returns Config Error cleanly.
    assert!(
        engine
            .set_peer_allowed_destination(0, dst_ok.addr, 128, Some((8000, 7000)))
            .is_err()
    );

    // Positive: WireGuardMeshAdapter configures local_ipv6 destination pinning cleanly.
    let adapter = WireGuardMeshAdapter::new(MeshIdentity {
        node_name: "n0".to_owned(),
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
            name: "p0".to_owned(),
            public_key_hex: "cc".repeat(32),
            allowed_ips_v6: vec!["fd00:db8::10/128".to_owned()],
            endpoint_v6: None,
            persistent_keepalive_secs: 25,
        }],
    };
    assert!(adapter.configure_tunnel(&cfg).is_ok());
}
