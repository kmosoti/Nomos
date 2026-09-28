# AGENTS.md

Guidance for automated coding agents in this repository. It applies to any agent or tool. Humans should read [CONTRIBUTING.md](CONTRIBUTING.md); the two agree.

## Project

Nomos is a host-state convergence and fleet-control system for Linux, written in Rust. It compares desired state (Canon) with observed state, computes the Variance, plans Actions as a dependency graph (Warp), applies them through the OS boundary (Substrate), verifies the result, and records Events.

The repository is at the **skeleton stage**. Crates contain module docs only. Do not add implementation unless the task asks for it. The next step is the Phase 0 grounding phase in `docs/research/2026-09-28-typed-core/grounding-plan.md`; a task that assigns one of its experiments is such an ask, for the crates that experiment names.

## Source of Truth

| Need | Read |
| --- | --- |
| Semantics, vocabulary, invariants | `docs/PROJECT-SPEC.md` |
| Crate layout and dependency rule | `docs/architecture/hexagon.md`, `docs/adr/0000-foundations.md` |
| Runtime behavior | `docs/architecture/runtime.md` |
| Algorithms and proof obligations | `docs/formal/` |
| Past decisions | `docs/adr/` |
| Research findings, grounding experiments, and their results | `docs/research/` |
| Prose style | `docs/style/prose-spec.yaml`, `docs/style/README.md` |

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
10. **Never weaken a specification to pass a check.** A test, proof, or model that passes because a postcondition was loosened, an `assume`, `admit`, or axiom was added, a test was ignored, or code was moved out of the verifier's view proves nothing. Such changes are trust-boundary changes: make them in their own commit, say what they weaken, and report verifier output as it is. Research snapshots under `docs/research/` are evidence and are never edited; a revision is a new dated snapshot.

## Documentation Conventions

- Prose follows [docs/style/prose-spec.yaml](docs/style/prose-spec.yaml), in *polished* mode. Its `agent_instruction.system_prompt` is your writing instruction. Apply its `rewrite_algorithm` to any prose you write or edit.
- Run `.vale/lint.sh`. Fix every error. Fix warnings unless the fix damages clarity. CI reports the same findings but does not block on them, so the fixing is on you.
- The review-only rules (KEN004, KEN008, KEN010–KEN012, KEN014–KEN016) are your job, not the linter's. [docs/style/README.md](docs/style/README.md) lists them.
- Humor is dry, understated, and rare. Never forced.
- The README is narrative. Technical detail goes under `docs/`.
- American spelling, Title Case headings, Oxford comma, one paragraph per line.
- Diagrams are **Mermaid only** and must render on GitHub. Never use ASCII or box-drawing art.
- Never label Mermaid arrows or state transitions (`-- text -->`, `-->|text|`, `S1 --> S2: text`). They fail to render on GitHub. Put the text in a node or a label node (`A --> L(["text"]) --> B`).
- Math uses GitHub math syntax (`$...$`, `$$...$$`).
- Update the relevant docs and index tables in the same change.

## Commits

- Imperative subject line, with the *why* in the body.
- One focused change per commit. Refactors and behavior changes go separately.
- Never commit generated artifacts or anything under `target/`.
