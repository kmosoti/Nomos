//! # nomos-cipher-vault
//!
//! **Driven adapter** for the `nomos-cipher` port, backed by HashiCorp Vault.
//!
//! Resolves `vault://<mount>/<path>/<key>` Cipher references. Nomos does not
//! store secrets; Vault remains the source of truth.
