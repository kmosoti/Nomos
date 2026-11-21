//! Fixed-slab buffer pool with O(1) bitmap release and jumbo frame support.

use nomos_mesh::{MeshError, MeshResult};

/// Single encrypted/decrypted packet slot capacity supporting jumbo 9000B MTUs.
pub const SLOT_BYTES: usize = 9216;

/// GSO super-buffer slot capacity holding coalesced segments (bytes).
pub const GSO_SUPER_BYTES: usize = 65_536;

/// Maximum handles split simultaneously into disjoint mutable slices without allocation.
pub const MAX_BATCH_SPLIT: usize = 64;

/// Compact index referencing a slot inside [`BufPool`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufHandle {
    index: u16,
    gso: bool,
}

impl BufHandle {
    /// Internal zero-cost placeholder overwritten before indexing.
    pub(crate) const fn placeholder() -> Self {
        Self {
            index: 0,
            gso: false,
        }
    }

    /// Slot index within the corresponding slab.
    pub const fn index(self) -> u16 {
        self.index
    }

    /// True if this handle references a [`GSO_SUPER_BYTES`] super-buffer.
    pub const fn is_gso(self) -> bool {
        self.gso
    }
}

/// Pre-allocated fixed-capacity buffer slab pool with `O(1)` bitmap occupancy tracking.
///
/// Steady-state acquisition, release, and batch slicing perform zero heap allocations.
#[derive(Debug)]
pub struct BufPool {
    pkt_slab: Vec<u8>,
    pkt_free: Vec<u16>,
    pkt_in_use: Vec<u64>,
    pkt_capacity: usize,
    gso_slab: Vec<u8>,
    gso_free: Vec<u16>,
    gso_in_use: Vec<u64>,
    gso_capacity: usize,
}

impl BufPool {
    /// Allocates a pool with `packet_slots` jumbo buffers and `gso_slots` super-buffers.
    pub fn new(packet_slots: usize, gso_slots: usize) -> MeshResult<Self> {
        if packet_slots > u16::MAX as usize || gso_slots > u16::MAX as usize {
            return Err(MeshError::InvalidConfig(
                "pool slot count exceeds u16 handle range".into(),
            ));
        }
        let pkt_slab = vec![0u8; packet_slots * SLOT_BYTES];
        let mut pkt_free = Vec::with_capacity(packet_slots);
        for i in (0..packet_slots).rev() {
            pkt_free.push(i as u16);
        }
        let pkt_words = packet_slots.div_ceil(64).max(1);

        let gso_slab = vec![0u8; gso_slots * GSO_SUPER_BYTES];
        let mut gso_free = Vec::with_capacity(gso_slots);
        for i in (0..gso_slots).rev() {
            gso_free.push(i as u16);
        }
        let gso_words = gso_slots.div_ceil(64).max(1);

        Ok(Self {
            pkt_slab,
            pkt_free,
            pkt_in_use: vec![0u64; pkt_words],
            pkt_capacity: packet_slots,
            gso_slab,
            gso_free,
            gso_in_use: vec![0u64; gso_words],
            gso_capacity: gso_slots,
        })
    }

    /// Acquires a single-packet [`SLOT_BYTES`] buffer handle without allocating.
    pub fn acquire(&mut self) -> MeshResult<BufHandle> {
        let index = self.pkt_free.pop().ok_or(MeshError::PoolExhausted)?;
        set_bit(&mut self.pkt_in_use, index as usize);
        Ok(BufHandle { index, gso: false })
    }

    /// Acquires a [`GSO_SUPER_BYTES`] super-buffer handle without allocating.
    pub fn acquire_gso(&mut self) -> MeshResult<BufHandle> {
        let index = self.gso_free.pop().ok_or(MeshError::PoolExhausted)?;
        set_bit(&mut self.gso_in_use, index as usize);
        Ok(BufHandle { index, gso: true })
    }

    /// Returns a buffer handle back to the pool free stack in `O(1)` time without heap ops.
    ///
    /// Double-releases are ignored safely in `O(1)` via the occupancy bitmap.
    pub fn release(&mut self, handle: BufHandle) {
        let idx = handle.index as usize;
        if handle.gso {
            if idx < self.gso_capacity && clear_bit_if_set(&mut self.gso_in_use, idx) {
                self.gso_free.push(handle.index);
            }
        } else if idx < self.pkt_capacity && clear_bit_if_set(&mut self.pkt_in_use, idx) {
            self.pkt_free.push(handle.index);
        }
    }

