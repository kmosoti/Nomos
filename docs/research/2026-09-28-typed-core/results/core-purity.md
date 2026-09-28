# Result: core-purity

- **Experiment.** `core-purity`, milestone `02-verification-foundation`.
- **Question.** Can the core crates be held to purity by the compiler and a policy, without a review reading every dependency?
- **Outcome.** Yes, for what the compiler and `cargo metadata` can see. Recorded in [ADR 0016](../../../adr/0016-core-purity.md); the contract and its gaps are in [core-purity.md](../../../formal/core-purity.md).

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1, edition 2024 |
| Crates | `nomos-core`, `nomos-canon`, `nomos-warp`, all documentation-only at the time |
| Harness | The `no_std` experiment on a scratch copy of each crate; `cargo xtask check-core-purity` over `tests/fixtures/core-purity/` |
| Receipt | `verification/receipts/2026-09-28-verification-foundation.ndjson`, `check-core-purity` |

## The no_std Experiment

Each crate root received `#![no_std]`, `extern crate alloc;`, and `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented)]` with `#![cfg_attr(test, allow(...))]` for the first three, after its module documentation. The attributes must follow the `//!` block; placing them before it is a compile error (E0753), which the first attempt produced and the record keeps.

| Check | Result |
| --- | --- |
| `cargo check --workspace --locked` | Passed for all three |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed |
| `cargo test --workspace --locked` | Passed; the crates have no tests, so this shows the test harness links under `no_std` with `alloc` |
| `std::env::var("X")` added to a core function | Compile error: `std` is not available |
| `Option::unwrap()` added to a non-test core function | Lint error `clippy::unwrap_used` |
| The same `unwrap()` in a `#[cfg(test)]` module | Allowed |
| `clippy.toml` with `disallowed-methods` under `crates/core/` | Found and applied from each core crate; also found when placed at the workspace root |

Dependencies checked against their manifests, not built: `petgraph` 0.8.3 builds without `std` through `default-features = false` (no separate `alloc` feature since 0.8.0; `graphmap`, `stable_graph`, and `matrix_graph` are re-added as needed, and `rayon`, `dot_parser`, and `quickcheck` pull `std`); `serde` with `default-features = false, features = ["alloc", "derive"]`; `serde_json` with `default-features = false, features = ["alloc"]`; `ciborium` 0.2.2 and `minicbor` 2.3.0 without `std`. None is added; the allowlist is empty.

## The Gate

`cargo xtask check-core-purity` reads `crates/core/PURITY.toml` and `cargo metadata` twice, `--no-deps` for declarations and `--all-features` for the resolved graph, and the crate roots and manifests. 21 tests: the real workspace conforms; the allowed fixture passes; a dev-dependency on a denied crate is exempt; restoring a valid manifest passes again; and one negative control per code:

| Code | Fixture case |
| --- | --- |
| `core-dependency-denied-class` | `rand-dep` (class `randomness`), `tokio-dep` (class `async-runtime`) |
| `core-dependency-not-allowlisted` | `unlisted-dep` (`itoa`, in no class) |
| `core-build-script` | `build-script` |
| `core-no-std-missing` | `no-std-dropped`, with a doc comment that still says `#![no_std]` |
| `core-dependency-features` | `default-features`, `extra-feature` |
| `core-transitive-dependency-not-allowlisted` | `transitive` (`serde` pulling `rand`, reported as reached through `serde`, class `randomness`) |
| `core-required-lint-missing` | `panic-lints-dropped`, one violation per missing lint |
| `core-lints-not-workspace` | `lints-not-workspace` |
| `core-crate-unlisted` | `unlisted-core-crate` |
| `purity-policy-names-unknown-crate` | `policy-names-unknown-crate` |
| `purity-policy-malformed` | `policy-malformed` |
| `purity-policy-missing` | The policy file deleted in the test |

The fixture vendors `serde`, `serde_derive`, `rand`, `tokio`, and `itoa` as empty path crates under `vendor/`, excluded from the workspace so they are not implicit members, so the check needs no network. One finding from building it: a path dependency inside the workspace root is an implicit workspace member, and the check treats workspace members as the layer checker's business. The fixture excludes `vendor/` for that reason, and the finding is recorded in the gate's module documentation.

## Negative Controls

The twelve rows above, each asserted by code, plus the positive control. A `rand-dep` case also produces a transitive violation on `nomos-canon`, which depends on `nomos-core`: correct, and asserted.

## Unchecked

- An effect reached through a caller-supplied function.
- Implicit panics from indexing and arithmetic; the lints that catch them are not adopted (ADR 0016).
- Whether `no_std` costs anything once the crates hold code; they hold none.
- The policy on a `no_std` crate that binds the operating system by a name outside the deny classes: the allowlist catches it only because it is empty.

## Decision

ADR 0016: `no_std` and the panic lints on all three core crates; the policy and the gate on the required path; Clippy's disallowed lists not adopted as redundant under `no_std`. The first allowlist entry is a reviewed verifier commit in the milestone that needs it.
