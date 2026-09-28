# Result: agent-proof-gate

- **Experiment.** `agent-proof-gate`, milestone `02-verification-foundation`.
- **Question.** Does a mechanical gate catch a commit that changes a specification or a verifier without saying so, or adds a known escape hatch?
- **Outcome.** Yes, for the detectable part. Semantic weakening inside a declared change is a review item, as [ADR 0007](../../../adr/0007-verification-gates.md) §5 said it would remain. Recorded in [ADR 0015](../../../adr/0015-generator-verifier-development-model.md) §3.

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1; Git 2.43 |
| Harness | `cargo xtask check-trust-boundary --base <ref>` over `tests/fixtures/agent-proof/`; each case is one commit made in a scratch repository from the fixture base |
| Policy | `verification/trust-boundary.toml` |
| Receipt | `verification/receipts/2026-09-28-verification-foundation.ndjson`, `check-trust-boundary`, over this branch's own commits |

## Cases

19 tests. Positive controls: an implementation-only commit; a declared specification change; a declared verifier change; a declared escape hatch; a lint allowed only under `cfg_attr(test, ...)`. Range behavior: every commit in the range is checked and named; a merge commit is skipped while its constituent commits are not; an unavailable base fails closed. Negative controls, each asserted by code and by the class and path it names:

| Code | Fixture commit |
| --- | --- |
| `undeclared-oracle-change` (specification) | `undeclared-spec-change`: `docs/PROJECT-SPEC.md` edited, no declaration |
| `undeclared-oracle-change` (verifier) | `undeclared-verifier-change`: the gate's source edited, no declaration |
| `undeclared-oracle-change` (verifier) | `weakened-gate`: a gate's test assertion replaced by a tautology, no declaration |
| `undeclared-oracle-change` (verifier) | `removed-ci-step`: a CI step deleted, no declaration |
| `mixed-oracle-and-implementation` | `mixed-spec-and-implementation`: specification and core changed together, declared |
| `escape-hatch-added` | `ignored-test`: `#[ignore]` added to a core test, no declaration |
| `trailer-without-change` | `trailer-without-change`: `Trust-Boundary: verifier` on an implementation-only commit |
| `unknown-trust-boundary-class` | `unknown-class`: `Trust-Boundary: implementation` |

The branch that built the gate was then checked by it: every commit since `origin/main` declares its class, or touches nothing protected.

## What the Gate Sees and Does Not

The `weakened-gate` case is the honest one. The gate names it because a verifier path changed without a declaration, not because it read the assertion. The same weakening inside a commit that declares `Trust-Boundary: verifier` passes the gate and reaches the reviewer, which is the design: the gate makes oracle changes visible by name, and a person judges them. The escape-hatch scan is a substring match against the patterns in the policy, with a per-pattern exemption for test-scoped lint allowances. It does not parse Rust.

A path block was considered and rejected: the generator can write any file, and a block that pretends otherwise would be routed around and would teach the wrong lesson.

## Unchecked

- Semantic weakening inside a declared commit.
- A squash merge that drops the declaration lines; the repository merges with merge commits, and the open-questions file records the assumption.
- Escape hatches not in the policy's list; the list grows with each verifier adopted.
- Whether the discipline holds up under a generator optimized against it; the catalogued behaviors in the digest came from such settings, and this gate has only been run against fixtures and this branch.

## Decision

The gate runs in CI on every push and pull request. Rule 11 keeps its review clause. The verification matrix records this row as `passed` for the fixtures and the branch, with the unchecked list above.
