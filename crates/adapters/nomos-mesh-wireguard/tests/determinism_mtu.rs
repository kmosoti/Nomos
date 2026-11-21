//! Determinism, jumbo PMTU sweep, AllowedIPs/UDP checksum, TimerWheel, and adaptive AEAD tests.

use std::net::Ipv6Addr;

use nomos_mesh::{
    AEAD_TAG_LEN, INNER_OFFSET, MeshError, TunnelConfig, WG_HEADER_LEN, max_quic_payload,
};
use nomos_mesh_wireguard::{
    AdaptiveCipher, BufPool, CipherDirection, CipherTask, CoreEngine, CoreOutput,
    CryptoDispatchMode, DecryptedRx, EcnCodepoint, FixedRing, InnerEndpoint, KeyStore,
    PARALLEL_CRYPTO_MIN_BYTES, PARALLEL_CRYPTO_THRESHOLD, ReplayWindow, SLOT_BYTES, SessionKeys,
    TimerAction, TimerEntry, TimerWheel, TunnelQuicBudget,
};

fn make_pair() -> (InnerEndpoint, InnerEndpoint) {
    (
        InnerEndpoint {
            addr: "fd00:db8::1".parse::<Ipv6Addr>().unwrap(),
            port: 5000,
        },
        InnerEndpoint {
            addr: "fd00:db8::2".parse::<Ipv6Addr>().unwrap(),
            port: 5001,
        },
    )
}

#[test]
fn pmtu_and_jumbo_slot_invariant_holds_across_link_sweep() {
    assert_eq!(SLOT_BYTES, 9216);
    let (src, dst) = make_pair();
    for outer_pmtu in [1280u16, 1380, 1460, 1500, 4096, 9000] {
        let budget = TunnelQuicBudget::from_outer_pmtu(outer_pmtu).unwrap();
        let max_payload = max_quic_payload(outer_pmtu);
        assert_eq!(max_payload, outer_pmtu - 80);
        assert!(budget.fits(max_payload as usize));
        assert!(!budget.fits(max_payload as usize + 1));
        assert_eq!(
            INNER_OFFSET + max_payload as usize + AEAD_TAG_LEN,
            outer_pmtu as usize
        );

        let mut cfg = quinn_proto::TransportConfig::default();
        budget.apply_to_transport_config(&mut cfg);

        let mut pool = BufPool::new(4, 0).unwrap();
        let mut engine = CoreEngine::new(1, outer_pmtu, src, dst).unwrap();
        let mut key_updates = FixedRing::with_capacity(4);
        engine
            .install_peer_session(
                0,
                SessionKeys::new(1, 2, [1u8; 32], [2u8; 32], 1),
                10,
                &mut key_updates,
            )
            .unwrap();

        let max_buf = vec![0xABu8; max_payload as usize];
        let ok = engine.encapsulate_quic_tx(0, &max_buf, src, dst, EcnCodepoint::None, &mut pool);
        assert!(
            ok.is_ok(),
            "outer_pmtu={outer_pmtu} max payload must fit jumbo slot"
        );

        let too_big = vec![0xABu8; max_payload as usize + 1];
        let err = engine
            .encapsulate_quic_tx(0, &too_big, src, dst, EcnCodepoint::None, &mut pool)
            .unwrap_err();
        assert!(matches!(err, MeshError::InvalidConfig(_)));
    }
    assert!(TunnelQuicBudget::from_outer_pmtu(1279).is_err());
}

