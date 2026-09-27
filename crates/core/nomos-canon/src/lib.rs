//! # nomos-canon
//!
//! **Hexagon: domain service.** Canon compilation pipeline:
//! parser → typed AST → validation → canonical IR → content-derived CanonID.
//!
//! Compilation must be deterministic for identical inputs (invariant N12).
