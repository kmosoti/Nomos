# Generator-Variance Candidates

These are the frozen inputs of experiment `generator-variance` ([design and result](../../../docs/research/2026-09-28-typed-core/results/generator-variance.md)). `cargo xtask generator-variance` reads every `*/candidate.rs` here by default.

| Path | Provenance |
| --- | --- |
| `pack.md` | The specification pack every candidate was generated from: spec excerpts and type definitions, with no function bodies and no tests |
| `cand-1/` to `cand-5/` | Generated on 2026-09-29 from `pack.md` by five separate agent sessions, with no shared context, no access to the repository, and no access to one another |
| `cand-0/` | The negative control, written by the harness author with the reference in view. It carries the `SM-ASSESS-001` defect and tests that agree with it |
| `reference-assessment.rs` | The reference the recorded run judged against, `crates/core/nomos-core/src/assessment.rs` at commit `576d50f`, copied on 2026-09-30 when `08-resource-families` generalized the core; its Secure Hash Algorithm (SHA) 256 digest is the record's `reference_sha256`, which a harness test checks. The harness's own tests read it; a run checks the workspace out at that commit |

Each directory holds `candidate.rs`, the candidate's three functions and its own `#[cfg(test)] mod tests`, and `NOTES.md`, the author's stated interpretive choices. A generated candidate's files are as the session returned them. The negative control was formatted with `rustfmt` before it was frozen.

The files are evidence. Never edit a candidate to make it pass, compile, or agree; a new candidate goes in a new directory, and a revised experiment is a new record.
