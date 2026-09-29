# ADR 0013: Substrate and Cipher Trust Boundaries

- **Status.** Proposed
- **Date.** 2026-09-28
- **Candidate.** `boundary-security` in the 2026-09-28 research snapshot (recommendations `substrate`, `cipher`)

## Context

Two boundaries carry most of the security weight. Substrate is where a privileged process touches the host. Cipher is where plaintext secrets exist at all. The research review found both stated more strongly than their mechanisms support. An observe-only interface prevents calls to mutate methods but cannot prove an adapter has no side effects (finding `trace-scope`). A syntactically valid path does not prevent symlink or rename races (finding `identity-path`). And a secret wrapper stops accidental logging, not every path to a sink (finding `secrets-scope`).

## Decision

Proposed. Linux mutation and secret support both come after the kernel milestones.

### 1. Trace Holds Only the Observe Capability

The Substrate port is split into observe and mutate capabilities, and Trace receives only the first. A compile-fail test is the evidence. N1 is stated over the managed-resource projection, and the residue that types cannot rule out is tested on Linux.

### 2. Paths Are Resolved at Use, Under the Boundary

A resource key is resolved to an OS identity at the moment of use, with kernel-enforced resolution rules where Linux provides them, for example `openat2` with `RESOLVE_BENEATH` and `RESOLVE_NO_SYMLINKS`. Authorization is a separate typed decision, never inferred from path shape. Mutation runs through a minimal privileged helper with a typed API (spec §55, Phase 5).

### 3. Secrets Have Named Sinks

Canon holds Cipher references, never secret bytes. A reference is resolved at the point of use through the Cipher port. The forbidden sinks are Events, Plans, Trace output, diagnostics and errors, logs, telemetry, and build artifacts. A protected file that the Canon names as a secret's destination is an intentional sink and is allowed. A secret's identity is never an unsalted digest of the secret. Rotation is driven by reference versions or provider metadata, never by reading plaintext to compare it.

### 4. Package-Manager Invocation Is Undecided

The package manager is `package`'s only interface. AGENTS.md rule 5 forbids shell execution in Substrate. A direct `execve` with a fixed argument vector is not a shell, and it is also not a native API. This ADR records the question and does not answer it. It is decided before the `package` resource is implemented.

### Acceptance Criteria

- **Milestone `03-assessment-kernel`.** Compile-fail tests for §1, and for the secret wrapper passing through a prohibited serialization path. *Partly met, 2026-09-28:* the secret wrapper has compile-fail cases for `Display`, `Serialize`, and an unused `expose`, and a sentinel test on its `Debug` output. The §1 case needs the observe and mutate split of the Substrate port, which does not exist yet; it moves to the milestone that writes the port shapes, `05-transition-kernel`, or to `07-substrate-conformance`.
- **Linux adapter, `substrate-contract`.** Denied reads, symlink races, directory replacement, aliases, and foreign writers on a disposable Linux machine. No unauthorized mutation, and no false Absent.
- **Phase 6, `secret-nondisclosure`.** Sentinel secrets and their common encodings are absent from every forbidden sink across success and failure paths.

## Note, 2026-09-29: Working Definitions for the First Linux Operation

Milestone `07-substrate-conformance` implements §1 and §2 for one resource, regular files beneath a root directory, against [substrate-contract.md](../formal/substrate-contract.md), written before the code. Paths resolve at use through `openat2` with `RESOLVE_BENEATH`, `RESOLVE_NO_SYMLINKS`, and `RESOLVE_NO_MAGICLINKS`, relative to a descriptor of the root, so no symbolic link is followed and nothing resolves outside it. The system call wrapper is `rustix`, which gives these calls a safe interface, because the workspace forbids `unsafe`. The adapter runs with the test's privileges: the privileged helper of §2 is still Phase 5. The acceptance criterion `substrate-contract` is run for symbolic links at the leaf and in a parent, a directory in place of a file, a foreign writer, and a denied read; a race in which a link is swapped in between two system calls of one execution is reasoned about in the contract and not provoked. This ADR stays Proposed: §3 and §4 have not been exercised.

## Note, 2026-09-29: Evidence From `07-substrate-conformance`

The acceptance criterion `substrate-contract` ran for the first operation, on one Linux host ([record](../research/2026-09-28-typed-core/results/substrate-contract.md)). Denied reads are failed collections, never absent files; symbolic links at the leaf, in a parent, and out of the root are refused; a directory and a named pipe in place of a file are unsupported; a foreign writer is seen by verification; a hard link planted at the temporary name is not written through. No unauthorized mutation and no false Absent occurred. Mutation testing found that an observation could block on a named pipe, now fixed. §1 and §2 hold as implemented for regular files; the ADR stays Proposed until §3 and §4 are exercised.

## Consequences

- The Trace-with-secrets question of spec §62 is answered by §3 when this ADR is accepted: Trace does not resolve secrets into any sink it writes, and an observation that would need plaintext is assessed Indeterminate unless a provider can compare without exposing it.
- **Revisit trigger.** Reopen when §4 is decided, or if a resource needs a capability that the observe and mutate split cannot express.
