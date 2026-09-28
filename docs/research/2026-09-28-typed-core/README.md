# Typed-Core Research Snapshot, 2026-09-28

Imported from `nomos-research-2026-09-28.zip`. The bundle reviewed `main` at commit `d4c11fa`, which is also the commit this import builds on, so its repository observations refer to the drafts as they stood. [snapshot/](snapshot/) holds the bundle byte for byte. This page is the evaluation and the map of where each result went.

## What the Bundle Is

A weighted property graph of 169 nodes and 348 edges in newline-delimited JSON (NDJSON), with a JSON Schema, a validator, a human-readable [recommendation index](snapshot/RECOMMENDATIONS.md), seven small Python counterexample models, and their reports. The bundle's own [README](snapshot/README.md) documents the record model and the weight rubric.

| Node kind | Count | What it is |
| --- | ---: | --- |
| Source | 37 | 30 external primary references, six repository files at `d4c11fa`, one conversation decision record |
| Concept | 18 | Vocabulary as the bundle uses it, which differs from spec §3 in places noted below |
| Finding | 26 | Documented mechanisms, repository observations, or labeled deductions |
| Recommendation | 26 | Proposed directions with constraints, acceptance criteria, and target paths |
| Experiment | 21 | Proposed tests. None has run |
| Open question | 14 | Unresolved decisions, each with a closure rule |
| Correction | 6 | Retractions of earlier design guidance from the same conversation |
| Counterexample | 7 | Executed Python models of draft semantics, not Nomos tests |
| Research direction | 6 | Contemporary work to borrow from selectively |
| ADR candidate | 8 | Decision bundles, not accepted ADRs |

It is a design artifact. It built no Nomos code, checked no TLA+ model, and executed no Linux test. Its weights are ordinal editorial judgments on a 0–4 scale. The field that matters for reading it is `disposition`: 18 recommendations are `adopt_direction`, one is `trial`, 6 are `research_required`, and one is `defer_until_measured`.

## Verification on Import

The bundle's own Python scripts were read, then run once on import: the validator's standard-library checks passed with the shipped graph hash, and the reproducer's output was identical to the shipped `counterexample-results.json`. The schema pass could not be rerun with Python because the `jsonschema` package was not installed here.

The repository's instrument is the Rust tool in `nomos-xtask` ([ADR 0003](../../adr/0003-xtask-tooling-crate.md)), which reimplements both scripts. Its results on this snapshot:

| Check | Command | Result | What it establishes |
| --- | --- | --- | --- |
| Manifest | `cargo xtask research verify snapshot` | All nine files match `MANIFEST.sha256` | The copy under `snapshot/` is the bundle as delivered |
| Graph checks | same | Passed. `graph_sha256`, `counts`, `checks`, and `negative_controls` are byte for byte the shipped report's | The graph is well formed and internally consistent |
| JSON Schema | same | All 518 records pass the snapshot's schema with `date` format checking, using the Rust `jsonschema` crate; the schema hash matches the shipped report | Per-record conformance is established here, not only by the bundle's report |
| Counterexamples | `cargo xtask research reproduce` | Seven models, `id` and `observed` identical to the shipped file | The models are deterministic and say what the bundle says they say |
| Continuous | `cargo test --workspace` | The manifest, graph, and schema checks and the seven models run as unit tests | A snapshot that stops verifying fails the build |

The Python scripts stay under `snapshot/` as received and are not the record's tool.

None of this establishes a claim about Nomos. The counterexamples are Python models of the draft formal documents. They show that the drafts, read literally, admit the bad outcomes. They do not show that Rust code has a bug, because there is no Rust code yet.

## Findings Against the Repository

Each finding was checked against the draft it cites. *Confirmed* means the draft says what the finding says and the argument holds. *Qualification* means the draft's claim is stronger than it can be. *Design input* means no draft covers the topic yet. The last column says where the correction or the concern now lives.

