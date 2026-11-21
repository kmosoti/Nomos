//! Fixed-allocation boundary contracts shared between the Sans-I/O tunnel core and async shell.

pub mod buf_pool;
pub mod descriptors;
pub mod key_table;
pub mod ring;

pub use buf_pool::{BufHandle, BufPool, GSO_SUPER_BYTES, MAX_BATCH_SPLIT, SLOT_BYTES};
pub use descriptors::{
    CipherDirection, CipherTask, CoreOutput, DecryptedRx, EcnCodepoint, PacketLayout,
};
pub use key_table::{KeyTable, KeyTableUpdate, SessionKeys};
pub use ring::{FixedRing, SliceWriter};
