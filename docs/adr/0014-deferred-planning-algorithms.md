# ADR 0014: Incremental Planning and Plan Witnesses Wait for Measurement

- **Status.** Accepted
- **Date.** 2026-09-28
- **Candidate.** `future-algorithms` in the 2026-09-28 research snapshot (recommendations `incremental`, `plan-witness`)

## Context

Two ideas from the research are attractive. The first is tracked incremental recomputation, the Salsa model, so that a change to one Condition does not re-plan a whole Canon. The second is an independent checker for a compact Plan witness, so that an optimizing planner never has to be trusted. Neither has anything to compare against yet. Spec §54 already says sophisticated algorithms are chosen by experiment, not by the attractiveness of their papers.

## Decision

Neither is adopted now, and each has a stated condition for reconsideration.

1. **Full recomputation first.** Nomos plans by complete deterministic recomputation from the Canon, the Assessments, and the Obligations. That path is the reference every optimization is compared with.
2. **Incremental planning waits for an ablation.** It is adopted only when experiment `incremental-ablation` shows zero semantic disagreement with the full path, under randomized invalidation and dropped notifications, and a measured end-to-end benefit that outweighs the cache's complexity. Its keys cover Condition, evidence, capability, policy, and driver versions. Change notifications are invalidation hints, never evidence.
3. **Plan witnesses wait for a baseline planner.** A checker is trialed only after a simple planner exists to check. It is called proof-carrying only once its certificate language and checker theorem are written down. A witness validates the encoded argument, not the truth of the Observations it cites.
4. **No constraint solver without an objective.** Greedy feasible admission ([ADR 0009](0009-warp-activation-semantics.md)) stays until a precise objective and a benchmark show it is insufficient.

### Evidence

Spec §54, and the research snapshot's `defer_until_measured` and `research_required` dispositions for these recommendations. This ADR accepts a sequencing decision, not a technical result.

## Consequences

- No kernel milestone adds incremental or witness machinery.
- **Revisit trigger.** The full recomputation path is correct and profiled, or a planner more complex than greedy selection is proposed.
