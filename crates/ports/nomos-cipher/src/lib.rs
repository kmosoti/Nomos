//! # nomos-cipher
//!
//! **Driven port.** Resolves a Cipher reference (e.g.
//! `vault://production/database/password`) to secret bytes at the boundary
//! where the plaintext is required.
//!
//! Plaintext must never reach Plan hashes, Events, Trace output, errors,
//! logs or telemetry (invariant N8).
//!
//! Adapters: `nomos-cipher-vault`.
