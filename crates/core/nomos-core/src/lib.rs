//! # nomos-core
//!
//! **Hexagon: domain.** The shared Nomos model. Depends on nothing else in
//! the workspace and on no I/O, runtime, or transport. Owns reconciliation
//! semantics (ADR 0006): adapters supply evidence, core interprets it.
//!
//! Planned modules (none implemented yet):
//! - `canon`       — Canon (compiled desired intent) domain types
//! - `condition`   — Conditions: propositions reality is expected to satisfy
//! - `observation` — Observations: evidence from Substrate, with provenance
//! - `assessment`  — Assessment: `Satisfied`, `Variance`, or `Indeterminate`
//! - `trait_`      — Trait (value, provenance, observation time, stability)
//! - `cipher`      — Cipher references (never plaintext)
//! - `id`          — NodeID, CanonID, PlanID, ActionID, EventID
//! - `action`      — Action and its lifecycle
//! - `plan`        — Plan (immutable compiled artifact, generation/fencing)
//! - `event`       — Event and Event Log semantics
