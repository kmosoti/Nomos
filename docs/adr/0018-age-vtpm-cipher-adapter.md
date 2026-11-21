# ADR 0018: Age and vTPM Cipher Adapter

- **Status.** Accepted
- **Date.** 2026-11-21

## Context

Nomos already defines the `nomos_core::cipher::Secret<T>` wrapper (invariant N8: Cipher values never appear in Events, Plans, Trace output, errors, or logs) and the `nomos-cipher` driven port ([ADR 0002](0002-vault-cipher-adapter.md)). Phase 1 needs a masterless, single-operator cipher backend alongside the planned HashiCorp Vault adapter so nodes can store encrypted secrets on disk, plan idempotent remote delivery, and audit Trusted Platform Module (TPM) 2.0 sealing without external secret infrastructure.

Porting a hardcoded operator layout would violate portability: fleet topologies, node hostnames, and IPv6 addresses vary per deployment, and embedding node identifiers or addresses inside the cipher crate couples domain policy to a single environment.

## Decision

### 1. Expand the `nomos-cipher` Driven Port

`crates/ports/nomos-cipher` defines the transport-neutral secret vault and delivery contract over `nomos_core::cipher::Secret`:

- `CipherRef` identifies a named secret reference and optional delivery target constraint.
- `SecretPolicy` and `SecretPolicyEntry` declare allowed secret keys, delivery modes, destination paths, POSIX ownership and permissions, and systemd credential names purely as configuration data.
- `NodeTarget` describes each destination host (`name`, IPv6/network `address`, transport endpoint or port, and staging paths) supplied by caller configuration, with zero hardcoded node names or addresses.
- `SecretMetadata`, `VaultStatus`, `SyncPlan`, `SyncAction`, and `SyncTargetState` expose non-secret metadata only; plaintext values cross the port solely inside `Secret<Vec<u8>>` or `Secret<String>` and are zeroized on drop.
- `SecretsPosture` reports host and vault readiness metadata: vault reachability, encrypted entry count, policy coverage, virtual Trusted Platform Module (vTPM) readiness (`systemd-creds has-tpm2`), and shared library availability (`libtss2-esys.so.0`, `libtss2-tcti-device.so.0`), never returning secret bytes or keys.
- `CipherVaultPort` and `CipherSyncPort` define typed async/sync operations (`init`, `status`, `get_secret`, `set_secret`, `delete_secret`, `plan_sync`, `apply_sync`, `audit_vtpm`) returning typed `CipherError` values that redact all secret material.

### 2. Add `crates/adapters/nomos-cipher-age-vtpm`

`crates/adapters/nomos-cipher-age-vtpm` implements `nomos-cipher` using `age` X25519 encryption, `blake3` integrity digests, `ring` cryptographic primitives, `zeroize` plaintext buffers, and `systemd-creds` / TSS2 checks:

- **Encrypted vault storage.** Identity keys and encrypted entry payloads live under an explicit vault directory path configured by the caller, validated against path traversal, and persisted atomically.
- **Data-driven sync planning.** `SyncPlan` diffs desired `SecretPolicy` entries against remote metadata digests and file permissions on each configured `NodeTarget`, staging via atomic rename (`install -m ...` or remote atomic file write) without mutating nodes when `confirm_apply` is false.
- **vTPM 2.0 posture verification.** Local and remote vTPM checks inspect `systemd-creds has-tpm2`, presence of `libtss2-esys.so.0` and `libtss2-tcti-device.so.0`, and metadata-only credential sealing posture without logging secrets.
- **Invariant N8 preservation.** `Secret<T>` wrappers forbid `Display` and `serde::Serialize`, `CipherError` contains keys and status only, and `SecretsPosture` exposes booleans, counts, and labels only.

### 3. Preserve Hexagonal Boundaries

`nomos-cipher-age-vtpm` depends solely on `nomos-core` and `nomos-cipher` ([ADR 0000](0000-foundations.md)). `nomos-cipher-vault` remains a parallel adapter implementing the `nomos-cipher` port contract (`UnsupportedBackend` until Vault transport lands). Only `bin/` composition roots (`nomos-cell`, `nomos-loom`) select and wire adapters.

## Consequences

- Operators and tests pass arbitrary `SecretPolicy` and `NodeTarget` sets (for example using `fd00:db8::/64` documentation prefixes) with zero embedded infrastructure constants.
- `nomos-core` acquires zero new dependencies; `Secret<T>` invariants apply uniformly across vaults.
- Automated `check-layers` verification requires `nomos-cipher-age-vtpm` to depend directly on `nomos-cipher` and never on another port or adapter.
