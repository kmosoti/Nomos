# ADR 0015: Generator-Verifier Development Model

- **Status.** Accepted
- **Date.** 2026-09-28
- **Provenance.** The [generator-verifier research digest](../research/2026-09-28-generator-verifier/README.md), the milestone specification the project owner supplied for the verification foundation, and the results of the milestone's own experiments: `core-purity`, `mutation-calibration`, and `agent-proof-gate`, recorded under `docs/research/2026-09-28-typed-core/results/`.

## Context

Code in this repository is written, in the main, by language models driving an editor, and reviewed by people. [ADR 0007](0007-verification-gates.md) set the rules for gates: a claim needs a record, a record needs a run, every gate has negative controls asserted by reason. It left three things to review alone. Rule 11's protection of specifications had no mechanical part. There was no record format, so "filled from records only" described a matrix with nothing to fill it from. And nothing said what a test written by the same process that wrote the code is evidence of.

The digest's sources agree on the shape of the problem. A generator satisfies the verifier's letter wherever it differs from its intent (Krakovna et al.; Baker et al. catalogued an agent exiting before its tests ran). Agents completing formal proofs cheated in 2% to 14% of tasks until a syntactic checker for the known cheats was offered (VeruSAGE). Model-written oracles are biased toward what the code does rather than what it should (Konstantinou et al.). Proofs and differential tests found different bugs on the same system, and the generators' assumptions were the blind spot (Cedar). Every one of these is a paper-reported result about other systems; what transfers is the mechanism.

## Decision

### 1. The Asymmetry Is Stated for the Verifier Only

$$\mathit{candidate} \sim \mathrm{Generator}(\ldots)$$

$$\mathrm{Verify}(\mathrm{normalize}(\mathit{candidate}), \mathit{specification}, \mathit{environment}) \text{ is a function}$$

Nothing is claimed about the generator: not determinism, not reproducibility, not that two implementations of one property agree byte for byte. Everything is claimed about the verifier: for a normalized candidate, a frozen specification, and a pinned environment, its verdict is the same on every run. Normalization is `cargo fmt` and the canonical form each gate defines. The environment is `rust-toolchain.toml`, `Cargo.lock`, and the fixtures. Rule 7's determinism requirement on compilation and planning is a separate, domain rule; this one is about the checks.

### 2. Oracles Are Protected Sets

The specification set is `docs/PROJECT-SPEC.md`, `docs/formal/`, `formal/`, and `docs/adr/`. The verifier set is `crates/bin/nomos-xtask/`, `tests/`, `verification/`, `crates/core/PURITY.toml`, `.github/workflows/`, `.cargo/`, the root `Cargo.toml`, and `rust-toolchain.toml`. The implementation set is every other workspace crate. `verification/trust-boundary.toml` is the list, and changing the list is itself a verifier change.

$$\mathrm{GeneratorWriteSet} \cap \mathrm{ProtectedOracleSet} = \varnothing \quad \text{unless declared}$$

### 3. Oracle Changes Are Declared, Detected, and Reviewed

A commit that touches a protected set carries a `Trust-Boundary: specification` or `Trust-Boundary: verifier` line in its message, touches no implementation path, and adds no escape hatch it does not declare with `Trust-Boundary: escape-hatch`. `cargo xtask check-trust-boundary --base <ref>` enforces the declaration over every non-merge commit since the merge base, in CI on every push and pull request, with a stable code per failure: `undeclared-oracle-change`, `mixed-oracle-and-implementation`, `escape-hatch-added`, `trailer-without-change`, `unknown-trust-boundary-class`.

The gate detects; it does not judge. A declared change is a request for review under rule 11, with the change visible under its own name. A path block was considered and rejected as theater: the generator can write any file, and a gate that pretends otherwise teaches nothing. The escape-hatch scan is syntactic and the record says so. A weakened assertion that keeps its shape passes the scan and fails only because it is an undeclared verifier change; a weakening inside a declared verifier commit is the reviewer's to catch.

### 4. The Verifier Set Is Stated as a Ladder

[docs/formal/verification-strategy.md](../formal/verification-strategy.md) lists twelve layers, from formatting to environment conformance, each with what it establishes and what it does not. The cheap layers run first and always; the expensive ones run on a schedule or when there is something to ask them. A layer with nothing to check is not adopted (ADR 0007 §6).

### 5. Generated Tests Are Regression Tests

A test written by the generator that wrote the code is evidence that the code does what the code does. It counts in the verification matrix as evidence for a property only when its oracle comes from outside the code: an invariant stated in `docs/formal/`, an exhaustive truth table, an independently written reference, a metamorphic relation, or a proof obligation. Until then it detects regressions, which is worth having and is not the same thing.

### 6. Differential Testing Uses an Independent Simple Reference

