# ADR 0013: Substrate and Cipher Trust Boundaries

- **Status.** Proposed
- **Date.** 2026-09-28
- **Candidate.** `boundary-security` in the 2026-09-28 research snapshot (recommendations `substrate`, `cipher`)

## Context

Two boundaries carry most of the security weight. Substrate is where a privileged process touches the host. Cipher is where plaintext secrets exist at all. The research review found both stated more strongly than their mechanisms support. An observe-only interface prevents calls to mutate methods but cannot prove an adapter has no side effects (finding `trace-scope`). A syntactically valid path does not prevent symlink or rename races (finding `identity-path`). And a secret wrapper stops accidental logging, not every path to a sink (finding `secrets-scope`).

## Decision

Proposed. Linux mutation and secret support both come after milestone 1.

### 1. Trace Holds Only the Observe Capability

The Substrate port is split into observe and mutate capabilities, and Trace receives only the first. A compile-fail test is the evidence. N1 is stated over the managed-resource projection, and the residue that types cannot rule out is tested on Linux.

### 2. Paths Are Resolved at Use, Under the Boundary

A resource key is resolved to an OS identity at the moment of use, with kernel-enforced resolution rules where Linux provides them, for example `openat2` with `RESOLVE_BENEATH` and `RESOLVE_NO_SYMLINKS`. Authorization is a separate typed decision, never inferred from path shape. Mutation runs through a minimal privileged helper with a typed API (spec §55, Phase 5).

### 3. Secrets Have Named Sinks

Canon holds Cipher references, never secret bytes. A reference is resolved at the point of use through the Cipher port. The forbidden sinks are Events, Plans, Trace output, diagnostics and errors, logs, telemetry, and build artifacts. A protected file that the Canon names as a secret's destination is an intentional sink and is allowed. A secret's identity is never an unsalted digest of the secret. Rotation is driven by reference versions or provider metadata, never by reading plaintext to compare it.

### 4. Package-Manager Invocation Is Undecided

The package manager is `package`'s only interface. AGENTS.md rule 5 forbids shell execution in Substrate. A direct `execve` with a fixed argument vector is not a shell, and it is also not a native API. This ADR records the question and does not answer it. It is decided before the `package` resource is implemented.

### Acceptance Criteria

- **Milestone 1 PR 2.** Compile-fail tests for §1, and for the secret wrapper passing through a prohibited serialization path.
- **Linux adapter, `substrate-contract`.** Denied reads, symlink races, directory replacement, aliases, and foreign writers on a disposable Linux machine. No unauthorized mutation, and no false Absent.
- **Phase 6, `secret-nondisclosure`.** Sentinel secrets and their common encodings are absent from every forbidden sink across success and failure paths.

## Consequences

- The Trace-with-secrets question of spec §62 is answered by §3 when this ADR is accepted: Trace does not resolve secrets into any sink it writes, and an observation that would need plaintext is assessed Indeterminate unless a provider can compare without exposing it.
- **Revisit trigger.** Reopen when §4 is decided, or if a resource needs a capability that the observe and mutate split cannot express.
