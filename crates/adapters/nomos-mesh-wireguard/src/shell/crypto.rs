//! Adaptive ChaCha20-Poly1305 AEAD crypto (`< 48` items or `< 48 KiB` inline,
//! `>= 48` items AND `>= 48 KiB` `rayon`), zero per-batch heap allocations,
//! and cached `ring::aead::LessSafeKey` per peer session.

use nomos_mesh::{AEAD_TAG_LEN, INNER_OFFSET, MeshError, MeshResult, WG_HEADER_LEN};
use rayon::prelude::*;
use ring::aead::{Aad, LessSafeKey, Nonce};

use crate::boundary::{
    BufHandle, BufPool, CipherDirection, CipherTask, DecryptedRx, KeyTable, MAX_BATCH_SPLIT,
};

/// Adaptive work-item threshold matching `green-infra` AVX2 benchmark sweet spot.
pub const PARALLEL_CRYPTO_THRESHOLD: usize = 48;

/// Alias for [`PARALLEL_CRYPTO_THRESHOLD`].
pub const DEFAULT_RAYON_THRESHOLD: usize = PARALLEL_CRYPTO_THRESHOLD;

/// Minimum aggregate payload bytes required before offloading AEAD to `rayon`.
pub const PARALLEL_CRYPTO_MIN_BYTES: usize = 48 * 1024;

/// Selected execution mode for an AEAD batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoDispatchMode {
    /// Execute cache-hot inline on the calling thread.
    Inline,
    /// Fan out across the `rayon` worker pool.
    Rayon,
}

/// Evaluates whether a batch of `(work_items, total_bytes)` should run inline or on `rayon`.
#[inline]
pub const fn dispatch_mode_for_batch(
    work_items: usize,
    total_bytes: usize,
    threshold: usize,
    min_bytes: usize,
) -> CryptoDispatchMode {
    if work_items >= threshold && total_bytes >= min_bytes {
        CryptoDispatchMode::Rayon
    } else {
        CryptoDispatchMode::Inline
    }
}

/// Adaptive AEAD batch processor running ChaCha20-Poly1305 at the shell edge.
///
/// Uses fixed stack arrays bounded by [`MAX_BATCH_SPLIT`] and cached
/// `LessSafeKey` instances inside [`KeyTable`] to avoid per-batch `Vec`
/// allocations and per-packet key expansion.
#[derive(Clone, Copy, Debug)]
pub struct AdaptiveCipher {
    rayon_threshold: usize,
    min_parallel_bytes: usize,
}

impl Default for AdaptiveCipher {
    fn default() -> Self {
        Self {
            rayon_threshold: PARALLEL_CRYPTO_THRESHOLD,
            min_parallel_bytes: PARALLEL_CRYPTO_MIN_BYTES,
        }
    }
}

impl AdaptiveCipher {
    /// Creates an adaptive cipher engine with a custom batch item threshold.
    pub fn with_threshold(rayon_threshold: usize) -> Self {
        Self {
            rayon_threshold: rayon_threshold.max(1),
            min_parallel_bytes: PARALLEL_CRYPTO_MIN_BYTES,
        }
    }

    /// Creates an adaptive cipher engine with explicit item and byte thresholds.
    pub fn with_thresholds(rayon_threshold: usize, min_parallel_bytes: usize) -> Self {
        Self {
            rayon_threshold: rayon_threshold.max(1),
            min_parallel_bytes,
        }
    }

    /// Configured item threshold.
    pub const fn rayon_threshold(self) -> usize {
        self.rayon_threshold
    }

    /// Configured minimum total byte guard for `rayon` dispatch.
    pub const fn min_parallel_bytes(self) -> usize {
        self.min_parallel_bytes
    }

    /// Returns the execution mode chosen for `(work_items, total_bytes)`.
    pub const fn dispatch_mode(self, work_items: usize, total_bytes: usize) -> CryptoDispatchMode {
        dispatch_mode_for_batch(
            work_items,
            total_bytes,
            self.rayon_threshold,
            self.min_parallel_bytes,
        )
    }

