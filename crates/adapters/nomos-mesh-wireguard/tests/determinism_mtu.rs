//! Determinism, PMTU/tailroom sweep, replay rejection, and adaptive AEAD tests.

use std::net::Ipv6Addr;

use nomos_mesh::{AEAD_TAG_LEN, INNER_OFFSET, MeshError, TunnelConfig, max_quic_payload};
use nomos_mesh_wireguard::{
    AdaptiveCipher, BufPool, CipherDirection, CipherTask, CoreEngine, CoreOutput, DecryptedRx,
    EcnCodepoint, FixedRing, InnerEndpoint, KeyStore, ReplayWindow, SessionKeys, TunnelQuicBudget,
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
fn pmtu_and_tailroom_invariant_holds_across_link_sweep() {
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
        if (max_payload as usize + 80) <= nomos_mesh_wireguard::SLOT_BYTES {
            let mut engine = CoreEngine::new(1, outer_pmtu, src, dst).unwrap();
            let mut key_updates = FixedRing::with_capacity(4);
            engine
                .install_peer_session(
                    0,
                    SessionKeys {
                        receiver_index: 1,
                        sender_index: 2,
                        tx_key: [1u8; 32],
                        rx_key: [2u8; 32],
                        epoch: 1,
                    },
                    10,
                    &mut key_updates,
                )
                .unwrap();

            let max_buf = vec![0xABu8; max_payload as usize];
            let ok =
                engine.encapsulate_quic_tx(0, &max_buf, src, dst, EcnCodepoint::None, &mut pool);
            assert!(ok.is_ok(), "outer_pmtu={outer_pmtu} max payload must fit");

            let too_big = vec![0xABu8; max_payload as usize + 1];
            let err = engine
                .encapsulate_quic_tx(0, &too_big, src, dst, EcnCodepoint::None, &mut pool)
                .unwrap_err();
            assert!(matches!(err, MeshError::InvalidConfig(_)));
        }
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
                SessionKeys {
                    receiver_index: 7,
                    sender_index: 9,
                    tx_key: [0x33u8; 32],
                    rx_key: [0x44u8; 32],
                    epoch: 1,
                },
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
fn adaptive_aead_inline_and_rayon_batches_roundtrip_and_enforce_replay() {
    let (src, dst) = make_pair();
    for batch_len in [4usize, 12usize] {
        let mut engine = CoreEngine::new(2, 1460, src, dst).unwrap();
        let mut pool = BufPool::new(32, 0).unwrap();
        let mut updates = FixedRing::with_capacity(4);
        let store = KeyStore::with_capacity(2);

        engine
            .install_peer_session(
                0,
                SessionKeys {
                    receiver_index: 55,
                    sender_index: 55,
                    tx_key: [0x99u8; 32],
                    rx_key: [0x99u8; 32],
                    epoch: 1,
                },
                100,
                &mut updates,
            )
            .unwrap();
        while let Some(u) = updates.pop() {
            store.apply(u);
        }

        let mut tx_tasks = Vec::new();
        for i in 0..batch_len {
            let msg = format!("packet-{i:02}");
            tx_tasks.push(
                engine
                    .encapsulate_quic_tx(0, msg.as_bytes(), src, dst, EcnCodepoint::None, &mut pool)
                    .unwrap(),
            );
        }

        let table = store.load();
        let cipher = AdaptiveCipher::default();
        cipher.encrypt_batch(&tx_tasks, &mut pool, &table).unwrap();

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
        assert_eq!(drops, 0);
        assert_eq!(verified.len(), batch_len);
    }

    let mut replay = ReplayWindow::new();
    assert!(replay.check_and_accept(1).is_ok());
    assert!(matches!(
        replay.check_and_accept(1),
        Err(MeshError::ReplayDetected(1))
    ));
    assert!(replay.check_and_accept(3000).is_ok());
    assert!(matches!(
        replay.check_and_accept(1),
        Err(MeshError::ReplayDetected(1))
    ));

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
