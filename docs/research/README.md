# Research

Dated research snapshots, the evaluation of each against the repository, and the experiments they propose. A snapshot is evidence about the design at one moment. The specification, ADRs, and formal documents remain the source of truth; a snapshot argues with them, and an ADR settles the argument.

| Snapshot | Scope | Status |
| --- | --- | --- |
| [2026-09-28-typed-core](2026-09-28-typed-core/README.md) | Typed core: algebraic Canon model, evidence and assessment, Warp gates, effect recovery, fencing, Event Log boundaries, verification discipline | Imported and evaluated. Decisions D1 and D2 recorded as ADRs 0004 to 0006. [Grounding plan](2026-09-28-typed-core/grounding-plan.md): `01-foundation-gates` and `02-verification-foundation` landed; four result records under `results/` |
| [2026-09-28-generator-verifier](2026-09-28-generator-verifier/README.md) | Digest: generator-verifier asymmetry, specification gaming, generated-test oracles, mutation, metamorphic and differential testing, receipts | Written from primary sources during `02-verification-foundation`; exploratory, fed [ADR 0015](../adr/0015-generator-verifier-development-model.md) |

## Layout

Each snapshot is one directory named by date and topic.

| Path | Contents | Mutable |
| --- | --- | --- |
| `snapshot/` | The bundle exactly as received, with its `MANIFEST.sha256`. A flat directory of regular files: no symlinks, subdirectories, or special files | Never. A revision is a new dated directory |
| `README.md` | What the bundle claims, what was verified on import, what holds against the repository, what conflicts with the specification, and where each result went | Yes |
| `grounding-plan.md` | The experiments to run before the affected semantics are frozen, in dependency order | Yes, until the phase closes |
| `results/` | One record per executed experiment: toolchain, bounds, what was and was not established, and the decision it feeds | Append only |

Verify a snapshot before trusting it. The command checks the manifest, parses the graph strictly, runs the referential checks and negative controls, and validates every record against the snapshot's schema ([ADR 0003](../adr/0003-xtask-tooling-crate.md)):

```sh
cargo xtask research verify docs/research/<snapshot>/snapshot   # one snapshot, full report
cargo xtask research verify-all docs/research                   # every snapshot, as CI runs it
```

`cargo test --workspace` runs the same checks through the same `verify_snapshot` function, so a snapshot that stops verifying fails the build. There is no weaker mode: a symlink, subdirectory, special file, or unreadable entry fails before anything is hashed, a missing, empty, malformed, or incomplete manifest fails, and so does any stage after it. Each failure carries a stable code, for example `[uncovered-file]` or `[json-duplicate-key]`. CI also runs `cargo xtask research frozen` on every push and pull request, against the base branch, the default branch's previous head, or the default branch: a file added to, changed in, removed from, or retyped in a snapshot that existed there fails, because matching checksums cannot tell an accepted manifest from a rewritten one. A new dated snapshot directory is allowed.

The snapshot directories are excluded from the prose and Markdown linters. Their text is evidence, and editing evidence to satisfy a style rule would falsify the checksums.

## Reading a Snapshot

The 2026-09-28 bundle is a property graph in newline-delimited JSON (NDJSON), one record per line, with a JSON Schema and a validator beside it. Later snapshots may use other formats; the rules below apply to all of them.

- **Weights are editorial.** An importance or salience score is a reviewer's ordinal judgment, useful for deciding what to read first. It is not evidence, probability, or a benchmark result.
- **Review order is a command away.** `cargo xtask research list docs/research/<snapshot>/snapshot` prints the recommendations by salience, then importance, with disposition and phase.
- **Disposition decides what a recommendation is for.** `adopt_direction` is a proposal to build toward. `trial` and `research_required` are questions with a suggested experiment. `defer_until_measured` is explicitly not a current dependency. A high score with the wrong disposition is not an instruction.
- **Proposed is not executed.** An experiment node describes a test. Its `execution_status` says whether the test ran. Until a record exists under `results/`, nothing in a snapshot has been demonstrated on Nomos code.
- **Vocabulary stays the specification's.** A snapshot may coin terms to make an argument. The repository documents use spec §3 vocabulary, and a new term enters the vocabulary only through an ADR.
