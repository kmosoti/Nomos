# Result: alpha2-release-integrity

- **Milestone.** `18-alpha2-release-integrity`, the release half of [issue #29](https://github.com/kmosoti/Nomos/issues/29), `v0.1.0-alpha.2`: issues #31 (tag release gates that fail closed), #32 (evidence-bearing tests protected from generator edits), and #33 (one release artifact, built, validated, and published). The rules are in [verification-strategy.md](../../formal/verification-strategy.md), Test Taxonomy and Release Admission, and [ADR 0015](../../adr/0015-generator-verifier-development-model.md)'s note of 2026-10-01, written before the code.
- **Question.** Can a release gate pass by comparing a revision with itself, can the tests that judge the code be edited as if they were the code, and can the package a user downloads differ from the one the Debian suites installed?
- **Outcome.** Not after this change, as far as repository code can establish it. A release is checked against the previous release tag, never against `main`, and a base that cannot be resolved, is not a release tag, is the tagged commit, or is not its ancestor fails the job. The integration tests of every crate are a verifier path, the inline tests the corpus names are pinned by their text, and a release publishes the one package both Debian suites installed, whose digest the evidence file records. The part only the repository host can enforce, protected `main` and protected release tags, is shipped as importable rulesets and **is not established**: see What This Does Not Establish.

## Context

| Field | Value |
| --- | --- |
| Commit | `972e8f6` for every receipt but the last two, and `727ab79`, the merge of `main`, for the second `check-trust-boundary` and for `receipts-validate` |
| Hosts | The development host, Ubuntu 24.04.4; Debian 12 and 13 in privileged containers, for the package install |
| Oracle | The two sections of verification-strategy.md; the alpha 1 tag run's own log, which printed `commits_checked: 0` for `v0.1.0-alpha.1` |
| Harness | `crates/bin/nomos-xtask/src/{release,evidence,pins,trust,package}.rs`, and the agent-proof fixtures under `tests/fixtures/agent-proof/` |
| Receipts | `verification/receipts/2026-10-01-alpha2-hardening.ndjson` |

## What the Alpha 1 Tag Run Showed

The `trust-boundary` job on `v0.1.0-alpha.1` took `origin/main` as its base. The tagged commit was the head of `main`, so the job printed `"commits_checked": 0` and passed: it compared the commit with itself. Every commit of the release had been checked on its pull request, so alpha 1 was not released unverified, but nothing in the tag's own run would have noticed one that was not. `research frozen` had the same base.

## What Was Built

- **`check-release-base`** (#31). On a tag, `trust-boundary` and `research-frozen` take the nearest earlier `v*` tag, found with `git describe`, as their base, and run the check first. It refuses with a stable code:
  `release-base-unresolved`, `release-base-not-a-tag`, `release-base-is-head`, `release-base-not-ancestor`, and `release-not-on-main`. A tag with no earlier release fails the job.
- **`check-evidence-tests`** (#32). `crates/*/*/tests/` is a verifier path, so the integration tests of a crate need a declaration. An inline test the semantic-mutant corpus names is pinned in `verification/evidence-oracles.toml` by the SHA-256 of its text, attributes included, whitespace collapsed; the check reports `evidence-oracle-changed`, `-unpinned`, `-missing`, `-not-found`, and `-ambiguous`, and `evidence-map-stale` when `docs/formal/oracle-map.md`, the generated map from each invariant to its test and what protects it, differs. 18 inline tests were pinned when it was written.
- **Build once, publish the same bytes** (#33). The `package` job builds the package and the upgrade fixture once and computes the digest; the Debian jobs install those files, refuse a digest that differs from the build's, and record the digest they installed; the release job runs `check-release-chain`, which refuses a published digest that differs from any suite's, a suite that built its own package, and a missing or failed suite, writes `RELEASE-EVIDENCE.json`, attaches a provenance attestation, and publishes the same bytes. Every action in the workflows is pinned to a full commit SHA with its tag in a comment, `check-workflow-pins` fails on any that is not, and Dependabot proposes the updates.

## Negative Controls

| Control | Code or result |
| --- | --- |
| The previous release's base, on a history where the tagged commit is the head of `main` and carries an undeclared verifier change: the admission path rejects it, alpha 1's gate, with `main` as its base, inspects nothing and passes, the new gate refuses `main` as a base, and with the previous release as the base it inspects the commit and rejects it again | `tagging_a_rejected_commit_does_not_pass_the_release_gate` |
| The base is a branch; the release itself; a base that cannot be resolved; a tag on a revision `main` does not contain; a base on another line of history | `release-base-not-a-tag`, `release-base-is-head`, `release-base-unresolved`, `release-not-on-main`, `release-base-not-ancestor` |
| An undeclared edit to a crate's integration tests; the same declared; the same changed with its implementation, declared | `UndeclaredOracleChange`; accepted; `MixedOracleAndImplementation` |
| An inline evidence test edited; its attribute removed; a pin missing; a pin with no test; a test the corpus names defined twice; a stale oracle map | `evidence-oracle-changed`, `-changed`, `-unpinned`, `-missing`, `-ambiguous`, `evidence-map-stale` |
| The package replaced after validation; a suite that built its own package; a suite missing or failed | `release-package-digest-mismatch`, `release-suite-did-not-run-package`, `release-suite-missing`, `release-suite-failed` |
| A prebuilt package whose digest is not the build's, or not the one `SHA256SUMS` records | `a_package_that_is_not_the_one_built_is_refused` |
| A workflow with an action that is not a full SHA | `a_floating_action_is_reported_with_its_line` |

These are verifier tests with controls, not semantic mutants: the corpus's wrong behaviors are the Cell's, and these gates have their own negative controls, as the other gates do.

## Findings

- **The first trust-boundary receipt failed, and said so.** The check ran against `origin/main` as it was before the state-ownership pull request merged. This branch carried that pull request's commits, and its own stricter rule, which makes `crates/*/*/tests/` a verifier path, read two of them as undeclared and one as mixing an oracle with implementation: `UNDECLARED [undeclared-oracle-change]` for `state_ownership.rs` twice and `UNDECLARED [mixed-oracle-and-implementation]` for the store's `Cargo.toml`. Those commits had passed under the rule in force when they were written. After the merge, the base contains them, the check passes, and it was recorded again. The failed receipt stays in the file. The rule applies to commits made after it lands, so a branch that began before it carries a base that predates it, and that is why release bases are tags.

- **The alpha.2 tag run failed its own gate, and that was right.** The release range from `v0.1.0-alpha.1` held three commits of #38 written before `crates/*/*/tests/` became a verifier path, and the tag check judged them by the policy at the tag: all three touched an integration test without a declaration and two of them also changed implementation, five violations in all, reproduced locally with `check-trust-boundary --base v0.1.0-alpha.1`. They had been reviewed and merged under the rules of the day, and `main` cannot be rewritten. The gate had been exercised on synthetic histories and on pull requests, never on a real release range, which is where a rule that lands mid-range shows. The fix is in its own specification and verifier commits: a pattern the policy dates with `since` is applied on a release only to commits that descend from the commit that added it, and every other check stays strict. With `--release` the same range passes (28 commits, no violations); without it the same five violations remain, which is the control that a branch started before a rule gets no exemption.

## What This Does Not Establish

- **The repository rules.** Protected `main`, protected `v*` tags, and no force pushes are settings of the repository host. Nothing here can check that they are on: [`.github/rulesets/`](../../../.github/rulesets/) holds them as importable JSON, and [release-process.md](../../release-process.md) says how to apply them. Until the owner applies them, the first criterion of #31 is not met, and the release job, whose base is the previous release, is the only part of the admission this repository enforces.
- **The release job itself**, until a tag runs it. Its steps, the evidence file, and the attestation are tested here as far as their commands go, not as a workflow on a tag.
- **Reproducible builds.** The package is built once, so the bytes validated are the bytes published; building it twice does not give the same bytes, since `dpkg-deb` records the time.
- **That an attestation proves a package is safe.** It binds the repository, the workflow, the commit, and the digest, and nothing about the code.
- **A pin guards the text of an evidence test, not what it calls.** An edit to a helper a test uses is implementation, and the semantic mutants judge it.

## Decision Fed

The remaining alpha.2 issues (#34, #35) build on this: the security model cites the release process and the pinned workflow as mechanisms, and the minimal-host job is part of the chain.