    /// Encrypts outbound [`CipherTask`]s inline or with `rayon` using zero per-batch `Vec` allocs.
    pub fn encrypt_batch(
        self,
        tasks: &[CipherTask],
        pool: &mut BufPool,
        keys: &KeyTable,
    ) -> MeshResult<()> {
        let total_bytes: usize = tasks.iter().map(|t| t.wire_len).sum();
        if self.dispatch_mode(tasks.len(), total_bytes) == CryptoDispatchMode::Inline
            || tasks.iter().any(|t| t.handle.is_gso())
        {
            for task in tasks {
                let session = keys
                    .get(task.peer_slot)
                    .ok_or_else(|| MeshError::Crypto("missing TX session keys".into()))?;
                let buf = pool.slice_mut(task.handle)?;
                seal_slot(task, buf, session.tx_aead())?;
            }
            return Ok(());
        }

        for chunk in tasks.chunks(MAX_BATCH_SPLIT) {
            let chunk_bytes: usize = chunk.iter().map(|t| t.wire_len).sum();
            let mut handles = [BufHandle::placeholder(); MAX_BATCH_SPLIT];
            for (i, task) in chunk.iter().enumerate() {
                handles[i] = task.handle;
            }
            let mut slices: [Option<&mut [u8]>; MAX_BATCH_SPLIT] =
                [const { None }; MAX_BATCH_SPLIT];
            pool.split_single_batch_mut(&handles[..chunk.len()], &mut slices)?;

            if self.dispatch_mode(chunk.len(), chunk_bytes) == CryptoDispatchMode::Inline {
                for (i, task) in chunk.iter().enumerate() {
                    let session = keys
                        .get(task.peer_slot)
                        .ok_or_else(|| MeshError::Crypto("missing TX session keys".into()))?;
                    let slot = slices[i]
                        .as_deref_mut()
                        .ok_or_else(|| MeshError::InvalidConfig("missing batch slice".into()))?;
                    seal_slot(task, slot, session.tx_aead())?;
                }
            } else {
                slices[..chunk.len()]
                    .par_iter_mut()
                    .enumerate()
                    .try_for_each(|(i, slot_opt)| {
                        let task = &chunk[i];
                        let session = keys
                            .get(task.peer_slot)
                            .ok_or_else(|| MeshError::Crypto("missing TX session keys".into()))?;
                        let slot = slot_opt.as_deref_mut().ok_or_else(|| {
                            MeshError::InvalidConfig("missing batch slice".into())
                        })?;
                        seal_slot(task, slot, session.tx_aead())
                    })?;
            }
        }
        Ok(())
    }

