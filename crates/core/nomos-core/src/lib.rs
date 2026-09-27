//! # nomos-core
//!
//! **Hexagon: domain.** The shared Nomos model. Depends on nothing else in
//! the workspace and on no I/O, runtime, or transport.
//!
//! Planned modules:
//! - `canon`   — Canon (desired state) domain types
//! - `trait_`  — Trait (value, provenance, observation time, stability)
//! - `cipher`  — Cipher references (never plaintext)
//! - `id`      — NodeID, CanonID, PlanID, ActionID, EventID
//! - `action`  — Action and its lifecycle
//! - `plan`    — Plan (immutable compiled artifact, generation/fencing)
//! - `event`   — Event and Event Log semantics
//! - `variance`— difference between desired and observed state
