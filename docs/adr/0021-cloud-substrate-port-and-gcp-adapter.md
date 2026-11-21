# ADR 0021: Cloud Substrate Port, GCP Adapter, Predictive Governor, and Headless Loom

- **Status.** Accepted
- **Date.** 2026-11-21

## Context

Phase 3 introduces `nomos-loom` as the single fleet coordinator alongside optional cloud substrate provisioning and host pressure control. Cloud control planes must obey the hexagonal boundary ([ADR 0000](0000-foundations.md)) without leaking transport clients or personal infrastructure topology into `nomos-core` or `nomos-app`. Pressure governance must execute deterministically inside `nomos-core` under `crates/core/PURITY.toml` ([ADR 0016](0016-core-purity.md)), and `nomos-loom` must stay headless while allowing prebuilt frontend assets via `--web-assets-dir`.

## Decision

### 1. Add the `nomos-cloud` Driven Port

`crates/ports/nomos-cloud` defines config-driven cloud provisioning contracts over `nomos-core` (`Digest`, `Generation`) and serializable configuration descriptors:

- `TopologySpec` declares desired project ID, region, zone, pure-IPv6 subnetwork Classless Inter-Domain Routing (CIDR) ranges (using documentation defaults such as `2001:db8::/32` and `fd00:db8::/64`), VM node specs, and firewall rules strictly from operator input, rejecting empty fields or IPv4-only assumptions.
- `CloudAuthConfig`, `AuthMode` (Application Default Credentials, known as ADC, or service account impersonation), and metadata-only `AuthPosture` forbid exposing raw OAuth bearer tokens in status output or errors (invariant N8).
- `CloudPlan`, `DriftItem`, `SpotBatchConfig` (`e2-standard-4`, `scale_to_zero = true`), `BatchJobHandle`, and `CloudSubstrate` define drift planning, convergent execution, and ephemeral Batch execution.

### 2. Add `crates/adapters/nomos-cloud-gcp`

`crates/adapters/nomos-cloud-gcp` implements `nomos-cloud` using `google-cloud-auth` 1.19 (ADC and service-account impersonation) and `reqwest` 0.13 over pure-IPv6 Representational State Transfer (REST) endpoints:

- Evaluates `TopologySpec` diffs against live Compute Engine subnets, firewall rules, and VM instances before applying mutations.
- Polls Google Cloud Platform (GCP) long-running operations (LRO) deterministically until `DONE`, failing closed on HTTP errors, operation errors, or timeout bounds.
- Submits and cleans up scale-to-zero `e2-standard-4` Spot Batch jobs against caller-configured API base URLs so integration suites verify full HTTP, LRO, and JSON round-trips against local `[::1]` mock servers.

### 3. Fold Pure `#![no_std]` PSI Governor Into `nomos-core` and `nomos-app`

`crates/core/nomos-core/src/governor.rs` defines pure `#![no_std]`, zero-dependency deterministic Pressure Stall Information (PSI) and cgroup resource control over `ServiceClass` (`Control`, `Interactive`, `Batch`, `Background`), `PressureTier` (`Normal`, `Elevated`, `High`, `Critical`), integer basis-point `ResourceSample` telemetry, `GovernorForecast`, `GovernorDirective`, and `ServiceGovernor`. `nomos-app` exposes governor admission and shedding orchestration to composition roots.

### 4. Implement Headless `crates/bin/nomos-loom`

`crates/bin/nomos-loom` wires `nomos-app`, `nomos-cipher-age-vtpm`, `nomos-mesh-wireguard`, `nomos-protocol-quic`, and `nomos-cloud-gcp`:

- Retains CLI commands (`compile`, `trace`, `enforce`) and adds headless `serve` over pure-IPv6 HTTP and pinned QUIC RPC.
- Hosts a SQLite write-ahead log (WAL) credential and workload lease broker storing only BLAKE3 token hashes, enforcing TTL and active lease caps, and loading operator credentials from `$CREDENTIALS_DIRECTORY`.
- Exposes `/health`, `/v1/leases`, `/v1/governor`, `/v1/governor/sample`, `/v1/telemetry`, and `/v1/events` Server-Sent Events (SSE), plus optional static asset hosting from `--web-assets-dir` with zero bundled WebAssembly (WASM) build artifacts.

## Consequences

- `nomos-core` preserves 100% `PURITY.toml` compliance (`#![no_std]`, zero external crates).
- `nomos-app` depends solely on `nomos-core` and `ports/`, keeping adapters isolated inside `crates/adapters/`.
- `nomos-loom` operates headless in minimal Debian and NixOS packages while supporting decoupled web UI bundles through `--web-assets-dir`.
