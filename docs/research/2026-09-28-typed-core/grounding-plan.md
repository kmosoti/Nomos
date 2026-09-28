# Grounding Plan: An Executable Kernel Contract

- **Status.** In progress, 2026-09-28. Four milestones landed; three remain. Six experiments have result records under [results/](results/).
- **Goal.** One typed Canon, one shared assessment model, one deterministic planner, one replayable transition kernel, and a harness that demonstrably rejects broken boundaries and broken semantics.
- **Not the goal.** Another round of adapters, networking, storage, or a testing platform with nothing meaningful to test.

The plan is organized as seven milestones with stable names. A milestone is a branch `milestone/<name>` merged with a merge commit, never a pull-request number: numbers are assigned by the hosting platform and change meaning when a change is split or resequenced. Historical pull-request numbers appear only as history, for example "pull request #2, merged as `0136a93`".

## What Main Establishes Today

| Area | Evidence | Not established |
| --- | --- | --- |
| Build and basic checks | CI green: formatting, Clippy, workspace tests, all `--locked` | Anything about the control system |
| Research tooling | `nomos-xtask` verifies each snapshot as a whole, freezes accepted snapshots, and reproduces seven counterexample models | Convergence, fencing, or adapter correctness |
| Architecture | `cargo xtask check-layers` rejects a forbidden workspace edge in the declared and resolved graphs | Anything about registry crates; see the next row |
| Core purity | `#![no_std]`, panic lints, and `cargo xtask check-core-purity` over `crates/core/PURITY.toml` | Purity of behavior; the crates are still empty |
| Oracle protection | `cargo xtask check-trust-boundary` fails an undeclared specification or verifier change and an undeclared escape hatch | Semantic weakening inside a declared change |
| Records | Receipts under `verification/receipts/`, validated; the matrix is filled from them | Any semantic property: every invariant row is `not run` or `planned` |
| Domain kernel | The assessment algebra for the file family and the Warp graph, frontier, and selection: truth tables, laws on generated inputs against independent references, compile-fail boundaries, five Kani harnesses, six semantic mutants caught | Any transition semantics, Obligations, budgets; every resource family but files |
| Formal verification | Written algorithms and proof sketches; `formal/tla/` holds a README | A checked executable model |

A green build is not a verified engine. The milestones exist to change the right-hand column, one row at a time, with evidence.

## Decisions Already Made

| Decision | Record | Still open |
| --- | --- | --- |
| Canon is authored in Rust and shipped as an inert Canonical IR | [ADR 0004](../../adr/0004-rust-typed-canon.md) | Encoding profile, schema versioning, migration rules |
| Condition, Observation, Assessment, Indeterminate, Obligation, Settled | [ADR 0005](../../adr/0005-assessment-vocabulary.md) | Settlement evidence per resource kind |
| Core owns semantics; the kernel is `step(snapshot, input) = decision` | [ADR 0006](../../adr/0006-kernel-contract.md) | Port shapes as code |
| Tooling is Rust in `nomos-xtask` | [ADR 0003](../../adr/0003-xtask-tooling-crate.md) | None |
| Every gate has a record and negative controls | [ADR 0007](../../adr/0007-verification-gates.md), amended | None |
| Generators are untrusted; oracles are protected and declared; receipts are the evidence; Kani is the bounded verifier | [ADR 0015](../../adr/0015-generator-verifier-development-model.md) | Signed receipts; proof-carrying Plans |
| Core crates are `no_std`, panic-free outside tests, and allowlisted | [ADR 0016](../../adr/0016-core-purity.md) | The first allowlist entry |

Open and not blocking: whether a fixed-`argv` package-manager invocation is allowed under AGENTS.md rule 5 ([ADR 0013](../../adr/0013-trust-boundaries.md)), and edge semantics ([ADR 0009](../../adr/0009-warp-activation-semantics.md), proposed until `04-warp-kernel` and `05-transition-kernel`).

## Rules

