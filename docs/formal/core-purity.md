# Core Purity

The contract every function in `crates/core/` keeps, what enforces each clause, and what nothing enforces. The decision is [ADR 0016](../adr/0016-core-purity.md); the kernel contract it serves is [ADR 0006](../adr/0006-kernel-contract.md).

## The Contract

A core function is pure: its result is a function of its arguments.

$$\mathrm{step}(\mathit{snapshot}, \mathit{input}) = \mathit{decision}$$

$$\mathit{decision} = (\mathit{next\_snapshot}, \mathit{events}, \mathit{effect\_requests})$$

An `EffectRequest` is data. It names an operation the application layer may perform through a port; the core never performs it, never waits for it, and learns its outcome only as a later `input`. The clock, when a decision needs it, arrives as a field of `input`. Randomness never arrives, because nothing in the kernel needs it (rule 7).

The following are not available to a core function, by construction or by policy:

| Ambient input | Why it is excluded | Enforced by |
| --- | --- | --- |
| Clock | Two runs of one decision would differ; timeouts become unreproducible | `#![no_std]` removes `std::time`; the `clock` deny class |
| Environment variables | Configuration by side channel | `#![no_std]` removes `std::env` |
| Filesystem | Observations come through Substrate with provenance, or not at all | `#![no_std]` removes `std::fs`; the `filesystem` deny class |
| Network | Same | `#![no_std]` removes `std::net`; the `network` deny class |
| Process identity, working directory, locale | Host-specific values would reach a Plan | `#![no_std]` removes `std::process` and `std::env`; the `process` deny class |
| Randomness | Planning would not be deterministic (N12, rule 7) | The `randomness` deny class; nothing in `core` or `alloc` supplies it |
| Threads and async runtimes | Scheduling order would become an input | `#![no_std]` removes `std::thread`; the `async-runtime` deny class |
| Global mutable state | A hidden argument | `unsafe` forbidden by the workspace lints, so no `static mut`; interior mutability in a global is reviewable and rare |
| Logging | A side effect and a plaintext hazard (rule 6) | The `logging` deny class |
| Panics | A panic is not an Assessment; failure is a value | `clippy::unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` denied |
| Build scripts | Arbitrary code at compile time | `check-core-purity` rejects a `custom-build` target |

## What Enforces It

| Mechanism | Sees | Does not see |
| --- | --- | --- |
| `#![no_std]` on every core crate | Every `std` path, at compile time | A `no_std` crate that reaches the operating system through `libc` or a runtime |
| `crates/core/PURITY.toml` and `cargo xtask check-core-purity` | Declared and resolved dependencies under all features, denied classes, feature shapes, build scripts, the crate-root attributes, the workspace lints | An effect reached through a function pointer a caller passes in; a dependency not in a deny class and wrongly allowlisted |
| Workspace lints | `unsafe` | Everything else |
| Panic lints | `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` outside tests | Arithmetic overflow, slice indexing, and other implicit panics, which need `clippy::indexing_slicing` and `clippy::arithmetic_side_effects`, not yet adopted |
| Review | The rest | |

Dev-dependencies are exempt from the policy. They build test binaries; the artifact a composition root links has none of them.

A denied class is absolute. A crate in one is rejected wherever the resolved graph reaches it, as a declared dependency or a transitive one, and no `[allow]` entry or `transitive` list admits it. A policy that names such a crate under `[allow]` contradicts its own `[deny]` and is itself rejected.

## What This Does Not Establish

- **Purity is not correctness.** A pure `step` can be wrong in every way the invariants forbid. The kernels' tests, semantic mutants, and models address that; this page addresses only that the inputs are the arguments.
- **`no_std` is not the absence of effects.** A caller can pass a closure that reads the clock. The kernel's port shapes, when written, take data, not closures, and that is a review item until a type makes it one.
- **The policy sees names, not behavior.** A crate named nothing in the deny classes that opens a socket would pass the class check and fail only the allowlist. The allowlist is therefore the real gate, and it is empty.
- **Implicit panics remain.** Indexing and arithmetic can panic in a `no_std` crate. The two lints that catch them are listed above as not adopted; the Assessment Kernel milestone decides.

## Interaction With the Layers

`check-layers` keeps workspace adapters out of the core. `check-core-purity` keeps registry effects out of it. Neither replaces the other: an adapter is a workspace crate the layer check sees, and `tokio` is a registry crate only the purity check sees. Both run on the required path.