Where a kernel function has a small formulation whose correctness a reader can check by inspection, the reference is written that way, in its own module, by a different route from the production one, and the two are compared on generated inputs. Disagreement is a finding about one of them and is never resolved by editing the reference to agree. The record states what the generators cover, because that is where Cedar's differential testing was blind.

### 7. Metamorphic Relations Are Stated From Invariants

Each kernel milestone states its relations before its tests exist: a permutation of insertion order does not change a Plan (N12); adding an Indeterminate Assessment does not add an Action (N13). Relations are necessary conditions, and the record never calls a set of them an oracle.

### 8. Mutation Testing Is Calibrated, Never Thresholded

`cargo-mutants` runs on a schedule and by hand, never on the required path. Every survivor is classified as caught, unviable, equivalent, excluded, survived, or inconclusive, by a person, and a `survived` on a safety property is a test to write. No score is a target, because a target is a proxy and a proxy is what generators optimize. The narrow calibration on `nomos-xtask` is the first record; its three real gaps are closed and its four equivalents are named.

### 9. Semantic Mutants Precede Their Kernels

`tests/semantic-mutants/corpus.toml` names, per invariant, the wrong behavior a kernel must be shown to reject and the one test that must fail against it. The corpus is written from the specification before the code exists, and `cargo xtask mutants semantic` runs the active entries on the required path.

### 10. Counterexamples Become Fixtures With Provenance

A failing input found by a property test, a model checker, a fuzzer, or a mutation run is minimized and committed as a named regression fixture with its provenance: what found it, the seed or trace, the minimized input, the property violated, and the fixing commit. A seed alone is not a fixture, because a changed generator changes what a seed means.

### 11. A Receipt Is the Unit of Evidence

`cargo xtask receipts record` runs a registered check and writes one NDJSON line with the command, the commit, the toolchain, the lockfile digest, the exit status, output digests, and what the result does not cover. `cargo xtask receipts validate` rejects a `passed` without evidence of a zero exit, a `not-run` with evidence, an unregistered check, or a command other than the registered one. The verification matrix is filled from receipts and result records, and from nothing else. No receipt is signed.

### 12. Completion Reports Are Structured

A task's completion report lists the branch and the base and final commits, the documents and ADRs touched, the commands actually run with their results, the test counts, the negative controls and their codes, the experiments and their results, the receipts written, what remains unchecked, and the decisions left open. A command not run is listed as not run. The evidence a report cites is what ran and what changed: commands, receipts, diffs, and test names. It never includes, and no reviewer asks for, a transcript of the generator's reasoning; reasoning is not evidence, and asking for it invites a narrative in place of a record. AGENTS.md carries the form.

## Not Decided

These were considered and are left open on purpose, each with a pointer:

- **Kani or Verus** for the first bounded harness. Decided by the Assessment Kernel milestone's record (ADR 0007 §6). *Decided 2026-09-28, see the note below.*
- **Mutation-score thresholds.** None, by §8. Reopen only with an argument that survives §8's reasoning.
- **Signed receipts or traces.** Reopen when a gate must trust a receipt produced outside the repository.
- **Proof-carrying Plans.** ADR 0014's revisit trigger.
- **Incremental verification** of only what changed. Reopen when the required path is too slow, with a measurement.
- **Remote attestation** of the environment a check ran in. Out of scope until a Loom exists.
- **A semantic escape-hatch scan.** The open-questions file names the case that would reopen it.

## Note, 2026-09-28: Kani Is the Bounded Verifier

Milestone `03-assessment-kernel` ran two Kani harnesses with cover checks over the pure assessment predicates, on the pinned toolchain, in under seven seconds, with every cover satisfied ([record](../research/2026-09-28-typed-core/results/assessment-algebra.md)). Verus was not tried: it needs its own toolchain and a specification language beside the Rust, and nothing in the kernel yet needs a proof that Kani's bounded exploration cannot give. Kani is therefore the bounded verifier for the kernels, one harness per pure predicate, each with cover checks. The escape hatches it admits, `kani::assume` and an unwinding bound too small to reach the assertion, are in `verification/trust-boundary.toml` and the unwinding bound is a review item. Reopen if a property needs unbounded induction over a recursive structure.

## Note, 2026-10-01: Evidence-Bearing Tests and the Admission of a Release

The first alpha showed two gaps in §2 and §3, both recorded as issues #31 and #32 for `v0.1.0-alpha.2`. They are closed by the rules below, which [verification-strategy.md](../formal/verification-strategy.md) states in full; this note records the decision.