- **Design before code.** Each experiment carries a question, a rival hypothesis, a harness, a negative control, measurements, and a decision rule. Code that serves none of them is not part of a milestone.
- **Negative controls.** Every check includes a case the draft or a plausible wrong implementation fails, asserted by its stable code. The seven bundle counterexamples stay as historical models in `nomos-xtask`. New tests call the kernel and assert the corrected behavior.
- **Result records.** An experiment ends with `results/<experiment>.md` in the verification-record form of ADR 0007 §1, plus receipts for the commands that ran. A record is written after the run, never before. No cell in the [matrix](../../formal/verification-matrix.md) says *passed* for planned work; a timeout stays *inconclusive*.
- **Determinism applies to harnesses.** Property tests record their seeds. Generated graphs and fixtures are reproducible from a seed and a version. A flaky harness is a finding about the harness.
- **Vocabulary and invariants.** Code and documents use spec §3 terms as expanded by ADR 0005. A change that touches N1–N12 adds the test for it. A new term or a changed invariant is an ADR.
- **Oracles are protected.** AGENTS.md rule 11, enforced by `check-trust-boundary`. An `assume` in a harness is allowed when explicit, justified, declared as an escape hatch, reviewed separately, and shown non-vacuous.
- **Generated tests are regression tests** until an invariant, a truth table, a reference, or a relation supplies their oracle (ADR 0015 §5).
- **Tooling is Rust.** Anything that regenerates a record is a `cargo xtask` command or a Rust test.

## Milestones

Seven, in order. Each names its exit condition, and the exit is a behavior a test demonstrates, not a file that exists.

### 01-foundation-gates

*Landed 2026-09-28 as pull request #2, merged as `0136a93`.* ADRs 0004 to 0014, the spec amendments, the corrected formal documents, strict whole-snapshot verification shared by the CLI and the tests, `cargo xtask check-layers` over the declared and resolved graphs with 16 fixture cases and one allowed workspace, `cargo xtask research frozen` on every push and pull request, and CI with `--locked` throughout.

Exit, met: a forbidden dependency and a corrupted snapshot each fail the required check for the stated reason, the allowed workspace passes, and restoring the valid input passes again.

### 02-verification-foundation

*Landed 2026-09-28 on branch `milestone/02-verification-foundation`.* The verification framework that every kernel milestone runs on, built before any kernel so that the first kernel is judged by oracles that existed before it did.

- The [generator-verifier digest](../2026-09-28-generator-verifier/README.md) and [ADR 0015](../../adr/0015-generator-verifier-development-model.md): the asymmetry stated for the verifier only, protected oracle sets, declared and detected oracle changes, receipts as the unit of evidence.
- [ADR 0016](../../adr/0016-core-purity.md) and the [core purity contract](../../formal/core-purity.md): `#![no_std]` on the three core crates, panic lints denied, `crates/core/PURITY.toml`, and `cargo xtask check-core-purity` with 14 negative controls.
- `cargo xtask check-trust-boundary` with 13 fixture commits, in CI; `verification/trust-boundary.toml`.
- Receipts: `verification/checks.toml`, `verification/receipt.schema.json`, `cargo xtask receipts record` and `validate` with 8 negative controls.
- The [semantic-mutant corpus](../../../tests/semantic-mutants/README.md), nine planned mutants, and `cargo xtask mutants semantic` with a fixture corpus that earns every outcome.
- `cargo-mutants` calibrated on three tooling modules, `.cargo/mutants.toml`, a scheduled non-blocking CI job, and three tests the calibration found missing.
- The [verification strategy](../../formal/verification-strategy.md) and the [verification matrix](../../formal/verification-matrix.md), the latter populated only with what the two landed milestones established.
- AGENTS.md and CONTRIBUTING.md: the generator-verifier discipline, the completion-report form, and milestone branch naming.

Exit, met: every new gate rejects each of its fixture shortcuts by code and accepts its positive control; the branch's own commits pass the trust-boundary gate; the real workspace passes the purity check; the receipts written during the milestone validate. Result records: [core-purity](results/core-purity.md), [mutation-calibration](results/mutation-calibration.md), [agent-proof-gate](results/agent-proof-gate.md), [generator-variance](results/generator-variance.md) (designed, not run).

### 03-assessment-kernel

*Landed 2026-09-28 on branch `milestone/03-assessment-kernel`.* One resource family first: file presence and content requirements. In `nomos-core`: validated `ResourcePath` and `Digest`, `FileCondition` as a sum type, `Condition` with private fields, `Observation` with provenance, collection window, and collection outcome, `assess` with `Satisfied`, `Variance`, and `Indeterminate` carrying its reason, and a `Report` that keeps every Assessment in a deterministic order and hands the Plan its Variances only. The `Secret` wrapper for N8.

