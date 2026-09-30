//! # nomos-substrate
//!
//! **Driven port.** Evidence and effects at the operating-system boundary:
//! `observe` returns Observations and `apply` returns effect receipts.
//! Assessment, planning, and verification are core semantics (ADR 0006), so
//! no adapter decides what satisfaction means.
//!
//! The port is two capabilities (ADR 0013 §1, ADR 0006 note). [`Observe`]
//! reads; [`Mutate`] changes. Trace is handed an `Observe` and nothing else,
//! so a mutation from Trace does not compile (N1). Both are synchronous and
//! typed: a transport failure while observing is a failed collection, which
//! core assesses as Indeterminate, and a lost receipt is a deadline that
//! passes in the kernel.
//!
//! Adapters: `nomos-substrate-linux`, `nomos-substrate-mock`.

use nomos_core::effect::{Apply, Receipt};
use nomos_core::observation::Observation;
use nomos_core::resource::{Digest, ResourceKey};

/// The observe capability: evidence about resources, never a verdict.
pub trait Observe {
    /// One Observation or more per resource in `resources`, each of the
    /// resource's family. A resource that cannot be read yields an
    /// Observation whose collection failed, never an absent resource; a
    /// family the adapter does not serve yields one whose collection failed
    /// as `Unsupported`.
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation>;
}

/// The mutate capability: performs one execution and reports on it.
pub trait Mutate {
    /// Performs `request` and returns what is known of it, in order. An
    /// adapter performs an execution at most once per idempotency key: a
    /// request whose key it has seen returns the receipts it returned then,
    /// and changes nothing (fencing-and-idempotency.md). The effect can
    /// cause no change after `request.settle_by`.
    fn apply(&mut self, request: &Apply) -> Vec<Receipt>;
}

/// Where an adapter gets the bytes an exact file requirement names by
/// digest ([ADR 0017](../../../../docs/adr/0017-local-store.md)). The
/// composition root backs it with the Cell's content store. An adapter
/// checks the digest of what it is given and treats a mismatch as absence.
pub trait ContentSource {
    /// The bytes whose SHA-256 digest is `digest`, if the source has them.
    fn bytes(&self, digest: &Digest) -> Option<Vec<u8>>;
}
