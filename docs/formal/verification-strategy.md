# Verification Strategy

How a claim about Nomos becomes evidence, layer by layer, and what each layer cannot show. The rules are [ADR 0007](../adr/0007-verification-gates.md) and [ADR 0015](../adr/0015-generator-verifier-development-model.md); this page is the ladder they refer to, the policies that sit on it, and the list of things that look like evidence and are not.

## Three Questions That Are Not One

Every result on this page answers exactly one of three questions, and the [verification matrix](verification-matrix.md) names which.

| Question | What it is about | Answered by |
| --- | --- | --- |
| **Specification correctness** | Do the written algorithms and invariants say what we mean, and are they consistent with each other? | Proof sketches in `docs/formal/`, counterexample models, TLA+ models checked by the TLA+ model checker (TLC) under bounds |
| **Implementation conformance** | Does the Rust do what the specification says? | Truth tables, property tests, differential and metamorphic tests, semantic mutants, bounded verifiers, trace replay |
| **Environment and adapter correspondence** | Does the operating system, through an adapter, behave the way the specification assumes an Observation or an effect behaves? | Port conformance suites, integration tests on Linux, failure injection |

A TLA+ model proves nothing about Linux. A property test proves nothing about the specification it encodes. A conformance suite on a mock proves nothing about `systemd`. The matrix keeps the three columns apart so that a green cell is never read as the wrong kind of evidence.

## The Ladder

Twelve layers. Each lists what a pass establishes, what it does not, and where it runs. A layer is adopted when there is something to ask it (ADR 0007 §6); layer 12 is not yet.

| # | Layer | Establishes | Does not establish | Runs |
| --- | --- | --- | --- | --- |
| 1 | Format: `cargo fmt --check` | The candidate is normalized, so diffs and gates see one form | Anything about behavior | Required |
| 2 | Compile: `cargo check --locked` | Types line up under the locked resolution; a `no_std` core cannot name `std` | That the types mean what their names say. A type gate is not evidence of truth: `Succeeded` built only from a `Verification` shows verification ran, not that it was right | Required |
| 3 | Lint: `cargo clippy -D warnings`, the panic lints | No `unsafe`, no panic path in core outside tests, no lint the workspace denies | Absence of logic errors | Required |
| 4 | Unit tests and exhaustive truth tables | Each finite decision rule handles every listed case as the table says | Cases the table omits; that the table is the specification's intent | Required |
| 5 | Structural and policy gates: `check-layers`, `check-core-purity`, `research verify-all`, `research frozen`, `check-trust-boundary`, `receipts validate` | The workspace obeys the dependency rule and the purity policy; research evidence is intact and unchanged; oracle changes are declared; records are well formed | Any semantics. A gate that passes because its input was weakened proves nothing (rule 11) | Required |
| 6 | Property tests | An algebraic law holds on the generated inputs, with the seed recorded | Inputs the generator cannot produce; the law's own correctness | Required, from the seed bank in each crate's `tests/laws.rs` |
| 7 | Metamorphic and differential tests | Related inputs yield related outputs; two independent formulations agree on generated inputs | An output both formulations get wrong; anything outside the generators' coverage, which the record states | Required; the first relations run in `03-assessment-kernel` |
| 8 | Semantic mutants and mutation calibration | A named wrong behavior is caught by a named test; syntactic mutants are classified by a person | Correctness. Mutation testing does not prove semantics; it measures whether the tests would notice a wrong implementation, and only the mutants it generated | Semantic mutants required; `cargo-mutants` scheduled |
| 9 | Deterministic simulation | Under scripted delays, crashes, duplicates, and supersession, the kernel's decisions satisfy the stated properties | Real timing, real crashes | Adopted in `05-transition-kernel`: `crates/bin/nomos-cell/tests/`, required, with the generated interleavings seeded |
| 10 | Executable formal models: TLA+ with the TLC model checker | The modeled transitions cannot reach a violating state within the bounds | Anything outside the bounds; that the model matches the Rust, which trace replay checks; anything about Linux | Adopted in `05-transition-kernel`: TLC run by hand and recorded under check `tlc`, not in CI; the traces are fixtures, and their replay is required |
| 11 | Bounded verifiers: Kani harnesses with cover checks | The harnessed predicate holds for every input within the harness's bounds, and the harness is not vacuous | Predicates not harnessed; assumptions the harness states; anything about the callers | Adopted in `03-assessment-kernel` (ADR 0015 note); run by hand and recorded, not yet in CI |
| 12 | Environment conformance: port suites on Linux, failure injection | An adapter honors its port's observable contract on the tested host | Hosts, kernels, and daemons not tested | Not adopted; `07-substrate-conformance` |

Coverage is a measure of what ran, not of test quality: a line executed by a test with no assertion is covered and unchecked. No coverage number appears in the matrix.

## What Is Not a Pass