Evidence, in the [record](results/assessment-algebra.md): the exhaustive requirement-by-evidence truth table; five laws on generated inputs from a fixed seed bank, including the Indeterminate monotonicity and permutation relations; six compile-fail cases; two Kani harnesses with cover checks, which decided the bounded verifier (ADR 0015 note); `SM-ASSESS-001` to `SM-ASSESS-003` active and caught; `cargo-mutants` over the crate with every survivor classified. The permutation law found the reported failure depended on Observation order; the fix and the minimized counterexample fixture are in the record. No `PURITY.toml` allowlist entry was needed.

Exit, met: a malformed path, digest, window, or Condition cannot be built; an absent file cannot carry content; a failed, missing, or contradicted Observation is Indeterminate and never Satisfied or a Variance; every active semantic mutant is caught. N13 is an invariant (spec §58). Not met here and moved: the Trace observe-only compile-fail case of ADR 0013 §1 needs the Substrate port's capability split, which `05-transition-kernel` writes.

### 04-warp-kernel

*Landed 2026-09-28 on branch `milestone/04-warp-kernel`.* In `nomos-warp`: vertices from Assessments with satisfaction and Indeterminate anchors, edges of the three kinds, Kahn's algorithm with an ordered queue, one witness per cyclic component with the acyclic remainder reported blocked, the frontier with per-edge and per-group resolution and Blocked and Skipped propagation, and greedy conflict-key selection over the reserved set with a capacity. No `petgraph`: the graph is a few hundred lines over `alloc`, and the allowlist stays empty. Two working definitions are recorded in ADR 0009's note: a Skipped source is met, and an `after` edge has its own state type because a Kani harness showed a shared one admitted an ill-typed row.

Evidence, in the [record](results/warp-truth-table.md): exhaustive edge and group truth tables including the Indeterminate-anchor rows; the one-changed-one-unchanged, cycle-witness, and self-loop cases; four laws on generated graphs, cycles included, against a reference written by another route (depth-first coloring and whole-graph fixed-point passes); three Kani harnesses; `SM-WARP-001` to `SM-WARP-003` active and caught; `cargo-mutants` over the crate with every survivor classified.

Exit, met: the one-changed-one-unchanged predecessor case activates; the $A \leftrightarrow B$, $B \to C$ graph yields one witness and reports $C$ as blocked; conflicting Actions are never selected together; order and frontier are invariant under insertion-order permutation; the reference and the production evaluator agree on every generated graph. Not here: Obligations as vertices, failure-domain budgets, and derived footprints, all `05-transition-kernel`.

### 05-transition-kernel

In `nomos-core` and `nomos-app`: `step(snapshot, input) = decision` as ADR 0006 states, driven by the application through ports and by the simulator with scripted inputs. The mock service gets separate disk and loaded configuration revisions. Crashes are dropped receipts; delays are reordered inputs; supersession is a second authority input. The first TLA+ models, with their counterexample traces replayed through `step`. `SM-TRANSITION-001` to `SM-TRANSITION-003` activated.

Exit: a crash after configuration replacement cannot lose the refresh Obligation; a timeout cannot free an unsettled conflicting reservation; the final permitted successful execution is reported `Converged`; the models' traces and the kernel's decisions agree on the modeled transitions.

### 06-canon-artifact

On the validated types: normalization, the encoding profile chosen by `canonical-encoding`, decoding through untrusted data-transfer objects, migrations, unknown variants, and two isolated builds compared byte for byte. Accepts [ADR 0011](../../adr/0011-canon-artifact-encoding.md) on top of ADR 0004, or records why not.

Exit: artifact acceptance cannot bypass domain validation or silently change executable intent, and identical declared inputs produce identical IR bytes with every undeclared input denied or recorded.

### 07-substrate-conformance

The first Linux adapter, with one narrowly scoped filesystem operation and the same conformance suite the mock passes, at a composition boundary (`crates/bin/nomos-cell/tests/` first). For `nomos-substrate`: absence differs from denied observation, Trace issues no mutation requests, postconditions are assessed by core, and an unresolved effect keeps its reservation. The first rows of the matrix's environment column.

Exit: the mock and the Linux adapter pass one suite; the suite's negative controls, a mutating Trace and a denied read reported as absence, fail it.

## Three Boundaries, Three Checks

"Ports and adapters are enforced" is three properties.