#[test]
fn identical_now_ns_and_rx_batches_produce_byte_identical_outputs() {
    fn run_sequence() -> Vec<Vec<u8>> {
        let (src, dst) = make_pair();
        let mut engine = CoreEngine::new(2, 1460, src, dst).unwrap();
        let mut pool = BufPool::new(16, 0).unwrap();
        let mut key_updates = FixedRing::with_capacity(8);
        engine
            .install_peer_session(
                0,
                SessionKeys::new(7, 9, [0x33u8; 32], [0x44u8; 32], 1),
                1_000,
                &mut key_updates,
            )
            .unwrap();

        let mut snapshots = Vec::new();
        for step in 0..4u64 {
            let mut payload = [0u8; 32];
            payload[0] = step as u8;
            let tx = engine
                .encapsulate_quic_tx(0, &payload, src, dst, EcnCodepoint::Ect0, &mut pool)
                .unwrap();
            snapshots.push(pool.slice(tx.handle, tx.wire_len).unwrap().to_vec());

            let rx = [DecryptedRx {
                handle: tx.handle,
                peer_slot: 0,
                counter: step,
                wire_len: tx.wire_len,
                ecn: EcnCodepoint::Ect0,
            }];
            let mut out = FixedRing::with_capacity(8);
            let mut cipher_out = FixedRing::with_capacity(8);
            engine
                .poll(
                    1_000 + (step + 1) * 30_000_000_000,
                    &rx,
                    &mut pool,
                    &mut out,
                    &mut cipher_out,
                )
                .unwrap();
            while let Some(item) = out.pop() {
                if let CoreOutput::DeliverInner { handle, .. } = item {
                    pool.release(handle);
                }
            }
            while let Some(task) = cipher_out.pop() {
                snapshots.push(pool.slice(task.handle, task.wire_len).unwrap().to_vec());
                pool.release(task.handle);
            }
        }
        snapshots
    }

    assert_eq!(run_sequence(), run_sequence());
}

#[test]
fn adaptive_aead_48_item_48kib_guard_and_tampered_recycle() {
    let cipher = AdaptiveCipher::default();
    assert_eq!(cipher.rayon_threshold(), PARALLEL_CRYPTO_THRESHOLD);
    assert_eq!(cipher.min_parallel_bytes(), PARALLEL_CRYPTO_MIN_BYTES);
    assert_eq!(
        cipher.dispatch_mode(47, 64 * 1024),
        CryptoDispatchMode::Inline
    );
    assert_eq!(
        cipher.dispatch_mode(48, 48 * 256),
        CryptoDispatchMode::Inline
    );
    assert_eq!(
        cipher.dispatch_mode(48, 48 * 1360),
        CryptoDispatchMode::Rayon
    );

    let (src, dst) = make_pair();
    for (batch_len, payload_size) in [(4usize, 128usize), (52usize, 1200usize)] {
        let mut engine = CoreEngine::new(2, 1460, src, dst).unwrap();
        let mut pool = BufPool::new(64, 0).unwrap();
        let mut updates = FixedRing::with_capacity(4);
        let store = KeyStore::with_capacity(2);
        engine
            .install_peer_session(
                0,
                SessionKeys::new(55, 55, [0x99u8; 32], [0x99u8; 32], 1),
                100,
                &mut updates,
            )
            .unwrap();
        while let Some(u) = updates.pop() {
            store.apply(u);
        }

        let payload = vec![0x5Au8; payload_size];
        let mut tx_tasks = Vec::new();
        for _ in 0..batch_len {
            tx_tasks.push(
                engine
                    .encapsulate_quic_tx(0, &payload, src, dst, EcnCodepoint::None, &mut pool)
                    .unwrap(),
            );
        }
        let table = store.load();
        cipher.encrypt_batch(&tx_tasks, &mut pool, &table).unwrap();

        // Tamper the last packet's tag and verify immediate BufHandle recycle.
        {
            let last = tx_tasks.last().unwrap();
            let slice = pool.slice_mut(last.handle).unwrap();
            slice[last.wire_len - 1] ^= 0xFF;
        }
        let rx_tasks: Vec<CipherTask> = tx_tasks
            .iter()
            .map(|t| CipherTask {
                direction: CipherDirection::Decrypt,
                ..*t
            })
            .collect();
        let mut verified = Vec::new();
        let drops = cipher
            .decrypt_and_verify_batch(&rx_tasks, &mut pool, &table, &mut verified)
            .unwrap();
        assert_eq!(drops, 1);
        assert_eq!(verified.len(), batch_len - 1);
        for v in verified {
            pool.release(v.handle);
            // O(1) double-release must be a safe no-op.
            pool.release(v.handle);
        }
        assert_eq!(pool.in_use_count(), 0);
    }

    let mut replay = ReplayWindow::new();
    assert!(replay.check_and_accept(1).is_ok());
    assert!(matches!(
        replay.check_and_accept(1),
        Err(MeshError::ReplayDetected(1))
    ));
    assert_eq!(WG_HEADER_LEN + 48 + AEAD_TAG_LEN, 80);

    let bad_cfg = TunnelConfig {
        interface: "wg0".to_owned(),
        local_ipv6: "fd00:db8::1/64".to_owned(),
        listen_port: 51820,
        outer_pmtu: 1460,
        accept_ra: 1,
        peers: Vec::new(),
    };
    assert!(bad_cfg.validate().is_err());
}

