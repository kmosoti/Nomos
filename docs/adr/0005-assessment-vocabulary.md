# ADR 0005: Condition, Observation, Assessment, Indeterminate, Obligation, and Settled

- **Status.** Accepted
- **Date.** 2026-09-28

## Context

Spec §3 fixed eight terms and forbade synonyms. Spec §9's driver contract gave `diff` two outcomes: a Variance, or nothing. Two outcomes cannot say "the Cell could not tell". A denied read, a stale observation, two observations that contradict each other, or a service whose loaded configuration nobody can see are neither a match nor a mismatch. Forced into Variance they plan a mutation from ignorance; forced into a match they hide drift behind a permission error.

The 2026-09-28 research snapshot used *Condition*, *Observation*, *Assessment*, and *Indeterminate* to make that argument ([evaluation](../research/2026-09-28-typed-core/README.md), recommendation `evidence-assessment`, counterexample `partial-assessment`). The [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md) recorded naming them as decision D2. On 2026-09-28 the project owner chose to expand the vocabulary.

A review of `main` at `013b9d0` added two further gaps the corrected formal documents had opened without naming: a change can leave a required follow-up effect that no Condition observes (a service refresh after a configuration replacement), and a satisfied Condition does not show that an earlier effect has stopped (the file holds the desired bytes; the earlier writer may still be running). Both need a word, or they get several.

## Decision

Spec §3 gains six terms and one revised definition. One concept, one name, as before.

| Term | Definition | Why it exists |
| --- | --- | --- |
| Condition | A requirement Canon states about one resource | The unit a Cell judges. A Canon is a set of Conditions plus relationships |
| Observation | Evidence about one resource: what was seen, by whom, when, and whether collection succeeded | Provenance and freshness are part of the evidence, not metadata beside it |
| Assessment | The judgment of one Condition against its Observations: Satisfied, Variance, or Indeterminate | Three outcomes, per Condition, never aggregated into one status for the Canon |
| Variance | Evidence-backed difference between a Condition and observed state | Revised from "difference between desired and observed state" so a failed observation cannot be one |
| Indeterminate | An Assessment whose evidence supports neither satisfaction nor Variance, with its reason | Plans no mutation. An Indeterminate Assessment of one resource never erases a Variance of another |
| Obligation | A follow-up effect a completed change requires and no Condition can observe, held durably until discharged | A refresh owed after a file replacement survives a crash only if it is recorded before the replacement |
| Settled | The state of an effect whose outcome is known and which can cause no further change | Reservation release, retry, and Plan supersession wait for settlement, not for a satisfied Condition |

The rejected synonyms join the list in spec §3: no "Requirement" for Condition, no "Check" or "Evaluation" for Assessment, no "Unknown" for Indeterminate, no "Pending action" for Obligation, no "Done" or "Quiesced" for Settled.

### Assumptions

- Three Assessment outcomes are enough for the six initial resources. A fourth, for example "satisfied under a weaker evidence policy", is a policy on Indeterminate, not a new outcome.
- Obligations are few and typed. They are created by drivers in core from a verified change, never by an adapter.

### Alternatives

- **Leave the third outcome unnamed** until wave 1 shows what it carries. Rejected. The kernel types need names now, and an unnamed concept collects synonyms in code review.
- **Reuse Variance for unknown evidence.** Rejected. It is the mistake the counterexample demonstrates.
- **Fold Obligation into Variance** by making "service has loaded revision $n$" a Condition. Preferred wherever a driver can observe the loaded revision; it is then a Condition and no Obligation exists. Obligation covers the services that cannot report it, and the spec should say so rather than pretend every service can.

### Evidence

Counterexamples `partial-assessment`, `lost-refresh`, and `fence-race` in the research snapshot, reproduced by `cargo xtask research reproduce`. None is a test of Nomos code. The verification section names the tests that will be.

## Consequences

- Spec §3, §8, and §9 are amended in the same change. `diff` becomes `assess` with three outcomes. Convergence requires every Assessment Satisfied, every Obligation discharged, and every relevant effect Settled ([reconciliation](../formal/reconciliation.md)).
- N3 is restated over that convergence condition, so a required Obligation discharge is not a violation of the fixed point ([invariants](../formal/invariants.md)).
- Core types carry these names: `Condition`, `Observation`, `Assessment` with variants `Satisfied`, `Variance`, and `Indeterminate`, `Obligation`, and a `Settled` state on effect receipts.
- AGENTS.md rule 3 and CONTRIBUTING.md list the expanded vocabulary.
- **Verification.** Milestone 1, pull request 2, implements the assessment algebra with exhaustive truth tables and property tests; pull request 4 exercises Obligations and settlement in the recovery simulator. Until those run, this ADR decides names and meanings, not behavior.
- **Failure behavior.** An adapter has no channel through which to report "satisfied"; it returns Observations, and core assesses them. A driver that cannot observe a property returns an Observation whose collection failed, which assesses as Indeterminate with that reason.
- **Revisit trigger.** Reopen if the assessment algebra experiment finds a case that three outcomes cannot express without lying, or if Obligations turn out to be needed for every service, in which case the Condition form should be made mandatory instead.
