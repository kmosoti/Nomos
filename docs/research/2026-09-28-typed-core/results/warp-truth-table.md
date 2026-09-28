# Result: warp-truth-table

- **Experiment.** `warp-truth-table`, milestone `04-warp-kernel`.
- **Question.** Do Startable, group-level Activated, and satisfaction anchors fully specify the v0 dependency semantics, with every case resolving to exactly one state and no fallthrough?
- **Outcome.** Yes, for the frontier over a compiled graph, with two working definitions recorded in [ADR 0009](../../../adr/0009-warp-activation-semantics.md)'s note. The tables, the laws against an independent reference, three Kani harnesses, and three semantic mutants agree. ADR 0009 stays Proposed until `05-transition-kernel` runs its second criterion.

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1; proptest 1.11; Kani 0.68.0; cargo-mutants 27.1.0 |
| Crate | `nomos-warp`, `no_std` with `alloc`, depends on `nomos-core` only; allowlist empty, no `petgraph` |
| Production functions | `Graph::compile`, `frontier`, `resolve_requires`, `resolve_after`, `resolve_group`, `resolve_vertex`, `select` |
| Receipts | `verification/receipts/2026-09-28-warp-kernel.ndjson` |

## What Was Built

`Vertex::from_assessment` gives a Variance an Action vertex, Satisfied a satisfaction anchor, and Indeterminate an Indeterminate anchor. `Graph::compile` takes vertices and edges of the three kinds, rejects a duplicate vertex or an edge to a resource with no vertex, collapses duplicate edges, orders the graph with Kahn's algorithm over an ordered queue, and on a cycle returns one witness per cyclic strongly connected component, found by breadth-first search from the component's smallest vertex, with the acyclic remainder reported as blocked. `frontier` walks the order once, classifies each source as it goes, and resolves every pending vertex to Ready, Waiting, Blocked, or Skipped from its `requires` edges, `after` edges, and `on_change` group. `select` is the greedy conflict-key pass with a capacity; failure-domain budgets are a slot with no policy.

## Working Definitions Recorded

- **A Skipped source is met.** A vertex Skipped because its group is Disabled counts as a met prerequisite for `requires`, terminal for `after`, and unchanged for `on_change`. The text of warp.md said "not failed, terminal without change, so its own dependents see it that way" without saying what `requires` sees; the alternative reading would block every prerequisite chain through a refresh with no reason to run.
- **An `after` edge has no Blocked state.** `AfterState` is a separate two-state type. The first version shared `EdgeState` with `requires`; the Kani harness on `resolve_vertex` produced an `after` edge in state Blocked, which `resolve_after` never returns but the type admitted, and the vertex resolved Ready with an unmet edge. The type now makes the row unwritable, and the harness verifies.

## Evidence

| Layer | Harness | Result |
| --- | --- | --- |
| 4, truth tables | `the_edge_truth_table_is_exhaustive`: ten source states, the Indeterminate anchor and the Blocked and Skipped resolutions included, against `requires` and `after`; `the_group_truth_table_is_exhaustive`: every single source, then the mixed cases | Passed; every `after` row is Waiting or Satisfied |
| 4, unit | Failed requirement blocks and propagates; failed `after` does not block; running source waits; one-changed-one-unchanged activates; unchanged configuration skips; failed source does not activate; Indeterminate anchor per edge; satisfaction anchor per edge; Skipped source per edge; $A \leftrightarrow B$, $B \to C$ one witness and $C$ blocked; `after` and `on_change` cycles rejected; self-loop; two witnesses; order ties by resource; conflicting Actions never together; held keys exclude; capacity counts the reserved set | 29 tests passed |
| 6 and 7, laws with a reference | `tests/laws.rs`, 512 cases each, ChaCha seeds `4e3401` to `4e3404`, graphs of six resources and up to eight edges of every kind, cycles included; the reference detects cycles by depth-first coloring and computes the frontier by whole-graph passes to a fixed point | 4 laws passed on the first run: compilation agrees with the reference on cycles and every witness is a cycle; graph and frontier are permutation-invariant; the frontier agrees with the reference on every pending vertex and every Ready vertex has met requirements; selection is mutually exclusive, bounded, prefix-maximal, and deterministic |
| 11, Kani | `edge_resolution_matches_the_table`, `group_resolution_matches_the_table` (three sources), `vertex_resolution_is_ready_only_when_startable_and_activated` (two edges of each kind) | 3 harnesses verified, 8 of 8 covers satisfied, 7.5 s; the vertex harness failed on the first version and led to `AfterState` |
| 8, semantic mutants | `SM-WARP-001` to `SM-WARP-003` | Each caught by its named test |
| 8, `cargo-mutants` | `cargo mutants -p nomos-warp`, 52 s | 96 mutants: 53 caught, 40 unviable, 3 survived, classified below |

The laws' reference is written by another route on purpose and was not edited during the run; the generators cover six-resource graphs with every edge kind and every progress value, and do not cover Obligations, budgets, or larger graphs.

## Survivor Classification

| Mutant | Class | Disposition |
| --- | --- | --- |
| `ConflictKey::as_str` returning `""` or `"xyzzy"` | **survived**: text nobody asserted | `keys_and_errors_say_what_they_are` now asserts it |
| `CompileError`'s `Display` returning `Ok(())` | **survived**: text nobody asserted | The same test asserts each variant's text names its subject |

The unviable mutants are `Default::default()`, `Box::leak`, and iterator replacements on types without those impls, and four `vec![]` replacements that do not compile because the crate is `no_std` and the macro is not imported at those sites. They carry no information.

## Negative Controls

- Every failed, timed-out, cancelled, rejected, and Indeterminate source blocks its `requires` dependent, in the table, in law 3, in Kani, and by `SM-WARP-001`.
- A failed source satisfies an `after` edge (`SM-WARP-002`) and never activates a group.
- An `after` cycle is a cycle (`SM-WARP-003`); the $A \leftrightarrow B$, $B \to C$ graph yields one witness and one blocked vertex.
- The Kani vertex harness fails on the first version's shared edge type.

## Unchecked

- Obligations as vertices, failure-domain budgets (N9), and derived footprints for `package` and `systemd_unit`: `05-transition-kernel`.
- The lifecycle side of N4, that Running follows Ready: the frontier decides Ready; nothing here dispatches.
- Graphs larger than six resources under the laws; the algorithms are the standard ones and the unit tests cover the small shapes that matter, which is not the same as coverage.
- Fairness and maximality beyond the prefix property; warp.md claims neither.
- The recursion in the component search is bounded by the vertex count of a Canon and is not itself tested for depth.

## Decision

The frontier semantics of warp.md are implemented and checked as stated, with the two working definitions above for ADR 0009 to accept or amend. `05-transition-kernel` takes `Frontier::ready` and `select` as the inputs to dispatch.
