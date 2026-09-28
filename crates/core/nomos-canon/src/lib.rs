//! # nomos-canon
//!
//! **Hexagon: domain service.** The typed Canon authoring API and the
//! Canonical IR (ADR 0004): typed resource specifications built as sum types,
//! validation, deterministic encoding, content-derived `CanonID`, and
//! validated decoding through untrusted data-transfer objects.
//!
//! An authoring crate depends on this crate to build a `Canon` value and emit
//! the IR. Loom and Cell use only the decoder. This crate performs no I/O.
//!
//! Compilation from a given IR must be deterministic (invariant N12). The
//! reproducibility of the build that produces the IR is a separate obligation,
//! tested by the `build-hermeticity` grounding experiment.
