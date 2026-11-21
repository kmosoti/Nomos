//! Adaptive ChaCha20-Poly1305 AEAD crypto (`< 8` inline, `>= 8` rayon batch).
//!
//! Edge decryption verifies the trailing 16-byte Poly1305 tag before handing
//! [`DecryptedRx`] to `CoreEngine::poll`; tampered packets are dropped and their
//! [`BufHandle`]s recycled immediately.

use nomos_mesh::{AEAD_TAG_LEN, INNER_OFFSET, MeshError, MeshResult, WG_HEADER_LEN};
use rayon::prelude::*;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};

use crate::boundary::{BufHandle, BufPool, CipherDirection, CipherTask, DecryptedRx, KeyTable};

/// Batch threshold switching from inline AEAD (`< 8`) to `rayon` parallel batching (`>= 8`).
pub const DEFAULT_RAYON_THRESHOLD: usize = 8;

/// Adaptive AEAD batch processor running ChaCha20-Poly1305 at the shell edge.
#[derive(Clone, Copy, Debug)]
pub struct AdaptiveCipher {
    rayon_threshold: usize,
}

impl Default for AdaptiveCipher {
    fn default() -> Self {
        Self {
            rayon_threshold: DEFAULT_RAYON_THRESHOLD,
        }
    }
}

impl AdaptiveCipher {
    /// Creates an adaptive cipher engine with custom batch switchover threshold.
    pub fn with_threshold(rayon_threshold: usize) -> Self {
        Self {
            rayon_threshold: rayon_threshold.max(1),
        }
    }

    /// Configured threshold (`tasks.len() >= threshold` uses `rayon`).
    pub const fn rayon_threshold(self) -> usize {
        self.rayon_threshold
    }

    /// Encrypts outbound [`CipherTask`]s inline (`< threshold`) or with `rayon` (`>= threshold`).
    pub fn encrypt_batch(
        self,
        tasks: &[CipherTask],
        pool: &mut BufPool,
        keys: &KeyTable,
    ) -> MeshResult<()> {
        if tasks.len() < self.rayon_threshold || tasks.iter().any(|t| t.handle.is_gso()) {
            for task in tasks {
                let raw_key = keys
                    .get(task.peer_slot)
                    .ok_or_else(|| MeshError::Crypto("missing TX session keys".into()))?
                    .tx_key;
                let buf = pool.slice_mut(task.handle)?;
                seal_slot(task, buf, raw_key)?;
            }
            return Ok(());
        }

        let mut slots = pool.split_pkt_slots_mut();
        let mut work = Vec::with_capacity(tasks.len());
        for task in tasks {
            let raw_key = keys
                .get(task.peer_slot)
                .ok_or_else(|| MeshError::Crypto("missing TX session keys".into()))?
                .tx_key;
            let slot = slots
                .get_mut(task.handle.index() as usize)
                .and_then(Option::take)
                .ok_or_else(|| MeshError::InvalidConfig("duplicate or invalid handle".into()))?;
            work.push((*task, slot, raw_key));
        }

        work.par_iter_mut()
            .try_for_each(|(task, slot, raw_key)| seal_slot(task, slot, *raw_key))
    }