A result is recorded as `not-run`, `inconclusive`, or `failed` when any of these holds, whatever color the terminal showed:

- A check did not run, or ran on a tree other than the one the record names.
- A check timed out or was killed. That is `inconclusive`.
- A postcondition was loosened, an `assume`, `admit`, or axiom was added, a test was ignored, a mutant was skipped, a lint was allowed, or code moved out of the verifier's view, and the commit did not declare it (rule 11; `check-trust-boundary`).
- A test the generator wrote is offered as evidence for a property. It is a regression test (ADR 0015 §5).
- A receipt's `passed` has no evidence of a zero exit, or its command is not the registered one (`receipts validate`).
- A mutation score is offered instead of a survivor classification (ADR 0015 §8).
- A finite-model check is called a proof, or a model result is offered as a result about the implementation or the host.

## Policies

### Counterexamples Become Fixtures

A counterexample found by any layer is minimized and committed as a named regression fixture. The fixture carries its provenance in a header or a sidecar, with these fields:

| Field | Meaning |
| --- | --- |
| `found_by` | The layer and tool: a property test, TLC, a fuzzer, a mutation run, a person |
| `seed` or `trace` | The seed and generator version, or the model trace, that produced it |
| `input` | The minimized input, in a form the test loads |
| `property` | The invariant or property violated, by name |
| `fix` | The commit that made the fixture pass |

A seed alone is not a fixture: a changed generator changes what a seed produces. Fixtures live beside the test that loads them, under the package's `tests/` or `src/`, and the root `tests/fixtures/` holds only data the gates load.

### Metamorphic Testing

A metamorphic relation is a necessary property over related inputs, stated from an invariant before the tests exist. Each kernel milestone lists its relations in its result card. The first two:

- **Permutation (N12).** Reordering the resources of a Canon, or the insertion order of a map that feeds planning, does not change the compiled artifact or the Plan.
- **Indeterminate monotonicity (N13).** Adding an Indeterminate Assessment to a report adds no Action to the Plan, and changing a Variance to Indeterminate never adds one.

A set of relations is not an oracle, and the record says which relations were checked.

### Differential Testing

