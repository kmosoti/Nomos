# AGENTS.md

Guidance for automated coding agents working in this repository. It applies to
any agent or tool. Human contributors should read
[CONTRIBUTING.md](CONTRIBUTING.md), which this file is consistent with.

## Project

Nomos is a host-state convergence and fleet-control system for Linux, written
in Rust. It compares desired state (Canon) with observed state, computes the
Variance, plans Actions as a dependency graph (Warp), applies them through the
OS boundary (Substrate), verifies the result, and records Events.

The repository is at the **skeleton stage**. Crates contain module docs only.
Do not add implementation unless the task asks for it.

## Source of truth

| Need | Read |
|---|---|
| Semantics, vocabulary, invariants | `docs/PROJECT-SPEC.md` |
| Crate layout and dependency rule | `docs/architecture/hexagon.md`, `docs/adr/0000-foundations.md` |
| Runtime behaviour | `docs/architecture/runtime.md` |
| Algorithms and proof obligations | `docs/formal/` |
| Past decisions | `docs/adr/` |

If a task conflicts with these documents, stop and report the conflict. Do not
silently diverge.

## Commands

The toolchain is pinned to Rust 1.98.1 in `rust-toolchain.toml`.

```sh
cargo check  --workspace
cargo fmt    --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test   --workspace
```

All four must pass before a change is complete.

## Layout

| Path | Layer | May depend on |
|---|---|---|
| `crates/core/` | domain | `core/` only (`nomos-core` depends on nothing) |
| `crates/ports/` | driven ports | `core/` |
| `crates/app/` | use cases | `core/`, `ports/` |
| `crates/adapters/` | driven adapters | `core/` and the one port they implement |
| `crates/bin/` | CLIs, composition roots | anything |

Workspace crates are referenced through `[workspace.dependencies]` in the root
`Cargo.toml`, using `name.workspace = true`.

## Rules

1. **Never break the dependency rule.** Adding an adapter dependency to `app/`,
   `core/` or `ports/` is always wrong.
2. **New crates, ports or changes to the rule require an ADR** in
   `docs/adr/`, with the next number.
3. **Use the spec's vocabulary exactly:** Canon, Trait, Cipher, Variance, Trace,
   Enforce, Event, Event Log. Do not introduce synonyms.
4. **Preserve invariants N1–N12** (spec §58). A change that touches one needs a
   test for it.
5. **No shell execution in Substrate.** Use native APIs, for example systemd
   over D-Bus.
6. **Never log, serialize or embed Cipher plaintext.**
7. **Determinism:** nothing feeding compilation or planning may depend on hash
   iteration order, randomness or wall-clock time.
8. `unsafe` code is forbidden by workspace lints.
9. Do not change the pinned toolchain, the license or CI without being asked.

## Documentation conventions

- Diagrams are **Mermaid only**, and must render on GitHub. Never use ASCII or
  box-drawing art.
- Math uses GitHub math syntax (`$...$`, `$$...$$`).
- The README is narrative. Technical detail goes under `docs/`.
- Update the relevant docs and index tables in the same change.

## Commits

- Imperative subject line, with the *why* in the body.
- Keep each change focused. Do not mix refactors with behaviour changes.
- Do not commit generated artefacts or anything under `target/`.
