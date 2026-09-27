# ADR 0002: HashiCorp Vault as the preliminary Cipher adapter

- Status: Accepted
- Date: 2026-09-27

## Context

Cipher references resolve to plaintext only at the boundary where it is used
(spec §7). Nomos must not become a secrets database.

## Decision

Implement the `nomos-cipher` port first with `nomos-cipher-vault`. It resolves
`vault://…` references against HashiCorp Vault.

## Consequences

- Vault remains the source of truth for secret material.
- The adapter must keep plaintext out of errors, logs, Events and Trace output (invariant N8).
- Local-file and environment-backed development providers can be added as further adapters.
