# Architecture Decision Records

| ADR | Title | Status |
| --- | --- | --- |
| [0000](0000-foundations.md) | Foundations, hexagonal layout, and pinned toolchain | Accepted |
| [0001](0001-headscale-mesh-adapter.md) | Headscale as the preliminary Mesh adapter | Accepted |
| [0002](0002-vault-cipher-adapter.md) | HashiCorp Vault as the preliminary Cipher adapter | Accepted |
| [0003](0003-xtask-tooling-crate.md) | Development tooling lives in the `nomos-xtask` crate | Accepted |
| [0004](0004-rust-typed-canon.md) | Canon is authored in Rust and shipped as an inert Canonical IR | Accepted |
| [0005](0005-assessment-vocabulary.md) | Condition, Observation, Assessment, Indeterminate, Obligation, and Settled | Accepted |
| [0006](0006-kernel-contract.md) | The kernel is a pure transition function and owns reconciliation semantics | Accepted |
| [0007](0007-verification-gates.md) | Verification scope and development gates | Accepted |
| [0008](0008-ownership-and-identity.md) | Resource ownership, controller composition, and identity | Proposed |
| [0009](0009-warp-activation-semantics.md) | Warp prerequisite, activation, and reservation semantics | Accepted |
| [0010](0010-effect-recovery-and-fencing.md) | Effect recovery, authority fencing, and disruption budgets | Accepted for the Cell; `fence-interleavings` owed |
| [0011](0011-canon-artifact-encoding.md) | Canon artifact encoding, identity, and versioning | Accepted |
| [0012](0012-event-history.md) | Event history, durability, and retention | Proposed |
| [0013](0013-trust-boundaries.md) | Substrate and Cipher trust boundaries | Proposed |
| [0014](0014-deferred-planning-algorithms.md) | Incremental planning and Plan witnesses wait for measurement | Accepted |
| [0015](0015-generator-verifier-development-model.md) | Generator-verifier development model | Accepted |
| [0016](0016-core-purity.md) | Core purity | Accepted |
| [0017](0017-local-store.md) | The Cell's local store: content and a durable Event Log | Proposed |

A new ADR takes the next number and uses the same layout: Status, Date, Context, Decision, Consequences.

## Statuses

- **Proposed.** The direction the documents follow as a working definition. It does not bind the implementation. Its Acceptance Criteria section names the experiments whose result records would accept it, and until they exist it stays Proposed.
- **Accepted.** Binding. A decision is accepted when the project owner made it, or when the evidence it needs exists. The ADR says which.
- **Superseded** or **Rejected.** Kept, with a pointer to what replaced it. Accepted ADRs are amended only by a dated note or a successor.

## From Research Candidates

The 2026-09-28 research snapshot proposed eight decision bundles. Each is now a numbered ADR.

| Candidate | ADR | Status |
| --- | --- | --- |
| `verification-gates` | [0007](0007-verification-gates.md), [0015](0015-generator-verifier-development-model.md) | Accepted: the gates exist with their negative controls; 0007 amended 2026-09-28 by 0015 |
| `evidence-model` | [0005](0005-assessment-vocabulary.md), [0008](0008-ownership-and-identity.md) | Assessment accepted; ownership and identity proposed |
| `warp-gates` | [0009](0009-warp-activation-semantics.md) | Accepted 2026-09-28 on the evidence of `04-warp-kernel` and `05-transition-kernel` |
| `recovery-authority` | [0010](0010-effect-recovery-and-fencing.md) | Accepted for the Cell 2026-09-28 on the evidence of `05-transition-kernel`; Loom transport waits for `fence-interleavings` |
| `canon-artifact` | [0004](0004-rust-typed-canon.md), [0011](0011-canon-artifact-encoding.md) | Both accepted; the encoding is deterministic Concise Binary Object Representation (CBOR), 2026-09-29 |
| `event-history` | [0012](0012-event-history.md) | Proposed until Phase 2 |
| `boundary-security` | [0013](0013-trust-boundaries.md) | Proposed until the Linux adapter and Phase 6 |
| `future-algorithms` | [0014](0014-deferred-planning-algorithms.md) | Accepted: deferred until measured |
