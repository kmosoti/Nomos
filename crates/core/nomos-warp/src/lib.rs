//! # nomos-warp
//!
//! **Hexagon: domain service.** Dependency and execution-graph engine.
//!
//! Compiles Variances and Obligations, with the Canon's relationships,
//! capabilities, and policy, into an Action DAG: dependency resolution, cycle
//! detection, topological ordering, `requires` / `after` / `on_change` edges,
//! conflict keys, and execution-frontier calculation.
//!
//! Implemented, milestone `04-warp-kernel`:
//! - [`graph`]    — vertices from Assessments, edges from the Canon, one
//!   deterministic order, one witness per cycle
//! - [`frontier`] — per-edge and per-group resolution, the Ready set
//! - [`select`]   — greedy conflict-aware selection over the reserved set
//!
//! Added, milestone `05-transition-kernel`:
//! - owed vertices for pending Obligations, Ready when their group is Disabled
//! - [`budget`]   — failure-domain budgets over admission (N9)
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

pub mod budget;
pub mod frontier;
pub mod graph;
pub mod select;

#[cfg(kani)]
mod verification;
