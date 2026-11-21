//! Age X25519 + vTPM 2.0 cipher adapter for [`nomos_cipher`] ([ADR 0018](../../../docs/adr/0018-age-vtpm-cipher-adapter.md)).
//!
//! All vault operations preserve invariant N8: plaintext secrets stay wrapped
//! in [`nomos_core::cipher::Secret`] with [`zeroize`] scrubbers on transient
//! buffers, while status, sync plans, and [`nomos_cipher::SecretsPosture`]
//! surface metadata and digests only.
//!
//! Delivery planning and host probes consume caller-supplied
//! [`nomos_cipher::SecretPolicy`] and [`nomos_cipher::NodeTarget`] records with
//! zero hardcoded hostnames or network addresses.

pub mod ssh_transport;
pub mod sync_plan;
pub mod transport;
pub mod vault;
pub mod vault_io;

pub use ssh_transport::SshTargetTransport;
pub use sync_plan::{plan_and_execute_sync, render_entry_payload};
pub use transport::{MemoryTargetTransport, probe_local_vtpm};
pub use vault::AgeVtpmVault;
