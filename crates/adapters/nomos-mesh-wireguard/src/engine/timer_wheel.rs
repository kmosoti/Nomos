//! Monotonic pre-allocated timer wheel driven solely by injected `now_ns`.

use nomos_mesh::{MeshError, MeshResult};

/// Kind of scheduled peer maintenance action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerAction {
    /// Trigger WireGuard persistent keepalive packet for `peer_slot`.
    Keepalive,
    /// Trigger deterministic session key rotation for `peer_slot`.
    Rekey,
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

/// Fixed-capacity timing array allocating only once at construction.
#[derive(Debug)]
pub struct TimerWheel {
    slots: Vec<Option<TimerEntry>>,
}

impl TimerWheel {
    /// Pre-allocates `capacity` timer slots.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: vec![None; capacity],
        }
    }

    /// Arms or replaces a timer for `(peer_slot, action)` without heap allocation.
    pub fn schedule(&mut self, entry: TimerEntry) -> MeshResult<()> {
        for slot in &mut self.slots {
            if let Some(existing) = slot
                && existing.peer_slot == entry.peer_slot
                && existing.action == entry.action
            {
                *existing = entry;
                return Ok(());
            }
        }
        for slot in &mut self.slots {
            if slot.is_none() {
                *slot = Some(entry);
                return Ok(());
            }
        }
        Err(MeshError::PoolExhausted)
    }

    /// Polls all timers whose `deadline_ns <= now_ns` into `out` without allocating.
    pub fn poll_expired(&mut self, now_ns: u64, out: &mut [Option<TimerEntry>]) -> usize {
        let mut count = 0;
        for slot in &mut self.slots {
            if count >= out.len() {
                break;
            }
            if let Some(entry) = *slot
                && entry.deadline_ns <= now_ns
            {
                out[count] = Some(entry);
                count += 1;
                *slot = None;
            }
        }
        count
    }
}
