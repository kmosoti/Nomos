# AGENTS.md

Guidance for automated coding agents in this repository. It applies to any agent or tool. Humans should read [CONTRIBUTING.md](CONTRIBUTING.md); the two agree.

## Project

Nomos is a host-state convergence and fleet-control system for Linux, written in Rust. Canon is compiled desired intent, authored in typed Rust and consumed as an inert artifact. Nomos assesses each Condition of the Canon against Observations from the OS boundary (Substrate) as Satisfied, Variance, or Indeterminate, plans Actions for the known Variances as a dependency graph (Warp), applies them through Substrate, verifies the result, and records Events. Unknown evidence does not imply noncompliance.

The repository is at its **second alpha**, `0.1.0-alpha.2`: the masterless Cell of `docs/plans/phase-1-masterless-cell.md`, built on the kernel of `docs/research/2026-09-28-typed-core/grounding-plan.md`. Do not add implementation unless the task asks for it. Work beyond Phase 1 waits for its plan; a task that assigns a milestone or experiment of a plan is such an ask, for the crates it names.

## Source of Truth

| Need | Read |
| --- | --- |
| Semantics, vocabulary, invariants | `docs/PROJECT-SPEC.md` |
| Crate layout and dependency rule | `docs/architecture/hexagon.md`, `docs/adr/0000-foundations.md` |
| Runtime behavior | `docs/architecture/runtime.md` |
| Algorithms and proof obligations | `docs/formal/` |
| What counts as evidence, and what is checked | `docs/formal/verification-strategy.md`, `docs/formal/verification-matrix.md` |
| How code is written and judged | `docs/adr/0015-generator-verifier-development-model.md` |
| What a core crate may depend on | `docs/formal/core-purity.md`, `crates/core/PURITY.toml` |
| Past decisions | `docs/adr/` |
| Research findings, grounding experiments, and their results | `docs/research/` |
| Prose style | `docs/style/prose-spec.yaml`, `docs/style/README.md` |

If a task conflicts with these documents, stop and report the conflict. Do not silently diverge.

## Commands

The toolchain is pinned to Rust 1.98.1 in `rust-toolchain.toml`.

```sh
cargo fmt    --all --check
cargo check  --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test   --workspace --locked
```

A change is complete only when all four pass. `--locked` is not optional: CI never lets dependency resolution change silently, and neither should you.

Development tooling runs as `cargo xtask <command>` (ADR 0003). CI also runs these on the required path, and so should you:

```sh
cargo xtask research verify-all docs/research   # every accepted snapshot: manifest, graph, schema
cargo xtask check-layers                         # the dependency rule, declared and resolved graphs
cargo xtask check-core-purity                    # crates/core/PURITY.toml: no_std, no build script, allowlisted deps
cargo xtask check-trust-boundary --base origin/main   # every commit declares its oracle changes
cargo xtask check-evidence-tests                 # evidence-bearing tests are protected by their path or their pin
cargo xtask check-workflow-pins                  # every workflow action is pinned to a full commit SHA
cargo xtask receipts validate                    # every receipt names an executed, registered check
cargo xtask mutants semantic                     # every active semantic mutant is caught by its named test
```

`cargo test --workspace` runs the same checks as unit tests, through the same functions. A failure prints a stable code: `[checksum-mismatch]` from the snapshot verifier, `FORBIDDEN [app-depends-outside-core-and-ports]` from the layer checker, `IMPURE [core-dependency-denied-class]` from the purity checker, `UNDECLARED [undeclared-oracle-change]` from the trust-boundary gate, `REJECTED [receipt-passed-without-evidence]` from the receipt validator. Changing what any gate accepts is a trust-boundary change under rule 11. CI also runs `cargo xtask research frozen` and `check-trust-boundary` on every push and pull request, against the base branch for a pull request, the previous head for a push to the default branch, and the default branch otherwise; a base that cannot be resolved fails the job. `cargo mutants` runs on a schedule and never on the required path.

## Layout

| Path | Layer | May depend on |
| --- | --- | --- |
| `crates/core/` | Domain | `core/` only (`nomos-core` depends on nothing); `no_std`; external crates only through `crates/core/PURITY.toml` |
| `crates/ports/` | Driven ports | `core/` |
| `crates/app/` | Use cases | `core/`, `ports/` |
| `crates/adapters/` | Driven adapters | `core/` and the one port they implement |
| `crates/bin/` | CLIs, composition roots, and the `nomos-xtask` tooling crate | Anything |

Workspace crates are referenced through `[workspace.dependencies]` in the root `Cargo.toml`, as `name.workspace = true`.

## Rules

