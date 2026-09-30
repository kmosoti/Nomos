//! # nomos-store
//!
//! **Driven port.** Persistence contracts: append-only Event Log,
//! materialized state, Action idempotency keys, Plans and Canons.
//!
//! Implemented, milestone `05-transition-kernel`: [`EventLog`], the
//! append-only log the transition kernel's snapshot is a fold of. Its Event
//! type is the application's, so the port is generic over it.
//!
//! Milestone `09-file-and-directory`: [`ContentStore`], the Cell's
//! digest-addressed store of the bytes file Conditions name
//! ([ADR 0017](../../../../docs/adr/0017-local-store.md)).
//!
//! Adapters: `nomos-store-fs`, the Cell's local disk. Loom's storage engine
//! is pending the persistence ablation (redb vs SQLite vs LMDB-family). The
//! transition kernel's tests use an in-memory log with injectable faults.

use nomos_core::resource::Digest;

/// The log refused an append; nothing of the batch was stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

impl std::fmt::Display for Full {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the Event Log cannot accept an append")
    }
}

impl std::error::Error for Full {}

/// An append-only sequence of Events (N7). There is no update and no
/// delete: an Event appended stays at its position.
pub trait EventLog<E> {
    /// Appends `batch` atomically: every Event of it, or none (ADR 0012 §1).
    fn append(&mut self, batch: &[E]) -> Result<(), Full>;

    /// Every Event, in append order.
    fn events(&self) -> Vec<E>;
}

/// Why the content store could not do what was asked. Nothing was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentError {
    /// The bytes on disk do not have the digest they are stored under.
    Corrupt,
    /// The operating system refused, with this error text.
    Io(String),
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentError::Corrupt => f.write_str("stored content does not match its digest"),
            ContentError::Io(e) => write!(f, "content store: {e}"),
        }
    }
}

impl std::error::Error for ContentError {}

/// A store of immutable blobs addressed by their SHA-256 digest (ADR 0017
/// §2). An entry never changes once stored.
pub trait ContentStore {
    /// Stores `bytes` and returns their digest. Storing bytes already
    /// present changes nothing.
    fn put(&mut self, bytes: &[u8]) -> Result<Digest, ContentError>;

    /// The bytes stored under `digest`: `Ok(None)` when there are none, and
    /// `Err(Corrupt)` when what is stored does not have that digest.
    fn get(&self, digest: &Digest) -> Result<Option<Vec<u8>>, ContentError>;
}

/// A value an Event Log can hold on disk: bytes out and bytes in (ADR 0017
/// §3). The application implements it for what it journals; an adapter
/// stores the bytes without interpreting them.
pub trait Record: Sized {
    /// The value's bytes.
    fn encode(&self) -> Vec<u8>;

    /// The value `bytes` encode, or `None` when they encode none.
    fn decode(bytes: &[u8]) -> Option<Self>;
}