    /// Verifies and decrypts inbound WireGuard frames BEFORE `CoreEngine::poll`.
    ///
    /// Uses stack-allocated chunk arrays (zero per-batch `Vec` allocations) and
    /// immediately recycles tampered/unknown-key [`BufHandle`]s back to `pool`.
    pub fn decrypt_and_verify_batch(
        self,
        tasks: &[CipherTask],
        pool: &mut BufPool,
        keys: &KeyTable,
        verified_out: &mut Vec<DecryptedRx>,
    ) -> MeshResult<u64> {
        let mut tampered_drops = 0u64;
        let total_bytes: usize = tasks.iter().map(|t| t.wire_len).sum();

        if self.dispatch_mode(tasks.len(), total_bytes) == CryptoDispatchMode::Inline
            || tasks.iter().any(|t| t.handle.is_gso())
        {
            for task in tasks {
                let ok = keys
                    .get(task.peer_slot)
                    .and_then(|s| {
                        pool.slice_mut(task.handle)
                            .ok()
                            .map(|buf| open_slot(task, buf, s.rx_aead()).is_ok())
                    })
                    .unwrap_or(false);
                if ok {
                    verified_out.push(to_decrypted_rx(task));
                } else {
                    pool.release(task.handle);
                    tampered_drops += 1;
                }
            }
            return Ok(tampered_drops);
        }

        for chunk in tasks.chunks(MAX_BATCH_SPLIT) {
            let mut handles = [BufHandle::placeholder(); MAX_BATCH_SPLIT];
            let mut chunk_bytes = 0usize;
            for (i, task) in chunk.iter().enumerate() {
                handles[i] = task.handle;
                chunk_bytes = chunk_bytes.saturating_add(task.wire_len);
            }
            let mut slices: [Option<&mut [u8]>; MAX_BATCH_SPLIT] =
                [const { None }; MAX_BATCH_SPLIT];
            if pool
                .split_single_batch_mut(&handles[..chunk.len()], &mut slices)
                .is_err()
            {
                for task in chunk {
                    pool.release(task.handle);
                    tampered_drops += 1;
                }
                continue;
            }

            let mut results = [false; MAX_BATCH_SPLIT];
            if self.dispatch_mode(chunk.len(), chunk_bytes) == CryptoDispatchMode::Inline {
                for (i, task) in chunk.iter().enumerate() {
                    results[i] = match (slices[i].as_deref_mut(), keys.get(task.peer_slot)) {
                        (Some(slot), Some(session)) => {
                            open_slot(task, slot, session.rx_aead()).is_ok()
                        }
                        _ => false,
                    };
                }
            } else {
                slices[..chunk.len()]
                    .par_iter_mut()
                    .zip(results[..chunk.len()].par_iter_mut())
                    .enumerate()
                    .for_each(|(i, (slot_opt, ok))| {
                        let task = &chunk[i];
                        *ok = match (slot_opt.as_deref_mut(), keys.get(task.peer_slot)) {
                            (Some(slot), Some(session)) => {
                                open_slot(task, slot, session.rx_aead()).is_ok()
                            }
                            _ => false,
                        };
                    });
            }

            for (i, task) in chunk.iter().enumerate() {
                if results[i] {
                    verified_out.push(to_decrypted_rx(task));
                } else {
                    pool.release(task.handle);
                    tampered_drops += 1;
                }
            }
        }
        Ok(tampered_drops)
    }
}

#[inline]
const fn to_decrypted_rx(task: &CipherTask) -> DecryptedRx {
    DecryptedRx {
        handle: task.handle,
        peer_slot: task.peer_slot,
        counter: task.counter,
        wire_len: task.wire_len,
        ecn: task.ecn,
    }
}

#[inline]
fn wireguard_nonce(counter: u64) -> Nonce {
    let mut raw = [0u8; 12];
    raw[4..12].copy_from_slice(&counter.to_le_bytes());
    Nonce::assume_unique_for_key(raw)
}

fn seal_slot(task: &CipherTask, buf: &mut [u8], key: &LessSafeKey) -> MeshResult<()> {
    if task.direction != CipherDirection::Encrypt {
        return Err(MeshError::InvalidConfig(
            "expected Encrypt direction".into(),
        ));
    }
    if task.wire_len < INNER_OFFSET + AEAD_TAG_LEN || task.wire_len > buf.len() {
        return Err(MeshError::InvalidConfig("invalid wire_len for seal".into()));
    }
    let (hdr, rest) = buf[..task.wire_len].split_at_mut(WG_HEADER_LEN);
    let body_len = rest.len() - AEAD_TAG_LEN;
    let (plaintext, tag_dst) = rest.split_at_mut(body_len);
    let tag = key
        .seal_in_place_separate_tag(wireguard_nonce(task.counter), Aad::from(hdr), plaintext)
        .map_err(|_| MeshError::Crypto("ChaCha20-Poly1305 seal failed".into()))?;
    tag_dst.copy_from_slice(tag.as_ref());
    Ok(())
}

fn open_slot(task: &CipherTask, buf: &mut [u8], key: &LessSafeKey) -> MeshResult<()> {
    if task.direction != CipherDirection::Decrypt {
        return Err(MeshError::InvalidConfig(
            "expected Decrypt direction".into(),
        ));
    }
    if task.wire_len < INNER_OFFSET + AEAD_TAG_LEN || task.wire_len > buf.len() {
        return Err(MeshError::InvalidConfig("invalid wire_len for open".into()));
    }
    let (hdr, rest) = buf[..task.wire_len].split_at_mut(WG_HEADER_LEN);
    key.open_in_place(wireguard_nonce(task.counter), Aad::from(hdr), rest)
        .map_err(|_| MeshError::Crypto("Poly1305 AEAD authentication failed".into()))?;
    Ok(())
}
