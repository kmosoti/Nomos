//! Zero-allocation steady-state gate and tampered-packet handle recycling tests.

use std::alloc::System;
use std::net::Ipv6Addr;

use nomos_mesh::{AEAD_TAG_LEN, INNER_OFFSET, WG_HEADER_LEN};
use nomos_mesh_wireguard::{
    AdaptiveCipher, BufPool, CipherDirection, CipherTask, CoreEngine, CoreOutput, DecryptedRx,
    EcnCodepoint, FixedRing, InnerEndpoint, KeyStore, SessionKeys,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn sample_endpoints() -> (InnerEndpoint, InnerEndpoint) {
    (
        InnerEndpoint {
            addr: "fd00:db8::1".parse::<Ipv6Addr>().unwrap(),
            port: 7443,
        },
        InnerEndpoint {
            addr: "fd00:db8::2".parse::<Ipv6Addr>().unwrap(),
            port: 7443,
        },
    )
}

#[test]
fn core_engine_poll_and_encapsulate_perform_zero_heap_allocations() {
    let (src, dst) = sample_endpoints();
    let mut engine = CoreEngine::new(4, 1460, src, dst).unwrap();
    let mut pool = BufPool::new(64, 4).unwrap();
    let mut out = FixedRing::with_capacity(64);
    let mut cipher_out = FixedRing::with_capacity(64);
    let mut key_updates = FixedRing::with_capacity(16);

    let keys = SessionKeys {
        receiver_index: 101,
        sender_index: 202,
        tx_key: [0x11u8; 32],
        rx_key: [0x22u8; 32],
        epoch: 1,
    };
    engine
        .install_peer_session(0, keys, 1_000_000_000, &mut key_updates)
        .unwrap();

    // Warm up one RX handle before counting steady-state allocations.
    let rx_payload = b"quic-steady-state-frame";
    let mut rx_engine = CoreEngine::new(4, 1460, dst, src).unwrap();
    let mut dummy_updates = FixedRing::with_capacity(4);
    rx_engine
        .install_peer_session(
            0,
            SessionKeys {
                receiver_index: 202,
                sender_index: 101,
                tx_key: [0x22u8; 32],
                rx_key: [0x11u8; 32],
                epoch: 1,
            },
            1_000_000_000,
            &mut dummy_updates,
        )
        .unwrap();

    let tx_from_peer = rx_engine
        .encapsulate_quic_tx(0, rx_payload, dst, src, EcnCodepoint::Ect0, &mut pool)
        .unwrap();
    let rx_batch = [DecryptedRx {
        handle: tx_from_peer.handle,
        peer_slot: 0,
        counter: tx_from_peer.counter,
        wire_len: tx_from_peer.wire_len,
        ecn: EcnCodepoint::Ect0,
    }];

    // Measure steady-state RX poll + TX encapsulation under global StatsAlloc.
    let region = Region::new(GLOBAL);
    engine
        .poll(
            1_000_050_000,
            &rx_batch,
            &mut pool,
            &mut out,
            &mut cipher_out,
        )
        .unwrap();
    let tx_task = engine
        .encapsulate_quic_tx(
            0,
            b"outbound-quic-datagram",
            src,
            dst,
            EcnCodepoint::Ect1,
            &mut pool,
        )
        .unwrap();
    let stats = region.change();
    assert_eq!(
        stats.allocations, 0,
        "steady-state CoreEngine::poll/encapsulate_quic_tx allocated on heap: {stats:?}"
    );
    assert_eq!(stats.reallocations, 0);

    // Release handles cleanly and verify zero leak.
    while let Some(item) = out.pop() {
        if let CoreOutput::DeliverInner {
            handle,
            payload_offset,
            payload_len,
            ..
        } = item
        {
            assert_eq!(payload_offset, INNER_OFFSET);
            assert_eq!(
                pool.slice(handle, INNER_OFFSET + payload_len).unwrap()
                    [INNER_OFFSET..INNER_OFFSET + payload_len],
                *rx_payload
            );
            pool.release(handle);
        }
    }
    pool.release(tx_task.handle);
    assert_eq!(pool.in_use_count(), 0);
}

#[test]
fn tampered_packets_fail_edge_aead_and_recycle_buf_handles() {
    let (src, dst) = sample_endpoints();
    let mut engine = CoreEngine::new(2, 1460, src, dst).unwrap();
    let mut pool = BufPool::new(16, 0).unwrap();
    let mut key_updates = FixedRing::with_capacity(8);
    let keys = KeyStore::with_capacity(2);

    let session = SessionKeys {
        receiver_index: 11,
        sender_index: 22,
        tx_key: [0x5Au8; 32],
        rx_key: [0x5Au8; 32],
        epoch: 1,
    };
    engine
        .install_peer_session(0, session, 500, &mut key_updates)
        .unwrap();
    while let Some(upd) = key_updates.pop() {
        keys.apply(upd);
    }

    let task = engine
        .encapsulate_quic_tx(
            0,
            b"sensitive-quic",
            src,
            dst,
            EcnCodepoint::None,
            &mut pool,
        )
        .unwrap();
    let cipher = AdaptiveCipher::default();
    let table = keys.load();
    cipher.encrypt_batch(&[task], &mut pool, &table).unwrap();

    // Tamper with trailing Poly1305 tag byte [len-1].
    {
        let slice = pool.slice_mut(task.handle).unwrap();
        let tag_last = task.wire_len - 1;
        slice[tag_last] ^= 0xFF;
    }

    let decrypt_task = CipherTask {
        direction: CipherDirection::Decrypt,
        ..CipherTask {
            handle: task.handle,
            peer_slot: 0,
            receiver_index: 11,
            counter: task.counter,
            wire_len: task.wire_len,
            direction: CipherDirection::Decrypt,
            ecn: EcnCodepoint::None,
        }
    };
    assert_eq!(pool.in_use_count(), 1);
    let mut verified = Vec::new();
    let drops = cipher
        .decrypt_and_verify_batch(&[decrypt_task], &mut pool, &table, &mut verified)
        .unwrap();

    assert_eq!(drops, 1);
    assert!(verified.is_empty());
    assert_eq!(
        pool.in_use_count(),
        0,
        "tampered BufHandle must be recycled immediately before CoreEngine::poll"
    );
    assert_eq!(WG_HEADER_LEN + 48 + AEAD_TAG_LEN, 80);
}
