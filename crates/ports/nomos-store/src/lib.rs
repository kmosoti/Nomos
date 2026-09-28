//! # nomos-store
//!
//! **Driven port.** Persistence contracts: append-only Event Log,
//! materialized state, Action idempotency keys, Plans and Canons.
//!
//! Implemented, milestone `05-transition-kernel`: [`EventLog`], the
//! append-only log the transition kernel's snapshot is a fold of. Its Event
//! type is the application's, so the port is generic over it.
//!
//! Adapters: none yet — the storage engine is pending the persistence
//! ablation (redb vs SQLite vs LMDB-family). The transition kernel's tests
//! use an in-memory log with injectable faults.

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