#[test]
fn core_engine_enforces_allowed_ips_udp_checksum_and_overflow_safe_timers() {
    let (src, dst) = make_pair();
    let spoofed = InnerEndpoint {
        addr: "fd00:dead::99".parse::<Ipv6Addr>().unwrap(),
        port: 5000,
    };

    let mut engine = CoreEngine::new(2, 1460, dst, src).unwrap();
    engine.set_peer_allowed_prefix(0, src.addr, 128).unwrap();
    let mut pool = BufPool::new(16, 0).unwrap();
    let mut updates = FixedRing::with_capacity(8);
    engine
        .install_peer_session(
            0,
            SessionKeys::new(10, 20, [0xAA; 32], [0xBB; 32], 1),
            100,
            &mut updates,
        )
        .unwrap();

    // 1. Spoofed source IPv6 outside AllowedIPs must be rejected.
    let bad_src_pkt = engine
        .encapsulate_quic_tx(0, b"spoofed", spoofed, dst, EcnCodepoint::None, &mut pool)
        .unwrap();
    let mut out = FixedRing::with_capacity(8);
    let mut cipher_out = FixedRing::with_capacity(8);
    engine
        .poll(
            200,
            &[DecryptedRx {
                handle: bad_src_pkt.handle,
                peer_slot: 0,
                counter: 0,
                wire_len: bad_src_pkt.wire_len,
                ecn: EcnCodepoint::None,
            }],
            &mut pool,
            &mut out,
            &mut cipher_out,
        )
        .unwrap();
    assert_eq!(engine.dropped_invalid, 1);
    assert!(matches!(out.pop(), Some(CoreOutput::RecycleHandle { .. })));
    pool.release(bad_src_pkt.handle);

    // 2. Corrupted inner UDP checksum must be rejected.
    let bad_csum_pkt = engine
        .encapsulate_quic_tx(0, b"checksum-test", src, dst, EcnCodepoint::None, &mut pool)
        .unwrap();
    {
        let slice = pool.slice_mut(bad_csum_pkt.handle).unwrap();
        slice[WG_HEADER_LEN + 46] ^= 0xFF; // corrupt UDP checksum field
    }
    engine
        .poll(
            300,
            &[DecryptedRx {
                handle: bad_csum_pkt.handle,
                peer_slot: 0,
                counter: 1,
                wire_len: bad_csum_pkt.wire_len,
                ecn: EcnCodepoint::None,
            }],
            &mut pool,
            &mut out,
            &mut cipher_out,
        )
        .unwrap();
    assert_eq!(engine.dropped_invalid, 2);
    pool.release(bad_csum_pkt.handle);

    // 3. TimerWheel O(1) next_wakeup_ns + overflow-safe draining (never loses expired timers).
    let mut wheel = TimerWheel::with_capacity(16);
    for slot in 0..5u16 {
        wheel
            .schedule(TimerEntry {
                deadline_ns: 1_000 + u64::from(slot) * 10,
                peer_slot: slot,
                action: TimerAction::Keepalive,
            })
            .unwrap();
    }
    assert_eq!(wheel.next_wakeup_ns(), Some(1_000));
    let mut small = [None; 2];
    assert_eq!(wheel.poll_expired(2_000, &mut small), 2);
    assert_eq!(wheel.len(), 3);
    assert_eq!(wheel.poll_expired(2_000, &mut small), 2);
    assert_eq!(wheel.len(), 1);
    assert_eq!(wheel.poll_expired(2_000, &mut small), 1);
    assert!(wheel.is_empty());
    assert_eq!(wheel.next_wakeup_ns(), None);
}
