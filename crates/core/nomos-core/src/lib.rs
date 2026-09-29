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
//! Implemented, milestone `05-transition-kernel`, the transition rules:
//! - [`action`]      — the Action lifecycle and verification (N6, N10)
//! - [`plan`]        — Plan identity and the generation fence (N5)
//! - [`effect`]      — idempotency keys, effect requests, receipts, settlement
//!
//! Working definition, experiment `controller-composition` (ADR 0008, Proposed):
//! - [`footprint`]   — what a controller writes and relies on, and the
//!   composition check over footprints
//!
//! Planned modules (none implemented yet):
//! - `canon`       — Canon (compiled desired intent) domain types
//! - `trait_`      — Trait (value, provenance, observation time, stability)
//! - `id`          — NodeID, CanonID, PlanID, ActionID, EventID
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

pub mod action;
pub mod assessment;
pub mod cipher;
pub mod condition;
pub mod effect;
pub mod footprint;
pub mod observation;
pub mod plan;
pub mod resource;

#[cfg(kani)]
mod verification;
