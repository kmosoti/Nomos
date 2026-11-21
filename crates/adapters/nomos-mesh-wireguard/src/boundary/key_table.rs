//! Zeroized session keys and atomic RCU key-table updates.

use std::sync::Arc;

/// Symmetric WireGuard-style session keys for one peer slot.
///
/// Key bytes are zeroed deterministically on [`Drop`].
#[derive(Clone)]
pub struct SessionKeys {
    /// Local receiver index advertised to the peer.
    pub receiver_index: u32,
    /// Remote peer's receiver index used on outbound transport headers.
    pub sender_index: u32,
    /// 256-bit ChaCha20-Poly1305 key for outbound encryption.
    pub tx_key: [u8; 32],
    /// 256-bit ChaCha20-Poly1305 key for inbound decryption.
    pub rx_key: [u8; 32],
    /// Monotonic key rotation epoch.
    pub epoch: u64,
}

impl Drop for SessionKeys {
    fn drop(&mut self) {
        self.tx_key.fill(0);
        self.rx_key.fill(0);
    }
}

impl core::fmt::Debug for SessionKeys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SessionKeys")
            .field("receiver_index", &self.receiver_index)
            .field("sender_index", &self.sender_index)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

/// Immutable snapshot of active peer AEAD keys shared across shell AEAD threads via `ArcSwap`.
#[derive(Clone, Debug, Default)]
pub struct KeyTable {
    slots: Vec<Option<Arc<SessionKeys>>>,
}

impl KeyTable {
    /// Creates an empty key table sized for `max_peers`.
    pub fn with_capacity(max_peers: usize) -> Self {
        Self {
            slots: vec![None; max_peers],
        }
    }

    /// Reads active keys for `peer_slot`.
    pub fn get(&self, peer_slot: u16) -> Option<&SessionKeys> {
        self.slots.get(peer_slot as usize)?.as_deref()
    }

    /// Resolves peer slot by matching `receiver_index`.
    pub fn find_by_receiver_index(&self, receiver_index: u32) -> Option<(u16, &SessionKeys)> {
        self.slots.iter().enumerate().find_map(|(idx, slot)| {
            let keys = slot.as_deref()?;
            (keys.receiver_index == receiver_index).then_some((idx as u16, keys))
        })
    }

    /// Applies an update into a cloned table snapshot ready for `ArcSwap::store`.
    pub fn apply_update(&self, update: KeyTableUpdate) -> Self {
        let mut next = self.clone();
        match update {
            KeyTableUpdate::Install { peer_slot, keys } => {
                let idx = peer_slot as usize;
                if idx >= next.slots.len() {
                    next.slots.resize(idx + 1, None);
                }
                next.slots[idx] = Some(Arc::new(keys));
            }
            KeyTableUpdate::Retire { peer_slot } => {
                if let Some(slot) = next.slots.get_mut(peer_slot as usize) {
                    *slot = None;
                }
            }
        }
        next
    }

    /// Count of peer slots with installed session keys.
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

/// Key rotation command emitted by the tunnel core for the async key store.
#[derive(Clone, Debug)]
pub enum KeyTableUpdate {
    /// Installs or rotates symmetric AEAD keys for `peer_slot`.
    Install {
        /// Target peer slot index.
        peer_slot: u16,
        /// New zeroizable key pair.
        keys: SessionKeys,
    },
    /// Retires active session keys on timeout or disconnect.
    Retire {
        /// Target peer slot index.
        peer_slot: u16,
    },
}
