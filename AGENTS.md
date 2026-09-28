# AGENTS.md

Guidance for automated coding agents in this repository. It applies to any agent or tool. Humans should read [CONTRIBUTING.md](CONTRIBUTING.md); the two agree.

## Project

Nomos is a host-state convergence and fleet-control system for Linux, written in Rust. It compares desired state (Canon) with observed state, computes the Variance, plans Actions as a dependency graph (Warp), applies them through the OS boundary (Substrate), verifies the result, and records Events.

The repository is at the **skeleton stage**. Crates contain module docs only. Do not add implementation unless the task asks for it.

## Source of truth

| Need | Read |
| --- | --- |
| Semantics, vocabulary, invariants | `docs/PROJECT-SPEC.md` |
| Crate layout and dependency rule | `docs/architecture/hexagon.md`, `docs/adr/0000-foundations.md` |
| Runtime behavior | `docs/architecture/runtime.md` |
| Algorithms and proof obligations | `docs/formal/` |
| Past decisions | `docs/adr/` |

If a task conflicts with these documents, stop and report the conflict. Do not silently diverge.

## Commands

The toolchain is pinned to Rust 1.98.1 in `rust-toolchain.toml`.

```sh
cargo check  --workspace
cargo fmt    --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test   --workspace
```

A change is complete only when all four pass.

## Layout

| Path | Layer | May depend on |
| --- | --- | --- |
| `crates/core/` | Domain | `core/` only (`nomos-core` depends on nothing) |
| `crates/ports/` | Driven ports | `core/` |
| `crates/app/` | Use cases | `core/`, `ports/` |
| `crates/adapters/` | Driven adapters | `core/` and the one port they implement |
| `crates/bin/` | CLIs, composition roots | Anything |

Workspace crates are referenced through `[workspace.dependencies]` in the root `Cargo.toml`, as `name.workspace = true`.

## Rules

1. **Never break the dependency rule.** An adapter dependency in `app/`, `core/`, or `ports/` is always wrong.
2. **New crates, new ports, and changes to the rule need an ADR** in `docs/adr/`, with the next number.
3. **Use the spec's vocabulary exactly:** Canon, Trait, Cipher, Variance, Trace, Enforce, Event, Event Log. No synonyms.
4. **Preserve invariants N1–N12** (spec §58). A change that touches one needs a test for it.
5. **No shell execution in Substrate.** Use native APIs, for example systemd over D-Bus.
6. **Never log, serialize, or embed Cipher plaintext.**
7. **Determinism.** Nothing that feeds compilation or planning may depend on hash iteration order, randomness, or wall-clock time.
8. **No `unsafe`.** Workspace lints forbid it.
9. **Leave the pinned toolchain, the license, and CI alone** unless asked.

## Documentation conventions

- The README is narrative. Technical detail goes under `docs/`.
- Voice: plain, direct statements. A dry aside is fine in prose, never in rules or definitions.
- American spelling, sentence-case headings, Oxford comma, one paragraph per line.
- Diagrams are **Mermaid only** and must render on GitHub. Never use ASCII or box-drawing art.
- Never label Mermaid arrows or state transitions (`-- text -->`, `-->|text|`, `S1 --> S2: text`). They fail to render on GitHub. Put the text in a node or a label node (`A --> L(["text"]) --> B`).
- Math uses GitHub math syntax (`$...$`, `$$...$$`).
- Update the relevant docs and index tables in the same change.

## Commits

- Imperative subject line, with the *why* in the body.
- One focused change per commit. Refactors and behavior changes go separately.
- Never commit generated artifacts or anything under `target/`.
