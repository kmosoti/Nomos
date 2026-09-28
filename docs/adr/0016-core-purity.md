# ADR 0016: Core Purity

- **Status.** Accepted
- **Date.** 2026-09-28
- **Provenance.** [ADR 0006](0006-kernel-contract.md) §1 states the kernel is pure. The `core-purity` experiment of the verification-foundation milestone, recorded under `docs/research/2026-09-28-typed-core/results/core-purity.md`, measured what that costs to enforce.

## Context

ADR 0006 says `step(snapshot, input) = decision` is a pure function and adapters supply evidence. The dependency rule keeps adapters out of `crates/core/`, and `cargo xtask check-layers` enforces it. Neither says anything about a registry crate. A core crate could take `tokio`, read the clock through `std::time`, or run a build script, and every check would pass. The purity ADR 0006 promises was documented, not established.

Purity here means one thing: a core function's result depends on its arguments and on nothing else. No clock, environment, filesystem, network, process identity, working directory, locale, randomness, threads, or global mutable state. Effects are requested as data, `EffectRequest` values in a `Decision`, and performed by the application layer through ports. The contract is written out in [docs/formal/core-purity.md](../formal/core-purity.md).

## Decision

### 1. Core Crates Are `no_std`

`nomos-core`, `nomos-canon`, and `nomos-warp` declare `#![no_std]` and use `alloc`. The experiment showed all three compile, lint, and test this way today, and that `std::env`, `std::time`, `std::fs`, and `std::net` become compile errors rather than review items. The compiler is the cheapest verifier available, and this is the one purity check it can run.

### 2. Panics Are Lint Errors

Each core crate denies `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::todo`, and `clippy::unimplemented` at its root, with the first three allowed under `cfg_attr(test, ...)`. A panic is not an Assessment; failure in the domain is a value. Tests keep `unwrap`.

### 3. Dependencies Are Allowlisted by Policy

`crates/core/PURITY.toml` lists the core crates, the external dependencies each may name with the feature shape it must use, the crates an allowed dependency may pull in transitively, and eight denied classes: async runtimes, network, filesystem, randomness, clocks, process and operating-system bindings, logging, and operating-system adapters. `cargo xtask check-core-purity` enforces it over `cargo metadata`'s declared and resolved graphs, rejects build scripts, and reads each crate root for the attributes of §1 and §2. Dev-dependencies are exempt, because test binaries are not the artifact a composition root links and property-test harnesses need `std`. The allowlist is empty: the first external dependency of a core crate arrives with the Assessment Kernel and its own review.

### 4. Clippy's Disallowed Lists Are Not Adopted

The experiment confirmed that `clippy.toml`'s `disallowed-methods` works from the core crates. It is not enabled, because under `no_std` every method it would name is already unavailable, and a second mechanism with the same coverage is maintenance without evidence. Reopen if a core crate ever needs `std`.

## Alternatives

- **Review only.** Rejected. ADR 0006 already relied on it, and nothing had been checked.
- **`no_std` without the policy file.** Rejected. `no_std` says nothing about a `no_std` randomness crate, a build script, or a dependency's default features pulling `std` back in.
- **The policy file without `no_std`.** Rejected. The allowlist cannot see `std::env`; the compiler can.
- **Per-crate Clippy configuration.** Not adopted; §4.

## Evidence

- All three core crates build, lint, and test under `#![no_std]` with `alloc` on the pinned toolchain; `std::env::var` in a core crate is a compile error; an `unwrap` in non-test code is a lint error.
- `check-core-purity`: 21 tests, one positive control, 14 fixture cases covering every code, on a vendored fixture workspace that needs no network; the real workspace conforms.
- `petgraph` 0.8 builds without `std` through `default-features = false` and is the likely first allowlist entry for `nomos-warp`, verified against its manifest, not yet added.

## Consequences

- A core crate gains an external dependency only through a reviewed edit to `PURITY.toml`, in its own commit (it is a verifier path under ADR 0015).
- Anything that needs the clock, the filesystem, or randomness lives outside `crates/core/`. The kernel receives time as an argument and randomness never.
- The check reads attributes and manifests; it does not compile. A `#![no_std]` crate that smuggles an effect through a function pointer supplied by a caller is the caller's effect, and outside this check.
- **Failure behavior.** A violation prints its code, crate, and dependency; a missing or malformed policy fails, it does not skip.
- **Revisit trigger.** The first allowlist entry, or the first core crate that needs `std`.