- **The verifier set gains the integration tests of a crate.** `crates/*/*/tests/` is a verifier path. A test there is a black-box check of a crate's public surface, which is what an oracle is, and `crates/bin/nomos-cell/tests/` held every milestone's evidence while counting as implementation, so a generator could change the test that judged its own code without a declaration. Inline `#[cfg(test)]` modules stay implementation-owned: they pin behavior, and most are regression tests (§5).
- **An inline test that something cites as an oracle is pinned.** A semantic mutant names its test (§9). Where that test is inline, `verification/evidence-oracles.toml`, a verifier file, holds the SHA-256 of the test's whitespace-normalized text, and `cargo xtask check-evidence-tests` fails on a changed, missing, unpinned, or stale pin. Editing such a test is therefore a declared verifier commit, with the new pin in it. A path block was rejected in §3 as theater and still is; this one is a detector with a stable code, like the others.
- **A release is checked against the previous release, never against itself.** On a tag `v*`, the trusted base of `check-trust-boundary` and `research frozen` is the previous release tag, and `check-release-base` refuses a base that is not a release tag, is the tagged commit, is not its ancestor, or when the tagged commit is not on `main`. The repository host, not repository code, enforces that `main` and `v*` tags cannot be pushed around: a check that lives in the repository cannot authenticate the repository.
- **A release publishes the bytes it validated.** The Debian package is built once; the Debian 12 and 13 suites run that exact file; the release publishes it. `check-release-chain` refuses when the published digest differs from the digest any suite recorded.

These add detectors; they do not change who reviews. Rule 11 is as it was.

## Note, 2026-10-01: A Rule Added Inside a Release Range

The tag run of `v0.1.0-alpha.2` failed its own `trust-boundary` job, which is the gate working as written on a range it had not been tried on. The base of a release is the previous release, so the range held three commits of #38 that were written before `crates/*/*/tests/` became a verifier path, and the policy at the tag judged them by a rule that did not exist when they were made: all three touched an integration test without a declaration, and two of them also changed implementation in the same commit. They were reviewed and merged under the rules of the day, and `main` cannot be rewritten, so no release could pass.

- **A rule applies from the commit that added it, on a release.** A `[since]` table in the policy maps a protected-path pattern to the commit that added it, and `check-trust-boundary --base <tag> --release` judges a commit by that pattern only when it descends from that commit. [verification-strategy.md](../formal/verification-strategy.md#rules-older-than-the-range) states the rule and its failure codes.
- **Every other check stays strict.** A pull request and a push to `main` ignore `since`. The alternative of applying the exemption everywhere would let a branch started before the rule carry an undeclared change into review, so the exemption is only for a commit that was already merged.
- **What it weakens.** A release check no longer holds commits older than a rule to that rule. It is a trust-boundary change, made in its own commits and declared, and the pull request that carries it is for review.
- **Rejected.** A named list of waived commits: it adds a mechanism that a later edit extends one commit at a time. Skipping the release: it hides the problem in the next one, which has the same shape. Rewriting `main`: forbidden by the repository rules.

## Alternatives

- **Trust the generator's tests.** Rejected. The oracle-bias results are paper-reported and from other languages, and the mechanism that produces them, a shared source for oracle and code, is present here regardless.
- **Block writes to protected paths.** Rejected as theater; §3.
- **Adopt a mutation-score threshold.** Rejected; §8.
- **Require byte-identical output from independent generations.** Rejected. It states determinism for the wrong party, and no source supports it.
- **Adopt every verifier in the ladder now.** Rejected by ADR 0007 §6, unchanged.

## Evidence

- `agent-proof-gate`: 20 tests over 14 fixture commits, each shortcut named by its code; the branch's own commits pass the gate.
- Seven hand-written breaks of the four new gates, each caught by a named test; the table is in the `mutation-calibration` record.
- `core-purity`: 21 tests over 14 fixture cases; the real workspace conforms.
- `mutation-calibration`: 66 mutants on three tooling modules, 31 caught, 28 unviable, 7 survived; after the follow-up tests and the exclusion of the unviable pattern, 41 mutants, 34 caught, 3 unviable, 4 survived and classified.
- Receipts: 13 tests over eight rejection cases and one valid file.
- Semantic mutants: the fixture corpus earns every outcome once; the repository corpus is nine planned mutants.

The records are under `docs/research/2026-09-28-typed-core/results/`.

## Consequences

- Every commit that changes a specification, a gate, a fixture, a policy, or CI carries its declaration, or CI fails. The cost is a line in a message and a separate commit; the benefit is that a reviewer sees oracle changes by name.
- Repository merges are merge commits, so that declarations survive. A squash loses them.
- The matrix stays honest by construction: no receipt, no status.
- A mutation run that finds a survivor produces a classification and possibly a test, never a threshold discussion.
- **Failure behavior.** Every gate in this ADR fails closed with a stable code. A gate that cannot read its policy fails; it does not skip.
- **Revisit trigger.** The first bounded-verifier harness, the first survivor the escape-hatch scan and review both miss, or the first receipt from outside the repository.
