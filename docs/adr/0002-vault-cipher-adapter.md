# ADR 0002: HashiCorp Vault as the preliminary Cipher adapter

- **Status.** Accepted
- **Date.** 2026-09-27

## Context

Cipher references resolve to plaintext only at the boundary where the value is used (spec §7). Nomos must not become a secrets database.

## Decision

Implement the `nomos-cipher` port first as `nomos-cipher-vault`. It resolves `vault://…` references against HashiCorp Vault.

## Consequences

- Vault stays the source of truth for secret material.
- The adapter keeps plaintext out of errors, logs, Events, and Trace output (invariant N8).
- Local-file and environment-backed development providers can follow as further adapters.
