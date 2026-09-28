//! # nomos-app
//!
//! **Hexagon: application layer.** Use cases that orchestrate the domain
//! through driven ports, and the driving ports exposed to the outside world:
//!
//! - `compile` — validate a Canon artifact and report its `CanonID`
//! - `trace`   — non-mutating evaluation of Assessments (invariant N1)
//! - `enforce` — reconcile reality toward Canon
//! - `target`  — select Cells by Trait predicates (Loom)
//! - `traits`  — discover local Traits (Cell)
//! - `events`  — inspect the Event Log
//!
//! Depends only on `nomos-core`, domain services and port crates — never on
//! adapters.