1. **Never break the dependency rule.** An adapter dependency in `app/`, `core/`, or `ports/` is always wrong.
2. **New crates, new ports, and changes to the rule need an ADR** in `docs/adr/`, with the next number.
3. **Use the spec's vocabulary exactly:** Canon, Condition, Observation, Assessment, Variance, Indeterminate, Obligation, Settled, Action, Plan, Trait, Cipher, Trace, Enforce, Event, Event Log. No synonyms. A failed observation is Indeterminate, never a Variance. "State" is still the right word for Action lifecycle state, protocol and scheduler state machines, and internal control state; it is the wrong word for a Condition or an Observation.
4. **Preserve invariants N1–N13** (spec §58). A change that touches one needs a test for it.
5. **No shell execution in Substrate.** Use native APIs, for example systemd over D-Bus.
6. **Never log, serialize, or embed Cipher plaintext.**
7. **Determinism.** Nothing that feeds compilation or planning may depend on hash iteration order, randomness, or wall-clock time.
8. **No `unsafe`.** Workspace lints forbid it.
9. **Leave the pinned toolchain, the license, and CI alone** unless asked.
10. **Tooling is Rust.** A check, generator, or reproducer that produces a record goes in `crates/bin/nomos-xtask`, not in a Python or shell script. A snapshot's own scripts stay under its `snapshot/` directory as received.
11. **Never weaken a specification to pass a check.** A test, proof, or model that passes because a postcondition was loosened, an `assume`, `admit`, or axiom was added, a test was ignored, a mutant was skipped, a lint was allowed, or code was moved out of the verifier's view proves nothing. Such changes are trust-boundary changes: make them in their own commit, declare them (below), say what they weaken, and report verifier output as it is. Research snapshots under `docs/research/` are evidence and are never edited; a revision is a new dated snapshot.
12. **Core crates are pure.** No clock, environment, filesystem, network, process identity, randomness, threads, globals, logging, panics outside tests, or build scripts in `crates/core/`; effects are `EffectRequest` values. The compiler and `check-core-purity` enforce what they can; the rest is `docs/formal/core-purity.md`.

## Generator-Verifier Discipline

You are the generator. The specification and the verifier judge what you write, and they are protected ([ADR 0015](docs/adr/0015-generator-verifier-development-model.md)).

- **Protected paths.** `docs/PROJECT-SPEC.md`, `docs/formal/`, `formal/`, and `docs/adr/` are the specification. `crates/bin/nomos-xtask/`, `tests/`, `crates/*/*/tests/` (the integration tests of every crate), `verification/`, `crates/core/PURITY.toml`, `.github/workflows/`, `.cargo/`, the root `Cargo.toml`, and `rust-toolchain.toml` are the verifier. The list is `verification/trust-boundary.toml`.
- **Where a test goes.** An integration test of a crate is a verifier path, so a new or changed one is its own `Trust-Boundary: verifier` commit, apart from the implementation it judges. A test inside a production source file that the semantic-mutant corpus names is pinned in `verification/evidence-oracles.toml`: editing it is a verifier commit that also carries the new pin (`cargo xtask evidence-tests write`), and the oracle map `docs/formal/oracle-map.md` it regenerates is a specification commit. Any other inline test is a regression test and yours to change. [Test Taxonomy](docs/formal/verification-strategy.md#test-taxonomy) has the table.
- **Declare oracle changes.** A commit that touches the specification carries `Trust-Boundary: specification` in its message body; one that touches the verifier carries `Trust-Boundary: verifier`; one that adds an escape hatch (an ignored test, a skipped mutant, an allowed panic lint, an `assume` or `admit`) carries `Trust-Boundary: escape-hatch`. Such a commit touches no implementation crate. `cargo xtask check-trust-boundary` fails otherwise, in CI. The declaration is a request for review, not a pass.
- **Your tests are regression tests.** A test you wrote for code you wrote is evidence that the code does what the code does. It counts as evidence for a property only when its oracle comes from outside the code: an invariant in `docs/formal/`, an exhaustive truth table, an independently written reference, a metamorphic relation, or a proof obligation. Say which, or say that it is a regression test.
- **Prefer the oracle that exists.** Before writing a test, check `tests/semantic-mutants/corpus.toml` and the invariants for the wrong behavior your change must reject, and name that test. A semantic mutant that survives is a test to strengthen, never a mutant to weaken.
- **Counterexamples become fixtures.** A failing input a property test, model, or mutation run finds is minimized and committed with its provenance: what found it, the seed or trace, the input, the property, and the fixing commit.
- **Record what ran.** A check counts when a receipt exists: `cargo xtask receipts record <check-id> --out verification/receipts/<file>.ndjson -- <command>`. Never write a receipt by hand. Never report a command you did not run as passing; a check that did not run is `not run`, a timeout is `inconclusive`.
- **No thresholds, no scores.** A mutation score, a coverage number, or a pass rate is never a target and never a claim.
- **Merge with merge commits.** A squash loses the declarations.

## Process Invariants

Hold these across every task, whatever it asks:

- A specification is never edited to make a check pass.
- Verifier output is reported as printed, with its codes.
- A gate that cannot run reports failure, not success.
- A result about a model is reported as a model result; a result about a mock as a mock result; a paper's result as the paper's.
- No implementation lands ahead of its milestone; a task that names a milestone or an experiment is the ask, and only for the crates it names.
- Milestone work is on a branch named `milestone/<nn>-<name>` from the grounding plan, never `claude/*`, `agent/*`, `pr-*`, or `feature/pr-*`.

## Completion Report

End a task with a report in this form. A field with nothing to say says so. The evidence is what ran and what changed: commands, receipts, diffs, and test names. Do not include, and nobody will ask for, a transcript of your reasoning; a narrative is not a record.

1. **Branch**, base commit, and final commit.
2. **Documents and ADRs** created or changed.
3. **Sources verified**, for research work, with what was and was not fetched.
4. **Commands run**, each with its result as printed, and any command the task named that was not run, with why.
5. **Test counts**: passed, failed, ignored.
6. **Negative controls** exercised, by code.
7. **Experiments** run, with their result records; experiments designed and not run, marked so.
8. **Surviving mutants** and their disposition, when mutation ran.
9. **Receipts** written.
10. **Unchecked behavior**: what the work does not establish.
11. **Open decisions** left for the owner.

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
- One focused change per commit. Refactors and behavior changes go separately; oracle changes go separately from implementation, with their `Trust-Boundary:` line.
- Never commit generated artifacts or anything under `target/`. Receipts under `verification/receipts/` are records, not artifacts, and are committed.