    /// Total single-packet handles currently checked out.
    pub fn in_use_count(&self) -> usize {
        self.pkt_capacity - self.pkt_free.len()
    }

    /// Total GSO super-buffer handles currently checked out.
    pub fn gso_in_use_count(&self) -> usize {
        self.gso_capacity - self.gso_free.len()
    }

    /// Maximum byte capacity of `handle`'s underlying slab slice.
    pub fn slot_capacity(&self, handle: BufHandle) -> usize {
        if handle.gso {
            GSO_SUPER_BYTES
        } else {
            SLOT_BYTES
        }
    }

    /// Immutable slice backing `handle` up to `len` bytes.
    pub fn slice(&self, handle: BufHandle, len: usize) -> MeshResult<&[u8]> {
        let cap = self.slot_capacity(handle);
        if len > cap {
            return Err(MeshError::InvalidConfig(format!(
                "requested slice len {len} exceeds slot capacity {cap}"
            )));
        }
        let start = self.slot_offset(handle)?;
        let end = start + len;
        if handle.gso {
            Ok(&self.gso_slab[start..end])
        } else {
            Ok(&self.pkt_slab[start..end])
        }
    }

    /// Mutable full backing slice for `handle`.
    pub fn slice_mut(&mut self, handle: BufHandle) -> MeshResult<&mut [u8]> {
        let cap = self.slot_capacity(handle);
        let start = self.slot_offset(handle)?;
        let end = start + cap;
        if handle.gso {
            Ok(&mut self.gso_slab[start..end])
        } else {
            Ok(&mut self.pkt_slab[start..end])
        }
    }

    /// Splits up to [`MAX_BATCH_SPLIT`] distinct single-packet handles into disjoint
    /// mutable slices in `out` with zero heap allocations.
    pub fn split_single_batch_mut<'a>(
        &'a mut self,
        handles: &[BufHandle],
        out: &mut [Option<&'a mut [u8]>],
    ) -> MeshResult<()> {
        let n = handles.len();
        if n > MAX_BATCH_SPLIT || out.len() < n {
            return Err(MeshError::InvalidConfig(
                "batch split exceeds MAX_BATCH_SPLIT or output capacity".into(),
            ));
        }
        let mut order: [(u16, usize); MAX_BATCH_SPLIT] = [(u16::MAX, 0); MAX_BATCH_SPLIT];
        for (pos, &handle) in handles.iter().enumerate() {
            if handle.gso || (handle.index as usize) >= self.pkt_capacity {
                return Err(MeshError::InvalidConfig(
                    "invalid or GSO handle in single-packet batch split".into(),
                ));
            }
            order[pos] = (handle.index, pos);
        }
        order[..n].sort_unstable_by_key(|&(slot_idx, _)| slot_idx);
        for pair in order[..n].windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(MeshError::InvalidConfig(
                    "duplicate BufHandle in single-packet batch split".into(),
                ));
            }
        }

        let mut remaining: &mut [u8] = &mut self.pkt_slab;
        let mut cursor_slot: usize = 0;
        for &(slot_idx, orig_pos) in &order[..n] {
            let skip_slots = (slot_idx as usize) - cursor_slot;
            let (_, after_skip) = remaining.split_at_mut(skip_slots * SLOT_BYTES);
            let (slot_slice, tail) = after_skip.split_at_mut(SLOT_BYTES);
            out[orig_pos] = Some(slot_slice);
            remaining = tail;
            cursor_slot = (slot_idx as usize) + 1;
        }
        Ok(())
    }

    fn slot_offset(&self, handle: BufHandle) -> MeshResult<usize> {
        let idx = handle.index as usize;
        if handle.gso {
            if idx >= self.gso_capacity {
                return Err(MeshError::InvalidConfig("GSO handle out of bounds".into()));
            }
            Ok(idx * GSO_SUPER_BYTES)
        } else {
            if idx >= self.pkt_capacity {
                return Err(MeshError::InvalidConfig(
                    "packet handle out of bounds".into(),
                ));
            }
            Ok(idx * SLOT_BYTES)
        }
    }
}

#[inline]
fn set_bit(words: &mut [u64], idx: usize) {
    words[idx / 64] |= 1u64 << (idx % 64);
}

#[inline]
fn clear_bit_if_set(words: &mut [u64], idx: usize) -> bool {
    let word = &mut words[idx / 64];
    let mask = 1u64 << (idx % 64);
    let was_set = (*word & mask) != 0;
    *word &= !mask;
    was_set
}
