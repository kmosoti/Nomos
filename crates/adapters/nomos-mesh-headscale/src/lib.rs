//! # nomos-mesh-headscale
//!
//! **Driven adapter** for the `nomos-mesh` port, backed by Headscale (a
//! self-hosted Tailscale control server).
//!
//! Maps mesh operations onto Headscale: pre-auth keys for Cell enrollment,
//! node registration and expiry, tag-based ACLs, and tailnet addressing for
//! Cell → Loom connectivity.