| Finding | Draft | Verdict | Where it went |
| --- | --- | --- | --- |
| `final-check` | `reconciliation.md` tested $V = \varnothing$ only before `execute`. With $k = 1$, a successful last mutation returned `NonConvergent(bound)` | Confirmed error | The loop now observes after the last permitted execution |
| `activation-missing` | `warp.md` made an `on_change` dependent ready once its source was terminal. Its own table said activation needs a change | Confirmed inconsistency | $\mathrm{Ready}$ now requires activation by a verified change |
| `noop-anchor` | $V$ held only mutating Actions, so a satisfied `requires` target had no vertex and its edge dangled | Confirmed gap | Satisfaction anchors, working definition |
| `scc-not-cycle` | `warp.md` called strongly connected components "the minimal cycles" | Confirmed overstatement | One explicit witness cycle per component, no minimality claim |
| `budget-not-world` | `invariants.md` wrote N9 as $\forall f: \mathrm{Unavailable}(f) \le k_f$, a bound on the world. Spec §58 says budgets are never *intentionally* exceeded | Confirmed overstatement | N9 predicate is now about admission |
| `bound-not-termination` | The termination proof counted iterations and said nothing about a call that never returns | Qualification | Termination names its deadline assumption; a fired deadline does not undo an effect |
| `fingerprint-limits` | One repeated observation fingerprint meant oscillation | Qualification | A repeat counts only on the managed-resource projection and with no work in flight |
| `fence-race` | `fencing-and-idempotency.md` checked the generation before dispatch; the effect came later | Confirmed gap | The N5 theorem now states its atomicity assumption. ADR candidate `recovery-authority` |
| `dedup-scope` | The idempotency key's scope was undefined. A semantic-content key would suppress later repairs | Design gap | The key names one execution within one Plan |
| `lost-refresh` | The crash-window argument assumed every effect is visible in Variance. A lost restart is not | Design gap | Caveat in the crash window; Known Gaps in `warp.md`. ADR candidate `warp-gates` |
| `replay-not-reality` | `event-log.md` called the fold over $L$ "state" | Qualification | The fold is control state. Recovery re-observes the host |
| `trace-scope` | N1 compared whole machines | Qualification | N1 compares a projection onto managed-resource properties |
| `layer-policy` | ADR 0000 said the manifests make the build enforce the dependency rule | Confirmed overstatement | ADR 0000 amended. Experiment `layer-policy` |
| `types-not-world` | The leverage section implied a constructor proves a property | Qualification | Note in `invariants.md` on what a type proves |
| `decode-validation`, `sum-not-product`, `enum-wire`, `canonical-not-wire`, `build-not-pure` | `CANON.md` is unwritten | Design input | Grounding plan, wave 3. ADR candidate `canon-artifact` |
| `partial-assessment` | Spec §9 `diff` returns Variance or nothing | Design input | Grounding plan, wave 1. Known Gaps in `reconciliation.md`. ADR candidate `evidence-model` |
| `single-controller` | The reconciliation model has one controller | Design input | Grounding plan, wave 4 |
| `agent-spec-gaming` | No rule protected specifications from being weakened to pass a check | Adopted | AGENTS.md rule 10 and CONTRIBUTING.md |
| `epoch-not-oracle`, `identity-path`, `secrets-scope`, `incremental-scope` | Beyond Phase 0 | Deferred | Listed under deferred experiments in the grounding plan |

The six corrections in the bundle retract guidance from the conversation that produced it, not text in this repository. None of the retracted forms appears in the repository, and the corrected forms are what the grounding plan carries forward.

## Conflicts With the Specification

Three points in the bundle disagreed with the specification or with the agent rules when it was imported. Each is a specification amendment plus an ADR, which is the project owner's decision. One has since been decided; two remain open.

**Canon authoring surface.** *Resolved on 2026-09-28 by [ADR 0004](../../adr/0004-rust-typed-canon.md).* At import, spec §5 showed Canon as YAML, §54 listed the syntax ablation as strict YAML against TOML with the same typed intermediate representation (IR) either way, and §5 said Canon does not embed a general-purpose programming language. The bundle took a Rust-authored Canon, a crate that generates an inert artifact, as the intended direction, on the strength of a conversation decision (`src:user-decisions`) rather than the specification, while noting that Rust authoring is code execution and needs an isolated build job without host credentials. The project owner confirmed the direction, and the specification was amended: Rust is the authoring surface, hosts accept only the Canonical IR, and the build job is treated as code execution. The encoding profile of the IR is still the wave 3 experiment.