Where a kernel function has a small formulation whose correctness a reader can check by inspection, it is written that way, in its own module, by a different route from the production one: a brute-force search where production uses an index, a recursive definition where production uses a loop. The two are compared on generated inputs. Disagreement is a finding about one of them, investigated and recorded, and never resolved by editing the reference until it agrees. The record states what the generators produce and what they cannot, because that is the blind spot ([Cedar](../research/2026-09-28-generator-verifier/framework.md#verification-guided-development) missed every bug behind malformed input for exactly this reason).

### Generative Variability

The `generator-variance` harness asks the same kernel property of several independent generations, model and human, and judges every candidate with the same fixed oracle. It measures how often a candidate that its own generated tests accept is rejected by the fixed oracle, and how much the candidates differ. It is exploratory: its result never gates, never sets a threshold, and is recorded as a research card, not in the matrix.

### Mutation Testing

`cargo-mutants` is configured in `.cargo/mutants.toml` and runs on a schedule and by hand. The weekly run mutates only the Rust lines changed in the last eight days; a run by hand mutates the whole workspace in four shards. Mutation runs leave out the repository-corpus test, which repeats `cargo xtask mutants semantic` and would make every mutant pay for every semantic mutant; the [mutation-calibration record](../research/2026-09-28-typed-core/results/mutation-calibration.md) shows the exclusion lost no catch. Every survivor is classified:

| Class | Meaning | Action |
| --- | --- | --- |
| `caught` | A test failed | None |
| `unviable` | The mutant did not compile | None; excluded by pattern when systematic |
| `equivalent` | The mutant cannot be distinguished by any test | Recorded with the reason |
| `excluded` | The mutant is outside the target, or in message text nobody asserts | Recorded with the reason |
| `survived` | A test should have failed and none did | A test to write, in its own verifier commit |
| `inconclusive` | Timeout or an operational failure | Rerun once; then recorded as inconclusive |

No score is a target. The [semantic-mutant corpus](../../tests/semantic-mutants/README.md) is the other half: wrong behaviors chosen from the invariants, each with the test that must catch it.

## Test Taxonomy

A test is **evidence-bearing** when something outside the code cites it as the oracle for a claim: a semantic mutant names it as the test that must fail ([the corpus](../../tests/semantic-mutants/README.md)), and the [oracle map](oracle-map.md) lists each by invariant. Everything else is a **regression test**: it records what the code does, and may be rewritten with the code (ADR 0015 §5). Fixtures and negative controls are inputs to evidence, not tests.

| Class | Where it lives | Who may change it without a declaration |
| --- | --- | --- |
| Evidence, integration | `tests/` at the top level, and `crates/*/*/tests/` | Nobody: a verifier path, so the commit says `Trust-Boundary: verifier` and touches no implementation |
| Evidence, inline | A `#[cfg(test)]` module in a production source file, named by the corpus | Nobody: `verification/evidence-oracles.toml` holds the SHA-256 of its whitespace-normalized text, and `cargo xtask check-evidence-tests` fails on a change |
| Regression | Any other inline `#[cfg(test)]` module | The generator |
| Fixture, negative control | `tests/fixtures/`, `tests/semantic-mutants/` | Nobody: a verifier path |

Where a new evidence-bearing test belongs: in an integration test directory when it can be written against the public surface, since the path then protects it; inline, with a pin added in the same verifier commit, only when it needs private access. A pin guards the text of the test, not what it calls: an edit to the code under test is implementation, and the semantic mutants judge it.

The check reports `[evidence-oracle-changed]` for a pinned test whose text changed, `[evidence-oracle-unpinned]` for an inline test the corpus names that has no pin, and `[evidence-oracle-missing]` for a pin whose test no longer exists. It also fails when the [oracle map](oracle-map.md) differs from the one it generates.

## Release Admission

A release is a tag `v<version>` on a commit that passed the same checks as every commit. Those checks live in the repository, so they cannot authenticate the repository: if the tagged commit is the head of `main`, a check whose base is `main` compares the commit with itself and passes whatever it holds. Three rules close that.

1. **The host enforces the outer boundary.** `main` is protected, requiring the designated CI checks before a change is accepted and refusing force pushes, and tags matching `v*` can be created only by the owner and never moved or deleted. The rules are in [docs/release-process.md](../release-process.md), as importable JSON. Nothing in this repository can check that they are on.
2. **The trusted base of a release is the previous release.** On a tag, `check-trust-boundary` and `research frozen` take the nearest earlier `v*` tag as their base. `check-release-base --base <tag>` refuses, with a code, when:

   | Code | When |
   | --- | --- |
   | `release-base-unresolved` | The base cannot be resolved |
   | `release-base-not-a-tag` | The base is not a `v*` release tag |
   | `release-base-is-head` | The base is the tagged commit itself |
   | `release-base-not-ancestor` | The base is not an ancestor of the tagged commit |
   | `release-not-on-main` | The tagged commit is not reachable from `main` |

   A release job whose base cannot be resolved fails.
3. **A release publishes what was validated.** The package is built once, and its SHA-256 is computed at once. The Debian 12 and 13 suites run that file, not a rebuild, and record its digest. The release attaches that same file, and `RELEASE-EVIDENCE.json` ties the tag to the commit, the package's digest, each suite's record, and the published digest. `check-release-chain` fails when any of them differs, and when a suite did not run the package. The release job also attaches a provenance attestation naming the repository, the workflow, the commit, and the digest.

The release job's third-party actions are pinned to full commit SHAs, and `cargo xtask check-workflow-pins` fails on any that is not.

### Rules Older Than the Range

A release range can hold commits written before a protected-path rule existed, because the base is the previous release and a rule can land inside the range. The first alpha showed it: the integration tests of every crate became a verifier path after three commits of the second alpha were written, and the tag check, which applies the policy at the tag to every commit since the previous release, rejected them. History on `main` cannot be rewritten, so a release would have been impossible by construction.

A path pattern in `verification/trust-boundary.toml` can therefore carry a `since` commit, in a `[since]` table that maps the pattern to a full commit SHA. The rule is:

1. **Only a release check reads it.** `check-trust-boundary --base <tag> --release` applies a pattern with a `since` commit to a commit only when `since` is that commit or one of its ancestors. For any other commit the pattern is treated as not listed, and a path it would have matched is classified by the sets that follow it, as it was before the rule existed. Every other pattern, and every other check, is unchanged.
2. **Every other check is strict.** A pull request and a push to `main` take every pattern on every commit, whatever `since` says. A branch started before the rule therefore cannot carry an undeclared change past review: the pull request that proposes it is judged by the rule in force.
3. **A check that cannot tell fails.** `--release` fails with `[trust-release-base-not-a-tag]` when its base is not a `v*` release tag, and the check fails with `[trust-since-unresolved]` when a `since` commit is not in the repository. Neither is a pass.

This narrows what a release check inspects: a commit that does not descend from a rule's `since` commit is not judged by that rule on a release. It does not narrow what a commit faces when it is proposed, which is why the exemption is limited to releases. The commit that adds a rule, and every commit after it on the same line of history, is judged by it.

## Records

A layer's result exists when a receipt or a result record exists. Receipts are written by `cargo xtask receipts record` under `verification/receipts/`; result records are cards under `docs/research/<snapshot>/results/`. The [matrix](verification-matrix.md) is filled from those and from nothing else. Its statuses are `not run`, `planned`, `passed`, `failed`, `inconclusive`, and `not applicable`; `planned` is a promise, not a result.
