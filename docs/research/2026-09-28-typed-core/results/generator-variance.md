# Result: generator-variance

- **Experiment.** `generator-variance`, designed in milestone `02-verification-foundation`.
- **Status.** **Not run.** There is no kernel function to ask for. This record is the design, written so the harness exists before the first candidate does, and it is exploratory: its result never gates and no threshold is set.
- **Question.** When several independent generations implement one kernel property, how often does a fixed oracle reject a candidate its own generated tests accept, and how much do the candidates differ?
- **Rival.** Generated tests and the fixed oracle agree, and [ADR 0015](../../../adr/0015-generator-verifier-development-model.md) §5's rule that generated tests are regression tests is unnecessary caution.

## Why

The digest's oracle results are paper-reported, for Java and Python, with older models: oracles biased toward the implementation shown, tests that compile 42% of the time, property-based tests correct for 21% of documented properties. None of it is a Nomos measurement. This experiment produces one, on one property, and reports it as a research finding.

## Harness

1. **Property.** One pure kernel function with a fixed oracle that predates every candidate: the exhaustive truth table for `assess` on the first resource family, from `03-assessment-kernel`, plus the semantic mutants `SM-ASSESS-001` and `SM-ASSESS-002`.
2. **Candidates.** $n \ge 5$ independent generations of the function from the same specification text: separate sessions, no shared context, at least one human-written. Each generation also writes its own tests, as it would in ordinary work.
3. **Normalization.** `cargo fmt` on every candidate.
4. **Judgment.** Every candidate is run against its own tests, against the fixed oracle, and against the active semantic mutants (its own code patched). Each result is recorded with a receipt.
5. **Variance.** For candidates that pass the fixed oracle, record the differences that remain: lines, public shape, and which unlisted inputs they disagree on under a generated-input comparison.

## Measurements

| Measure | Meaning |
| --- | --- |
| Own-test acceptance | Candidates accepted by the tests they wrote |
| Oracle acceptance | Candidates accepted by the fixed oracle |
| Disagreement rate | Accepted by own tests, rejected by the oracle |
| Mutant catch rate per candidate's tests | Whether the generated tests would have caught the semantic mutants |
| Residual variance | Differences among oracle-accepted candidates on generated inputs |

## Negative Control

One candidate is written to be wrong in a way the oracle names (the `SM-ASSESS-001` behavior) with tests that agree with it. The harness must report it as own-tests-accepted and oracle-rejected. If it does not, the harness is not measuring.

## Decision Rule

None that gates. The rates are recorded here, with their $n$, and the digest's open question on generated-test bias is updated. If the disagreement rate is zero on $n \ge 5$, that is a finding worth recording as well, and ADR 0015 §5 stays as written until it recurs.

## Unchecked, When It Runs

- Properties other than the one chosen.
- Generators other than the ones used, on other days.
- Anything about candidates that do not compile; they are counted and set aside.