**Vocabulary.** The bundle uses *Condition*, *Observation*, *Assessment*, and *Indeterminate*. Spec §3 fixes Canon, Trait, Cipher, Variance, Trace, Enforce, Event, and Event Log, and §9 already uses Observation and Variance in the driver contract. The mapping is:

| Bundle term | Specification | Status |
| --- | --- | --- |
| Condition | A resource's `spec` in Canon, $D_r$ | Existing concept, no new name needed yet |
| Observation | `Observation`, $O_r$ | Already in spec §9 |
| Assessment: Satisfied, Variance, Indeterminate | The result of `diff` | `diff` has two outcomes today. The third, for evidence that supports neither, is proposed. ADR candidate `evidence-model` |

The repository documents describe the third outcome in words where they need it and coin no term for it.

**Package-manager invocation.** Recommendation `substrate` says a controlled `argv` invocation of an unavoidable package-manager command line is not shell interpolation. AGENTS.md rule 5 says no shell execution in Substrate, and spec §12 requires D-Bus for systemd. A direct `execve` with a fixed argument vector is not a shell, but nobody has decided whether it is allowed, and `package` is a Phase 1 resource (spec §10, §55). Recorded, not decided.

## Recommendations and Where They Land

The bundle groups its 26 recommendations into eight ADR candidates. The grounding plan assigns each candidate to the wave whose experiments produce its evidence.

| ADR candidate | Recommendations | Fed by |
| --- | --- | --- |
| `verification-gates` | `formal`, `agent-proof`, `layer-enforcement` | Wave 0, wave 4 |
| `evidence-model` | `evidence-assessment`, `composition`, `identity-recovery` | Wave 1, wave 4. `identity-recovery` deferred |
| `warp-gates` | `warp-semantics`, `durable-refresh`, `scheduler` | Wave 1, wave 2 |
| `recovery-authority` | `effect-recovery`, `fencing`, `budget` | Wave 2. `fencing` deferred to before remote execution |
| `canon-artifact` | `typed-canon`, `validated-boundary`, `algebraic-model`, `canonical-profile`, `bounded-bindings` | Wave 3, after decision D1 |
| `event-history` | `log-boundary`, `log-retention` | Phase 2. Concerns recorded in `event-log.md` |
| `boundary-security` | `substrate`, `cipher` | Phase 1 and Phase 6 |
| `future-algorithms` | `incremental`, `plan-witness` | After Phase 0 |

The bundle's recommendation dependency edges form a directed acyclic graph (DAG). Six recommendations depend on nothing and are prerequisites of others: `algebraic-model`, `evidence-assessment`, `effect-recovery`, `log-boundary`, `pure-kernel`, and `formal`. The wave order follows that DAG.

## Querying the Graph

```sh
cargo xtask research list docs/research/2026-09-28-typed-core/snapshot
```

```text
[4/4] adopt_direction      phase0                     Protect invariants from verification gaming by coding agents
[4/4] adopt_direction      phase0                     Use sum types to remove contradictions, not just to rename tags
[4/4] adopt_direction      before_fleet_mutation      Replace impossible world-wide safety with admission safety
[4/4] trial                phase0                     Specify semantic normalization separately from encoding
```

The bracket is salience over importance. Anything else is a `serde_json` read of `snapshot/nomos-research.ndjson`, one object per line; the `graph` module of `nomos-xtask` is the reference for the record shapes.

Node IDs are stable within the snapshot and have the form `urn:moiric:nomos:research:2026-09-28:<kind>:<key>`. The formal documents cite counterexamples and experiments by their `<key>`.

## Revision Policy

`snapshot/` never changes. A revised graph is a new dated directory with its own manifest and its own evaluation. Results of the grounding experiments go under `results/` here, one record per experiment, and a result record is written only after the experiment ran.
