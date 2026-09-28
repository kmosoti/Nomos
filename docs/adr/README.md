# Architecture Decision Records

| ADR | Title | Status |
| --- | --- | --- |
| [0000](0000-foundations.md) | Foundations, hexagonal layout, and pinned toolchain | Accepted |
| [0001](0001-headscale-mesh-adapter.md) | Headscale as the preliminary Mesh adapter | Accepted |
| [0002](0002-vault-cipher-adapter.md) | HashiCorp Vault as the preliminary Cipher adapter | Accepted |
| [0003](0003-xtask-tooling-crate.md) | Development tooling lives in the `nomos-xtask` crate | Accepted |

A new ADR takes the next number and uses the same layout: Status, Date, Context, Decision, Consequences.

## Candidates

The 2026-09-28 research snapshot proposes eight decision bundles. None is an ADR yet. Each becomes one, with the next number, when the experiments that feed it have run; the mapping from candidate to experiment is in the [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md).

| Candidate | Decides | Waits on |
| --- | --- | --- |
| `verification-gates` | Verification scope, the layer-policy check, and protection of specifications from being weakened to pass a check | Wave 0, wave 4 |
| `evidence-model` | Per-resource assessment with a third outcome, resource ownership across controllers, identity | Wave 1, wave 4 |
| `warp-gates` | Edge semantics (spec §62), satisfaction anchors, durable change consumption, reservations | Wave 1, wave 2 |
| `recovery-authority` | Effect recovery contracts, fencing at the effect boundary, admission-scoped budgets | Wave 2 |
| `canon-artifact` | Canon authoring surface, validated decoding, algebraic resource model, canonical encoding profile | Decision D1, wave 3 |
| `event-history` | Outbox, retention, and disk-pressure behavior | Phase 2 |
| `boundary-security` | Substrate privilege boundary and Cipher sinks | Phases 1 and 6 |
| `future-algorithms` | Incremental planning and Plan witnesses | After Phase 0, measured first |
