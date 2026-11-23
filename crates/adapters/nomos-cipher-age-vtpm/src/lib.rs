//! Hybrid ML-KEM-768+X25519 and classical `age` X25519 + vTPM 2.0 cipher adapter
//! for [`nomos_cipher`] ([ADR 0018](../../../docs/adr/0018-age-vtpm-cipher-adapter.md)).
//!
//! All vault operations preserve invariant N8: plaintext secrets stay wrapped
//! in [`nomos_core::cipher::Secret`] with zeroizing scrubbers on transient
//! buffers, while status, sync plans, and [`nomos_cipher::SecretsPosture`]
//! surface metadata and digests only.

pub mod bounded_proc;
pub mod pq_hybrid;
mod pq_types;
pub mod sealer;
pub mod ssh_transport;
pub mod sync_plan;
pub mod transport;
pub mod vault;
pub mod vault_io;
pub mod vault_keys;

pub use pq_hybrid::{
    HybridIdentity, HybridRecipient, decrypt_hybrid, encrypt_hybrid, is_hybrid_ciphertext,
};
pub use sealer::{
    CredSealer, MockCredSealer, SystemdCredsSealer, VaultCipherMode, VaultKeyStorage,
};
pub use ssh_transport::{SshTargetTransport, posix_single_quote};
pub use sync_plan::{
    plan_and_execute_sync, plan_and_execute_sync_with_sealer, render_entry_payload,
    render_entry_payload_with_sealer,
};
pub use transport::{MemoryTargetTransport, probe_local_vtpm};
pub use vault::AgeVtpmVault;