    /// Verifies and decrypts inbound WireGuard frames BEFORE `CoreEngine::poll`.
    ///
    /// Returns verified [`DecryptedRx`] descriptors and drops/recycles tampered
    /// packets immediately, returning the count of tampered drops.
    pub fn decrypt_and_verify_batch(
        self,
        tasks: &[CipherTask],
        pool: &mut BufPool,
        keys: &KeyTable,
        verified_out: &mut Vec<DecryptedRx>,
    ) -> MeshResult<u64> {
        let mut tampered_drops = 0u64;
        if tasks.len() < self.rayon_threshold || tasks.iter().any(|t| t.handle.is_gso()) {
            for task in tasks {
                let Some(session) = keys.get(task.peer_slot) else {
                    pool.release(task.handle);
                    tampered_drops += 1;
                    continue;
                };
                let ok = match pool.slice_mut(task.handle) {
                    Ok(buf) => open_slot(task, buf, session.rx_key).is_ok(),
                    Err(_) => false,
                };
                if ok {
                    verified_out.push(DecryptedRx {
                        handle: task.handle,
                        peer_slot: task.peer_slot,
                        counter: task.counter,
                        wire_len: task.wire_len,
                        ecn: task.ecn,
                    });
                } else {
                    pool.release(task.handle);
                    tampered_drops += 1;
                }
            }
            return Ok(tampered_drops);
        }

        let mut missing_handles: Vec<BufHandle> = Vec::new();
        let outcomes: Vec<(CipherTask, bool)> = {
            let mut slots = pool.split_pkt_slots_mut();
            let mut work = Vec::with_capacity(tasks.len());
            for task in tasks {
                let Some(session) = keys.get(task.peer_slot) else {
                    missing_handles.push(task.handle);
                    continue;
                };
                if let Some(slot) = slots
                    .get_mut(task.handle.index() as usize)
                    .and_then(Option::take)
                {
                    work.push((*task, slot, session.rx_key, false));
                } else {
                    missing_handles.push(task.handle);
                }
            }

            work.par_iter_mut().for_each(|(task, slot, raw_key, ok)| {
                *ok = open_slot(task, slot, *raw_key).is_ok();
            });
            work.into_iter()
                .map(|(task, _, _, ok)| (task, ok))
                .collect()
        };

        for handle in missing_handles {
            pool.release(handle);
            tampered_drops += 1;
        }
        for (task, ok) in outcomes {
            if ok {
                verified_out.push(DecryptedRx {
                    handle: task.handle,
                    peer_slot: task.peer_slot,
                    counter: task.counter,
                    wire_len: task.wire_len,
                    ecn: task.ecn,
                });
            } else {
                pool.release(task.handle);
                tampered_drops += 1;
            }
        }
        Ok(tampered_drops)
    }
}

fn wireguard_nonce(counter: u64) -> Nonce {
    let mut raw = [0u8; 12];
    raw[4..12].copy_from_slice(&counter.to_le_bytes());
    Nonce::assume_unique_for_key(raw)
}

fn build_aead_key(raw_key: [u8; 32]) -> MeshResult<LessSafeKey> {
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, &raw_key)
        .map_err(|_| MeshError::Crypto("invalid ChaCha20-Poly1305 key".into()))?;
    Ok(LessSafeKey::new(unbound))
}

fn seal_slot(task: &CipherTask, buf: &mut [u8], raw_key: [u8; 32]) -> MeshResult<()> {
    if task.direction != CipherDirection::Encrypt {
        return Err(MeshError::InvalidConfig(
            "expected Encrypt direction".into(),
        ));
    }
    if task.wire_len < INNER_OFFSET + AEAD_TAG_LEN || task.wire_len > buf.len() {
        return Err(MeshError::InvalidConfig("invalid wire_len for seal".into()));
    }
    let key = build_aead_key(raw_key)?;
    let (hdr, rest) = buf[..task.wire_len].split_at_mut(WG_HEADER_LEN);
    let body_len = rest.len() - AEAD_TAG_LEN;
    let (plaintext, tag_dst) = rest.split_at_mut(body_len);
    let tag = key
        .seal_in_place_separate_tag(wireguard_nonce(task.counter), Aad::from(hdr), plaintext)
        .map_err(|_| MeshError::Crypto("ChaCha20-Poly1305 seal failed".into()))?;
    tag_dst.copy_from_slice(tag.as_ref());
    Ok(())
}

fn open_slot(task: &CipherTask, buf: &mut [u8], raw_key: [u8; 32]) -> MeshResult<()> {
    if task.direction != CipherDirection::Decrypt {
        return Err(MeshError::InvalidConfig(
            "expected Decrypt direction".into(),
        ));
    }
    if task.wire_len < INNER_OFFSET + AEAD_TAG_LEN || task.wire_len > buf.len() {
        return Err(MeshError::InvalidConfig("invalid wire_len for open".into()));
    }
    let key = build_aead_key(raw_key)?;
    let (hdr, rest) = buf[..task.wire_len].split_at_mut(WG_HEADER_LEN);
    key.open_in_place(wireguard_nonce(task.counter), Aad::from(hdr), rest)
        .map_err(|_| MeshError::Crypto("Poly1305 AEAD authentication failed".into()))?;
    Ok(())
}
