//! Bounded pre-allocated ring queue and zero-alloc slice writer.

use nomos_mesh::{MeshError, MeshResult};

/// Fixed-capacity FIFO queue backed by a pre-allocated buffer (zero runtime allocation).
#[derive(Debug)]
pub struct FixedRing<T> {
    buf: Vec<Option<T>>,
    head: usize,
    len: usize,
}

impl<T> FixedRing<T> {
    /// Pre-allocates `capacity` slots once at startup.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity);
        buf.resize_with(capacity, || None);
        Self {
            buf,
            head: 0,
            len: 0,
        }
    }

    /// Current element count.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Maximum element capacity.
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Pushes `item` without heap allocation, returning [`MeshError::PoolExhausted`] if full.
    pub fn push(&mut self, item: T) -> MeshResult<()> {
        let cap = self.buf.len();
        if cap == 0 || self.len == cap {
            return Err(MeshError::PoolExhausted);
        }
        let tail = (self.head + self.len) % cap;
        self.buf[tail] = Some(item);
        self.len += 1;
        Ok(())
    }

    /// Pops the oldest element without heap allocation.
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let item = self.buf[self.head].take();
        self.head = (self.head + 1) % self.buf.len();
        self.len -= 1;
        item
    }
}

/// Cursor writing bytes sequentially into a caller-owned mutable slice without allocating.
pub struct SliceWriter<'a> {
    dst: &'a mut [u8],
    pos: usize,
}

impl<'a> SliceWriter<'a> {
    /// Wraps an existing byte slice at offset 0.
    pub fn new(dst: &'a mut [u8]) -> Self {
        Self { dst, pos: 0 }
    }

    /// Bytes written so far.
    pub const fn written(&self) -> usize {
        self.pos
    }

    /// Remaining writable bytes in slice.
    pub const fn remaining(&self) -> usize {
        self.dst.len().saturating_sub(self.pos)
    }

    /// Copies `src` bytes into the remaining buffer or returns [`MeshError::PoolExhausted`].
    pub fn write_bytes(&mut self, src: &[u8]) -> MeshResult<()> {
        let end = self
            .pos
            .checked_add(src.len())
            .ok_or(MeshError::PoolExhausted)?;
        if end > self.dst.len() {
            return Err(MeshError::PoolExhausted);
        }
        self.dst[self.pos..end].copy_from_slice(src);
        self.pos = end;
        Ok(())
    }

    /// Appends a big-endian `u16`.
    pub fn write_u16_be(&mut self, value: u16) -> MeshResult<()> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Appends a big-endian `u32`.
    pub fn write_u32_be(&mut self, value: u32) -> MeshResult<()> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Appends a little-endian `u32`.
    pub fn write_u32_le(&mut self, value: u32) -> MeshResult<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    /// Appends a little-endian `u64`.
    pub fn write_u64_le(&mut self, value: u64) -> MeshResult<()> {
        self.write_bytes(&value.to_le_bytes())
    }
}