**Structural: who may depend on whom.** The layer checker and the purity checker. Cargo metadata exposes declared dependency kinds, optionality, target conditions, and renames; its resolved graph omits inactive optional dependencies. Checking only one of the two leaves a hole.

**Type: what callers can construct or request.** Compile-fail tests with `trybuild`, which works for ordinary APIs and checks the intended diagnostic. Used for promises Nomos makes, not for every wrong argument type.

| Promise | Misuse that must fail to compile |
| --- | --- |
| A validated Canon cannot be fabricated | Constructing its private fields directly |
| An absent file cannot specify contents | Constructing the contradictory combination |
| Trace receives observation capability only | Calling a mutation operation through the interface Trace is given |
| Success requires verification evidence | Manufacturing a `Succeeded` result outside the verification path |
| Cipher material cannot enter ordinary serialization | Passing the secret wrapper through a prohibited serialization path |

These establish API restrictions. They do not prove an adapter's Observation is truthful, and a type gate is not evidence of truth (ADR 0015).

**Behavioral: does an adapter honor its contract.** One conformance suite per port, run against each implementation, `07-substrate-conformance`. The workspace is virtual: a test belongs to a package target, and the root `tests/` directory holds only fixture data the gates load.

## The Test Stack

Organized by the question each layer answers, introduced when there is something to ask it of. The full ladder, with what each layer does not establish, is the [verification strategy](../../formal/verification-strategy.md).

| Category | Question | Introduced |
| --- | --- | --- |
| Architecture and policy gates | Can a forbidden dependency, an impure core, an undeclared oracle change, or an unearned receipt enter the workspace? | `01-foundation-gates`, `02-verification-foundation` |
| Unit and exhaustive truth tables | Does each finite decision rule handle every case? | `03-assessment-kernel` |
| Property tests | Do the algebraic laws hold across generated Conditions, Observations, and graphs? | `03-assessment-kernel` |
| Compile-fail tests | Can callers bypass the intended type boundaries? | `03-assessment-kernel` |
| Metamorphic and differential tests | Does the implementation preserve semantics under permitted transformations and agree with an independent reference? | `03-assessment-kernel`, `04-warp-kernel` |
| Semantic mutants and mutation calibration | Would the tests notice a plausible wrong implementation? | `02-verification-foundation` for tooling, then each kernel |
| Deterministic simulation | What happens under delayed results, crashes, duplicates, and supersession? | `05-transition-kernel` |
| Executable formal models | Can modeled interleavings violate safety or conditional liveness? | `05-transition-kernel` |
| Bounded verifiers | Does one pure predicate hold for every input within bounds, non-vacuously? | `03-assessment-kernel` |
| Port conformance tests | Do adapters honor the same observable contract? | `07-substrate-conformance` |
| Fuzzing and real integration | Do untrusted inputs and actual OS behavior violate the boundaries? | `06-canon-artifact` for decoding; `07-substrate-conformance` for the OS |

**Property laws first in line.** $\mathrm{Normalize}(\mathrm{Normalize}(C)) = \mathrm{Normalize}(C)$. Planning results invariant under permitted resource and map insertion-order permutations. Unknown evidence cannot manufacture satisfaction or a known mismatch. A report keeps a Variance beside an unrelated Indeterminate rather than collapsing.

