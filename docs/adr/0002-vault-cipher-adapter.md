# ADR 0002: HashiCorp Vault as the Preliminary Cipher Adapter

- **Status.** Accepted
- **Date.** 2026-09-27

## Context

Cipher references resolve to plaintext only at the boundary where the value is used (spec §7). Nomos must not become a secrets database.

## Decision

Implement the `nomos-cipher` port first as `nomos-cipher-vault`. It resolves `vault://…` references against HashiCorp Vault.

What actually happens on resolve? An authenticated read over HTTPS against a Vault secrets engine, typically the key/value engine, using a short-lived token. The Cipher port models the primitive (resolve a reference to secret bytes at the point of use), so Vault is one implementation, not the concept. The exact mapping from `vault://mount/path/key` to Vault paths is a working definition until the adapter exists.

## Consequences

- Vault stays the source of truth for secret material.
- The adapter keeps plaintext out of errors, logs, Events, and Trace output (invariant N8).
- Local-file and environment-backed development providers can follow as further adapters.
- **Failure behavior.** If Vault is unreachable, resolution fails closed. The Action that needs the secret fails before it touches Substrate. It never proceeds with an empty or cached-plaintext value.
