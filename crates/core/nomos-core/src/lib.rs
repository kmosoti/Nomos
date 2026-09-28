//! # nomos-core
//!
//! **Hexagon: domain.** The shared Nomos model. Depends on nothing else in
//! the workspace and on no I/O, runtime, or transport. Owns reconciliation
//! semantics (ADR 0006): adapters supply evidence, core interprets it.
//!
//! Implemented, milestone `03-assessment-kernel`, for the file resource family:
//! - [`resource`]    — validated resource paths and content digests
//! - [`condition`]   — Conditions: propositions reality is expected to satisfy
//! - [`observation`] — Observations: evidence from Substrate, with provenance
//! - [`assessment`]  — Assessment: `Satisfied`, `Variance`, or `Indeterminate`,
//!   and the per-Condition Report the Plan reads Variances from
//! - [`cipher`]      — the Secret wrapper; Cipher references come later
//!
//! Planned modules (none implemented yet):
//! - `canon`       — Canon (compiled desired intent) domain types
//! - `trait_`      — Trait (value, provenance, observation time, stability)
//! - `id`          — NodeID, CanonID, PlanID, ActionID, EventID
//! - `action`      — Action and its lifecycle
//! - `plan`        — Plan (immutable compiled artifact, generation/fencing)
//! - `event`       — Event and Event Log semantics
#![no_std]
// Core purity (ADR 0016, `crates/core/PURITY.toml`): a panic is not an
// Assessment. Failure is a value in the domain; these lints keep it one.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

pub mod assessment;
pub mod cipher;
pub mod condition;
pub mod observation;
pub mod resource;

#[cfg(kani)]
mod verification;
