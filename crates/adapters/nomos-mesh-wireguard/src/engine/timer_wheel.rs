//! Monotonic pre-allocated timer wheel with `u64` occupied bitmasks and `O(1)` next-wakeup.
//!
//! Overflow-safe draining preserves any expired timers that do not fit into the
//! caller's output slice so timers are never silently lost.

use nomos_mesh::{MeshError, MeshResult};

/// Kind of scheduled peer maintenance action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerAction {
    /// Trigger WireGuard persistent keepalive packet for `peer_slot`.
    Keepalive,
    /// Trigger Noise_IKpsk2 session key rotation after 120s or byte limit.
    Rekey,
    /// Retire an expired session that exceeded the hard lifetime without rekeying.
    SessionExpire,
}

/// One armed timer entry inside [`TimerWheel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerEntry {
    /// Monotonic deadline in nanoseconds.
    pub deadline_ns: u64,
    /// Target peer slot.
    pub peer_slot: u16,
    /// Scheduled action.
    pub action: TimerAction,
}

/// Fixed-capacity timing wheel allocating once at construction.
///
/// Tracks occupied slots with `u64` bitmask words and caches `min_deadline_ns`
/// for `O(1)` [`Self::next_wakeup_ns`].
#[derive(Debug)]
pub struct TimerWheel {
    slots: Vec<Option<TimerEntry>>,
    occupied: Vec<u64>,
    len: usize,
    min_deadline_ns: Option<u64>,
}

impl TimerWheel {
    /// Pre-allocates `capacity` timer slots and backing `u64` occupancy bitmasks.
    pub fn with_capacity(capacity: usize) -> Self {
        let words = capacity.div_ceil(64).max(1);
        Self {
            slots: vec![None; capacity],
            occupied: vec![0u64; words],
            len: 0,
            min_deadline_ns: None,
        }
    }

    /// Number of currently armed timers.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True if no timers are currently armed.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the earliest armed deadline in `O(1)` time without scanning.
    #[inline]
    pub const fn next_wakeup_ns(&self) -> Option<u64> {
        self.min_deadline_ns
    }

    /// Arms or replaces a timer for `(peer_slot, action)` without heap allocation.
    pub fn schedule(&mut self, entry: TimerEntry) -> MeshResult<()> {
        // Fast scan over occupied slots only via u64 bitmask words.
        for (word_idx, &word_val) in self.occupied.iter().enumerate() {
            let mut bits = word_val;
            while bits != 0 {
                let tz = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let idx = word_idx * 64 + tz;
                if let Some(existing) = &mut self.slots[idx]
                    && existing.peer_slot == entry.peer_slot
                    && existing.action == entry.action
                {
                    let prev_deadline = existing.deadline_ns;
                    *existing = entry;
                    if self.min_deadline_ns == Some(prev_deadline)
                        && entry.deadline_ns > prev_deadline
                    {
                        self.recompute_min_deadline();
                    } else {
                        self.min_deadline_ns = Some(
                            self.min_deadline_ns
                                .map_or(entry.deadline_ns, |m| m.min(entry.deadline_ns)),
                        );
                    }
                    return Ok(());
                }
            }
        }

        // Find first free bit across occupancy words.
        let cap = self.slots.len();
        for (word_idx, word) in self.occupied.iter_mut().enumerate() {
            let inv = !*word;
            if inv == 0 {
                continue;
            }
            let tz = inv.trailing_zeros() as usize;
            let idx = word_idx * 64 + tz;
            if idx >= cap {
                break;
            }
            *word |= 1u64 << tz;
            self.slots[idx] = Some(entry);
            self.len += 1;
            self.min_deadline_ns = Some(
                self.min_deadline_ns
                    .map_or(entry.deadline_ns, |m| m.min(entry.deadline_ns)),
            );
            return Ok(());
        }
        Err(MeshError::PoolExhausted)
    }

    /// Cancels any armed timers matching `peer_slot`.
    pub fn cancel_peer(&mut self, peer_slot: u16) {
        let mut touched_min = false;
        for word_idx in 0..self.occupied.len() {
            let mut bits = self.occupied[word_idx];
            while bits != 0 {
                let tz = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let idx = word_idx * 64 + tz;
                if let Some(entry) = self.slots[idx]
                    && entry.peer_slot == peer_slot
                {
                    if self.min_deadline_ns == Some(entry.deadline_ns) {
                        touched_min = true;
                    }
                    self.slots[idx] = None;
                    self.occupied[word_idx] &= !(1u64 << tz);
                    self.len -= 1;
                }
            }
        }
        if touched_min {
            self.recompute_min_deadline();
        }
    }

    /// Polls up to `out.len()` expired timers (`deadline_ns <= now_ns`) without losing
    /// overflow entries: remaining expired timers stay armed for the next poll.
    pub fn poll_expired(&mut self, now_ns: u64, out: &mut [Option<TimerEntry>]) -> usize {
        if out.is_empty() || self.min_deadline_ns.is_none_or(|m| m > now_ns) {
            return 0;
        }
        let mut count = 0usize;
        let mut next_min: Option<u64> = None;

        for word_idx in 0..self.occupied.len() {
            let mut bits = self.occupied[word_idx];
            while bits != 0 {
                let tz = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let idx = word_idx * 64 + tz;
                let Some(entry) = self.slots[idx] else {
                    continue;
                };
                if entry.deadline_ns <= now_ns && count < out.len() {
                    out[count] = Some(entry);
                    count += 1;
                    self.slots[idx] = None;
                    self.occupied[word_idx] &= !(1u64 << tz);
                    self.len -= 1;
                } else {
                    next_min =
                        Some(next_min.map_or(entry.deadline_ns, |m| m.min(entry.deadline_ns)));
                }
            }
        }
        self.min_deadline_ns = next_min;
        count
    }

    fn recompute_min_deadline(&mut self) {
        let mut next_min: Option<u64> = None;
        for (word_idx, &word_val) in self.occupied.iter().enumerate() {
            let mut bits = word_val;
            while bits != 0 {
                let tz = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if let Some(entry) = self.slots[word_idx * 64 + tz] {
                    next_min =
                        Some(next_min.map_or(entry.deadline_ns, |m| m.min(entry.deadline_ns)));
                }
            }
        }
        self.min_deadline_ns = next_min;
    }
}
