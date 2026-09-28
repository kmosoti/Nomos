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
//!
//! Implemented, milestone `06-canon-artifact`, to
//! [canon-ir.md](../../../docs/formal/canon-ir.md):
//! - [`model`]    — the validated `Canon`, its untrusted `RawCanon`, the one
//!   validator between them, and the authoring `CanonBuilder`
//! - [`value`]    — the restricted data model both encodings carry
//! - [`cbor`]     — deterministic CBOR, RFC 8949 §4.2.1
//! - [`jcs`]      — the JSON Canonicalization Scheme, RFC 8785
//! - [`artifact`] — strict decoding, readers, archival inspection,
//!   migration, and `CanonID`
//! - [`sha256`]   — the hash behind `CanonID`
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

pub mod artifact;
pub mod cbor;
pub mod jcs;
pub mod model;
pub mod sha256;
pub mod value;
