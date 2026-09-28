# ADR 0003: Development Tooling Lives in the nomos-xtask Crate

- **Status.** Accepted
- **Date.** 2026-09-28

## Context

The first research snapshot arrived with its own tooling in Python: a graph validator and a reproducer for seven counterexample models. The record of what was verified on import should be reproducible with the pinned toolchain, in the language the repository is written in, and checked by the same `cargo test --workspace` that CI already runs. The grounding plan also needs a home for the dependency-policy check and the specification-protection gate, which are neither domain code nor a Nomos binary. AGENTS.md rule 2 says a new crate needs an ADR.

## Decision

1. Development tooling is one crate, `crates/bin/nomos-xtask`, invoked as `cargo xtask <command>` through the alias in `.cargo/config.toml`.
2. It sits in `crates/bin/` because that layer may depend on anything. No workspace crate depends on it, and it is not listed under `[workspace.dependencies]`, so it can never enter the domain, a port, the application, or an adapter.
3. Its external dependencies are grouped under a `Tooling` comment in `[workspace.dependencies]` and used by nothing else. Today: `serde`, `serde_json` with `preserve_order`, `sha2`, and `jsonschema` without its default features, so it resolves no references over the network or the filesystem.
4. Its first commands are `research verify`, `research list`, and `research reproduce`. The verifier checks a snapshot's manifest, parses the NDJSON graph strictly with duplicate-key detection, runs the referential and structural checks, validates every record against the snapshot's JSON Schema with `date` format checking on, and runs the negative controls. The reproducer runs the seven counterexample models, which are also unit tests. The [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md) adds the layer-policy check and the specification gate here.
5. A snapshot's own scripts stay under `snapshot/` exactly as received, because the snapshot is evidence. The Rust tool is the repository's instrument, and result records cite it.
6. Tooling is written in Rust. A check, generator, or reproducer that produces a record goes in this crate, not in a script.

## Consequences

- `Cargo.lock` grows by the tool's dependencies, and `cargo test --workspace` compiles them. That cost buys the snapshot's manifest, graph, and schema being re-verified, and the seven counterexamples re-run, on every test run.
- The verifier's report has the same shape as the Python report the snapshot ships. Its `graph_sha256`, `counts`, `checks`, and `negative_controls` agree with the shipped report byte for byte; the `json_schema` block names the Rust crate and its semver requirement instead of the Python package, and `Cargo.lock` pins the exact version.
- **Failure behavior.** Any failed check exits non-zero with the failing check's message and writes no report. A checksum mismatch stops the run before the graph is read.
- The counterexamples in this crate model prose, not Nomos code. They stay here as the record's reproducer even after the grounding plan ports each one into the crate it concerns as a negative control.