**Seeds are not enough.** `proptest` persists failing seeds, and a changed generator can make an old seed produce a different value. Keep the seed, and promote important minimized failures into concrete regression fixtures with provenance, per the [counterexample policy](../../formal/verification-strategy.md#counterexamples-become-fixtures).

**Mutation tests have teeth or they are decoration.** `cargo-mutants` on changed kernel modules, on a schedule, with every survivor classified; and the [semantic-mutant corpus](../../../tests/semantic-mutants/README.md), each entry caught by a named test, on the required path. The corpus replaces the table of deliberate defects the earlier plan carried; its entries are the same defects, with identifiers.

## Formal Modeling, Early and Small

The first executable model arrives with `05-transition-kernel`, not after it. Two authority inputs can be simulated without a network coordinator. The model holds one Action, its reservation, an effect outcome, an Obligation, and a superseding authority input, and checks:

- **Admission safety.** Conflicting effects never both hold permission to proceed.
- **Uncertainty preservation.** A timeout establishes neither success, failure, nor settlement.
- **Recovery safety.** An effect request is not forgotten because the process that requested it restarted.
- **Completion soundness.** Convergence requires satisfied Conditions, discharged Obligations, and Settled relevant effects.

The TLA+ model checker (TLC) checks the specified model under its bounds. The record says so, and never calls it a proof of the Rust or of Linux. The `loom` model checker waits for an actual shared-memory synchronization point; a component with the same name is not a reason.

## The Verification Record

Every important property gets a receipt and, for an experiment, a result card under `results/` with the fields of ADR 0007 §1. `docs/formal/verification-matrix.md` is filled from those only.

## Resequenced Deferrals

The earlier plan deferred three areas because their concrete integrations do not exist. Their logical contracts do not need them.

- **Secrets.** Test secret-wrapper redaction and serialization restrictions in `03-assessment-kernel`, with fake values. Vault integration waits.
- **Fencing.** Model competing authority, atomic acceptance, and effect admission in `05-transition-kernel`. Loom transport waits.
- **Event durability.** Model the ordering of intent, dispatch, completion, and acknowledgment in `05-transition-kernel` against a fault-injectable in-memory store. Physical crash and fsync testing waits for the storage adapter.

## Experiments by Milestone

The research snapshot's experiments remain the evidence units, joined by four the verification foundation added.

| Experiment | Milestone | Record |
| --- | --- | --- |
| `layer-policy` | `01-foundation-gates` | In [ADR 0007](../../adr/0007-verification-gates.md) Evidence; receipt `check-layers` |
| `agent-proof-gate` | `02-verification-foundation` | [results/agent-proof-gate.md](results/agent-proof-gate.md) |
| `core-purity` | `02-verification-foundation` | [results/core-purity.md](results/core-purity.md) |
| `mutation-calibration` | `02-verification-foundation` | [results/mutation-calibration.md](results/mutation-calibration.md) |
| `generator-variance` | `02-verification-foundation` designed; runs when a kernel exists | [results/generator-variance.md](results/generator-variance.md) |
| `assessment-algebra` | `03-assessment-kernel` | [results/assessment-algebra.md](results/assessment-algebra.md) |
| `warp-truth-table` | `04-warp-kernel` | [results/warp-truth-table.md](results/warp-truth-table.md) |
| `bounded-convergence`, `effect-recovery`, `refresh-recovery`, `scheduler-admission`, `kernel-conformance` | `05-transition-kernel` | none |
| `typed-validation`, `canonical-encoding`, `compatibility-matrix`, `build-hermeticity` | `06-canon-artifact` | none |
| `controller-composition` | After `05-transition-kernel`, before `07-substrate-conformance` | none |
| `substrate-contract` | `07-substrate-conformance` | none |

### layer-policy

- **Question.** Does the checker reject a forbidden workspace edge in both the declared and the resolved graph, including behind a feature, a rename, a target condition, or a dev or build dependency?
- **Rival.** Conditional dependencies escape the check, or the checker succeeds by rejecting everything.
- **Harness.** `cargo xtask check-layers` over fixture workspaces under `tests/fixtures/layer-policy/`, one per dependency kind, plus one allowed workspace.
- **Negative control.** Each forbidden fixture; the allowed fixture is the positive control.
- **Decision rule, met.** Every forbidden fixture rejected, the allowed one accepted, and ordinary compilation never reported as policy conformance.

### agent-proof-gate

- **Question.** Does a mechanical gate catch a commit that changes a specification or a verifier without saying so, or adds a known escape hatch?
- **Rival.** The change is made silently and a green result is accepted; or the gate blocks paths and is routed around.
- **Harness.** `cargo xtask check-trust-boundary` over fixture commits under `tests/fixtures/agent-proof/`.
- **Negative control.** An undeclared specification change, an undeclared verifier change, a loosened gate test, a removed CI step, an ignored test, an oracle mixed with implementation, a declaration without a change, an unknown class.
- **Decision rule, met.** Every fixture shortcut fails with its code; the positive controls pass; semantic weakening remains a human review item under rule 11, and the record says so.

### core-purity

- **Question.** Can the core crates be held to purity by the compiler and a policy, without a review reading every dependency?
- **Rival.** `no_std` costs something the crates need; the policy cannot see transitive or feature-gated effects; a dependency class list is unmaintainable.
- **Harness.** The `no_std` experiment on all three crates; `cargo xtask check-core-purity` over `tests/fixtures/core-purity/`.
- **Negative control.** A randomness crate, a runtime, an unlisted crate, a build script, dropped `no_std`, default features, an extra feature, a transitive reach, dropped lints, dropped workspace lints, an unlisted core crate, a policy naming a ghost, a malformed policy, a missing policy.
- **Decision rule, met.** Every case rejected by code, the allowed fixture and the real workspace accepted, a dev-dependency exempt.

### mutation-calibration

- **Question.** What does `cargo-mutants` cost on this repository, what does it generate, and what do its survivors mean?
- **Rival.** The run is too slow for any schedule, or the survivors are noise.
- **Harness.** `cargo mutants -p nomos-xtask` on `manifest.rs`, `strict_json.rs`, `snapshot.rs`, twice: before and after the tests it found missing.
- **Decision rule, met.** Runtime, counts, and every survivor classified by hand; a scheduled non-blocking job; no threshold.

### generator-variance

- **Question.** When several independent generations implement one kernel property, how often does a fixed oracle reject a candidate its own generated tests accept?
- **Rival.** Generated tests and fixed oracles agree, and ADR 0015 §5 is unnecessary caution.
- **Harness.** Designed in its [record](results/generator-variance.md). Runs on the first kernel function; exploratory; never gates.
- **Decision rule.** The disagreement rate is recorded as a research finding, and no threshold is set.

### assessment-algebra

- **Question.** Are three Assessment outcomes per Condition, kept unaggregated, enough for the first resource family?
- **Rival.** An aggregate shortcut collapses unknown, absent, stale, or contradictory evidence into one of the two old outcomes.
- **Harness.** `03-assessment-kernel`'s types; exhaustive pair tables; `proptest` laws from a seed bank; Observation fixtures for present, absent, old, denied, and conflicting; the Indeterminate monotonicity relation; `SM-ASSESS-001` to `SM-ASSESS-003`; two Kani harnesses.
- **Negative control.** `partial-assessment`: a Variance beside an Indeterminate must still be reported. A denied read must never assess as Satisfied or Variance. Each semantic mutant must be caught.
- **Measurements.** Truth-table agreement; Indeterminate Assessments reaching the Plan (zero); survivors of `cargo-mutants`, classified.
- **Decision rule, met.** No Indeterminate becomes Variance or Satisfied, and no unrelated Indeterminate hides a Variance. Bounded typed bindings were not needed for the file family and are not measured.

### warp-truth-table

- **Question.** Do Startable, group-level Activated, and satisfaction anchors fully specify v0 dependencies?
- **Rival.** Terminal, changed, and succeeded are conflated; an unchanged source vetoes a changed one; a satisfied prerequisite without an Action breaks the graph.
- **Harness.** `04-warp-kernel`'s rules; exhaustive predecessor-outcome tables; generated graphs with insertion-order permutations; the reference evaluator, differentially; three Kani harnesses; `SM-WARP-001` to `SM-WARP-003`.
- **Negative control.** `activation-missing`; the one-changed-one-unchanged case; the $A \leftrightarrow B$, $B \to C$ witness case; a failed partial write must not activate its dependent; each semantic mutant caught.
- **Measurements.** Activation correctness; order determinism (N12); witness validity; generator coverage as stated; survivors of `cargo-mutants`, classified.
- **Decision rule, met.** Every case resolves to exactly one state with no fallthrough. Feeds [ADR 0009](../../adr/0009-warp-activation-semantics.md), which stays Proposed until `05-transition-kernel` runs its second criterion.

### bounded-convergence

- **Question.** Does the loop report the right outcome at the bound without hiding effects still in flight, and does it distinguish Indeterminate from Converged and Failed?
- **Rival.** The last successful execution is misreported, slow completion is classified as oscillation, or an Indeterminate run is reported as converged.
- **Harness.** `05-transition-kernel`'s kernel against the mock with scripted resources: converge on attempt $k$, never converge, complete one observation late, oscillate against a scripted writer, deny a read, change unrelated telemetry every observation.
- **Negative control.** `final-check`; a slow completion must not be reported as oscillation; a denied read with nothing else to do must end `Indeterminate`.
- **Measurements.** Outcome classification per script; observation count; effects left unsettled at exit.
- **Decision rule.** Converged, bound reached, Indeterminate, known failure, and unknown outcome are distinguished, and Converged is never reported with an effect unsettled.

### effect-recovery

- **Question.** Do run-scoped idempotency keys deduplicate retries within one execution while allowing later repairs?
- **Rival.** A content-derived key suppresses drift repair, or a retry overlaps a still-live operation.
- **Harness.** `05-transition-kernel`'s simulator: duplicate receipts before, during, and after completion; new drift after completion followed by a new run with the same semantic Action; one scripted effect with no readable postcondition.
- **Negative control.** `dedup-scope`.
- **Measurements.** Duplicate live effects; suppressed repairs; unsafe retries; effects left explicitly unresolved.
- **Decision rule.** No silent re-execution and no silent suppression; an effect without settlement evidence stays unresolved (N10).

### refresh-recovery

- **Question.** Which design keeps a service refresh across a crash between file replacement and restart: a durable Obligation recorded before the replacement, a loaded-revision Condition, or the draft's transient `on_change`?
- **Rival.** The file comparison after recovery sees Satisfied and the refresh is lost, or a restart receipt is mistaken for evidence that the new revision loaded.
- **Harness.** `05-transition-kernel`'s mock service with separate disk and loaded revisions; a crash at every step boundary; all three designs against the same script.
- **Negative control.** `lost-refresh`: the transient design must lose the refresh, and the harness must see it.
- **Measurements.** Lost refreshes; unnecessary refreshes; unresolved outcomes, per design.
- **Decision rule.** No declared Obligation disappears; a service that cannot report its loaded revision gets a documented uncertainty and retry policy, not an exactly-once claim.

### scheduler-admission

- **Question.** Is serialized, reservation-based greedy selection safe under declared footprints, budgets, and involuntary failures, with reservations held until settlement?
- **Rival.** Concurrent admission, aliased keys, implicit effects, or unsettled effects bypass the reservations.
- **Harness.** `04-warp-kernel`'s selection over `Reserved`; generated ready sets and footprints; interleavings of reserve, dispatch, timeout, verify, settle, and release explored in the `05-transition-kernel` simulator; scripted involuntary failures and repeated arrivals.
- **Negative control.** `budget-not-world`; a reservation released at `Running` exit must allow an overlap with a timed-out Action, and the corrected reservation must not; two admissions against one stale snapshot must not both pass.
- **Measurements.** Conflicting overlaps; budget violations; starvation under the stated fairness premise.
- **Decision rule.** No modeled conflict or oversubscription; maximality and fairness claimed only under stated premises.

### kernel-conformance

- **Question.** Can the `05-transition-kernel` kernel be modeled in TLA+ with explicit bounds, and do its counterexample traces replay through `step`?
- **Rival.** The model omits an implementation transition or assumes more of the adapters than they promise.
- **Harness.** `formal/tla/` models of the transitions in the Formal Modeling section; TLC traces exported and replayed through `step`; the bounded-verifier harness with cover checks.
- **Negative control.** A known-bad transition inserted into the kernel must produce a trace the model rejects; a vacuous harness must be exposed by its cover check.
- **Measurements.** Transitions mapped; negative-control failures; model bounds; check time.
- **Decision rule.** The record names exactly which properties were checked, under which bounds, with which adapters trusted, and never calls a finite-model check a proof.

### typed-validation

- **Question.** Do construction, decoding, and migration enforce the same domain invariants?
- **Rival.** Derived deserialization, a public field, or a migration constructs a value the constructor would reject.
- **Harness.** `06-canon-artifact` on `03-assessment-kernel`'s types; generated valid and malformed inputs; `trybuild` compile-fail cases; the absent-with-contents countercase.
- **Negative control.** A `serde` derive without the `try_from` boundary must let a malformed value through, and the harness must detect it.
- **Measurements.** Rejection parity across paths; panic count (target zero); smallest counterexample found.
- **Decision rule.** No malformed input becomes a validated Canon by any supported path; every error is typed and secret-free.

### canonical-encoding

- **Question.** Which restricted profile makes semantic equivalence and byte equality coincide on the IR: deterministic Concise Binary Object Representation (CBOR) per Request for Comments (RFC) 8949 §4.2, or the JSON Canonicalization Scheme (JCS) of RFC 8785?
- **Rival.** Number, Unicode, map ordering, duplicate keys, or default handling breaks the equivalence in one of them.
- **Harness.** Semantic equivalences defined first; golden and adversarial fixtures encoded by both profiles, decoded independently, compared across two builds; `CanonID` over a domain tag, a version, and the normalized content.
- **Negative control.** Two semantically different Canons that a naive normalization collapses must keep distinct identities.
- **Measurements.** Golden-vector agreement; artifact size; encode and decode cost; dependency count.
- **Decision rule.** The simpler profile that passes every semantic test; throughput decides nothing.

### compatibility-matrix

- **Question.** Do explicit schema versions and migrations preserve accepted semantics across old and new readers?
- **Rival.** An unknown variant or a changed default silently changes executable intent.
- **Harness.** Version 1 and 2 fixtures across readers; unknown capabilities and unknown mutating variants; migrations that keep original bytes and lineage.
- **Negative control.** An unknown mutating variant accepted for forward compatibility must be caught before any effect.
- **Measurements.** Silent semantic changes; correct rejections; identity consistency across migration.
- **Decision rule.** Unsupported execution fails before effect; archival preservation never implies permission to execute.

### build-hermeticity

- **Question.** Do two isolated builds with the same declared inputs produce the same IR, and is every undeclared input denied or recorded? The contract is ADR 0004 §3.
- **Rival.** Wall clock, locale, temporary paths, hash seeds, environment, or network reach the generator.
- **Harness.** The same Canon crate built twice in isolation with each ambient input varied; undeclared reads and network attempts observed, not assumed absent.
- **Decision rule.** Adopt the pipeline only after each undeclared-input path is denied or recorded; identical outputs alone prove nothing.

### controller-composition

- **Question.** Do explicit footprints with stated guarantees and reliance assumptions detect a harmful composition of two controllers that each converge alone?
- **Rival.** The two oscillate together and nothing in the model sees it.
- **Harness.** Two mock controllers sharing a file or a sysctl, in shared, single-owner, and disjoint configurations, on the `05-transition-kernel` simulator.
- **Negative control.** The shared-ownership configuration must oscillate and be detected or rejected.
- **Decision rule.** One safe composition demonstrated and one counterexample detected before any whole-host claim.

### substrate-contract

- **Question.** Does the Linux adapter honor the `nomos-substrate` contract the mock honors: absence distinct from denial, no mutation through Trace, postconditions assessed by core?
- **Rival.** Symlink races, path aliases, or foreign writers make an Observation lie, and the suite cannot tell.
- **Harness.** The port conformance suite of `07-substrate-conformance` on Linux, with failure injection.
- **Negative control.** A mutating Trace and a denied read reported as absence must both fail the suite.
- **Decision rule.** One narrowly scoped operation passes the same suite as the mock; every environment-column entry it earns names the host it ran on.

## Deferred Beyond the Milestones

| Experiment | Phase | Reason |
| --- | --- | --- |
| `event-crash-replay` | 2 | The logical ordering is modeled in `05-transition-kernel`; physical fsync and crash testing needs the storage adapter |
| `fence-interleavings` | 3–4 | The authority model is in `05-transition-kernel`; transport races need Loom |
| `identity-rollback` | 3–4 | Needs enrollment, snapshots, and clones |
| `secret-nondisclosure` | 6 | Wrapper and serialization checks are in `03-assessment-kernel`; provider paths need an adapter |
| `incremental-ablation` | after the milestones | Needs the full recomputation path as the reference |
| `plan-witness` | after the milestones | Needs a baseline planner to check against |

## Exit Criteria

1. Seven milestones merged, each with its stated exit demonstrated by a named test.
2. A result record under `results/` for every experiment in the table above, none claiming more than it ran, and a receipt for every command a record cites.
3. `docs/formal/verification-matrix.md` filled from records, with *not run* and *inconclusive* wherever true.
4. ADRs accepted from their evidence, or kept Proposed with the reason recorded: [0009](../../adr/0009-warp-activation-semantics.md), [0010](../../adr/0010-effect-recovery-and-fencing.md) for the Cell, and [0011](../../adr/0011-canon-artifact-encoding.md). [ADR 0007](../../adr/0007-verification-gates.md), [0015](../../adr/0015-generator-verifier-development-model.md), and [0016](../../adr/0016-core-purity.md) are already accepted.
5. Spec §62 updated: edge semantics closed, the encoding question closed.
6. The required CI path is small and green, with `--locked`, the layer checker, the purity checker, the trust-boundary gate, the receipt validator, the semantic mutants, and the snapshot freeze in it; mutation runs stay scheduled and non-blocking.
