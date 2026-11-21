//! Fixed-slab buffer pool for single packets and GSO super-buffers.

use nomos_mesh::{MeshError, MeshResult};

/// Single encrypted/decrypted packet slot capacity (bytes).
pub const SLOT_BYTES: usize = 2048;

/// GSO super-buffer slot capacity holding coalesced segments (bytes).
pub const GSO_SUPER_BYTES: usize = 16_384;

/// Compact index referencing a slot inside [`BufPool`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufHandle {
    index: u16,
    gso: bool,
}

impl BufHandle {
    /// Slot index within the corresponding slab.
    pub const fn index(self) -> u16 {
        self.index
    }

    /// True if this handle references a [`GSO_SUPER_BYTES`] super-buffer.
    pub const fn is_gso(self) -> bool {
        self.gso
    }
}

/// Pre-allocated fixed-capacity buffer slab pool.
///
/// Steady-state acquisition and release swap pre-allocated stack indices with
/// zero heap allocation or deallocation.
#[derive(Debug)]
pub struct BufPool {
    pkt_slab: Vec<u8>,
    pkt_free: Vec<u16>,
    pkt_capacity: usize,
    gso_slab: Vec<u8>,
    gso_free: Vec<u16>,
    gso_capacity: usize,
}

impl BufPool {
    /// Allocates a pool with `packet_slots` normal buffers and `gso_slots` super-buffers.
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

        let gso_slab = vec![0u8; gso_slots * GSO_SUPER_BYTES];
        let mut gso_free = Vec::with_capacity(gso_slots);
        for i in (0..gso_slots).rev() {
            gso_free.push(i as u16);
        }

        Ok(Self {
            pkt_slab,
            pkt_free,
            pkt_capacity: packet_slots,
            gso_slab,
            gso_free,
            gso_capacity: gso_slots,
        })
    }

    /// Acquires a single-packet [`SLOT_BYTES`] buffer handle without allocating.
    pub fn acquire(&mut self) -> MeshResult<BufHandle> {
        let index = self.pkt_free.pop().ok_or(MeshError::PoolExhausted)?;
        Ok(BufHandle { index, gso: false })
    }

    /// Acquires a [`GSO_SUPER_BYTES`] super-buffer handle without allocating.
    pub fn acquire_gso(&mut self) -> MeshResult<BufHandle> {
        let index = self.gso_free.pop().ok_or(MeshError::PoolExhausted)?;
        Ok(BufHandle { index, gso: true })
    }

    /// Returns a buffer handle back to the pool free stack without heap operations.
    pub fn release(&mut self, handle: BufHandle) {
        if handle.gso {
            if (handle.index as usize) < self.gso_capacity && !self.gso_free.contains(&handle.index)
            {
                self.gso_free.push(handle.index);
            }
        } else if (handle.index as usize) < self.pkt_capacity
            && !self.pkt_free.contains(&handle.index)
        {
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

    /// Splits the packet slab into disjoint mutable slot slices for zero-copy parallel AEAD.
    pub(crate) fn split_pkt_slots_mut(&mut self) -> Vec<Option<&mut [u8]>> {
        self.pkt_slab
            .as_chunks_mut::<SLOT_BYTES>()
            .0
            .iter_mut()
            .map(|chunk| Some(chunk.as_mut_slice()))
            .collect()
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
