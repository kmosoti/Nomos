# Semantic Mutants

A semantic mutant is a hand-written patch that makes the code wrong in one way an invariant forbids, paired with the one test that must notice. It runs the other way from `cargo mutants`, which mutates syntax and asks whether any test notices. Here the wrong behavior comes first, from the invariant, and the test is named before the patch exists.

`cargo xtask mutants semantic` applies every active patch to a scratch copy of the workspace and runs only the named test. The run passes when every active mutant is `caught`. The runner's outcomes, and what each means, are in the module docs of `crates/bin/nomos-xtask/src/semantic.rs`.

## Corpus Format

`corpus.toml` holds one `[[mutant]]` table per entry:

| Field | Meaning |
| --- | --- |
| `id` | Stable identifier, `SM-<KERNEL>-<NNN>`. Never reused. |
| `property` | The invariant or property threatened, by its name in [invariants.md](../../docs/formal/invariants.md). |
| `threat` | What the invariant forbids, in one sentence. |
| `wrong_behavior` | What the patched code does instead. |
| `crate` | The workspace package the named test lives in. |
| `patch` | Unified diff under `patches/`, applied at the workspace root with `git apply`. |
| `test` | The one test that must fail, as a `cargo test` filter matched exactly. |
| `status` | `planned` (listed, not run), `active` (run, must be caught), or `retired` (kept for the record, not run). |

A mutant is `active` only with a patch that applies and a test that exists. A `survived` outcome is a finding against the test, not the code: the test does not test what its name claims. The fix is a stronger test, never a weaker mutant.

## Why the Corpus Exists Before the Kernels

Every entry is `active` and caught: the Assessment, Warp, and Transition Kernels' seventeen, and the Canon artifact's seven (`SM-CANON-001` to `007`). Each milestone names here the wrong behaviors it must be shown to reject before it is complete. Writing the mutant before the code is the point: it is the invariant's own negative control, chosen from the specification rather than from whatever the implementation happened to make easy to mutate.
