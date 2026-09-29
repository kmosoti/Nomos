# Candidate 0 Notes: the Negative Control

The harness author wrote this candidate by hand, with the reference in view, to be wrong in the one way the fixed oracle names. It is not a generation and is not counted among the candidates.

- **The defect.** When every Observation of the resource failed, `assess` returns `Variance(Missing)` instead of `Indeterminate(CollectionFailed)`. That is the wrong behavior of semantic mutant `SM-ASSESS-001`, and it breaks N13: unknown evidence is treated as noncompliance.
- **Its tests agree with it.** `a_failed_observation_is_a_missing_file` asserts the defect. The other two tests hold for the reference too.
- **What the harness must report.** Own tests accepted, oracle rejected. Any other result means the harness is not measuring.
