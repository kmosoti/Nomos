# Milestone 1: An Executable Kernel Contract

- **Status.** Proposed, 2026-09-28, revised the same day after a review of `main` at `013b9d0`. No experiment has run.
- **Goal.** One typed Canon, one shared assessment model, one deterministic planner, one replayable transition kernel, and a harness that demonstrably rejects broken boundaries and broken semantics.
- **Not the goal.** Another round of adapters, networking, storage, or a testing platform with nothing meaningful to test.

## What Main Establishes Today

| Area | Evidence | Not established |
| --- | --- | --- |
| Build and basic checks | CI green: formatting, Clippy, workspace tests | Anything about the control system |
| Research tooling | `nomos-xtask` verifies the snapshot as a whole and reproduces seven counterexample models | Convergence, fencing, or adapter correctness |
| Domain kernel | Module documentation only | Any assessment or transition semantics |
| Architecture | Dependency rules documented; ADR 0000 says Cargo alone does not enforce them | Rejection of a future forbidden dependency |
| Formal verification | Written algorithms and proof sketches; `formal/tla/` holds a README | A checked executable model |

A green research-import build is not a verified engine. The milestone exists to change the right-hand column, one row at a time, with evidence.

## Decisions Already Made

| Decision | Record | Still open |
| --- | --- | --- |
| Canon is authored in Rust and shipped as an inert Canonical IR | [ADR 0004](../../adr/0004-rust-typed-canon.md) | Encoding profile, schema versioning, migration rules, IR suffix |
| Condition, Observation, Assessment, Indeterminate, Obligation, Settled | [ADR 0005](../../adr/0005-assessment-vocabulary.md) | Settlement evidence per resource kind |
| Core owns semantics; the kernel is `step(snapshot, input) = decision`; adapters obtain evidence and perform operations | [ADR 0006](../../adr/0006-kernel-contract.md) | Port shapes as code |
| Tooling is Rust in `nomos-xtask` | [ADR 0003](../../adr/0003-xtask-tooling-crate.md) | Layer checker, snapshot freeze in CI |

Open and not blocking: whether a fixed-`argv` package-manager invocation is allowed under AGENTS.md rule 5 (Phase 1), and edge semantics as an ADR (`warp-gates`, closed by PR 3).

## Rules

- **Design before code.** Each experiment carries a question, a rival hypothesis, a harness, a negative control, measurements, and a decision rule. Code that serves none of them is not part of the milestone.
- **Negative controls.** Every check includes a case the draft or a plausible wrong implementation fails. A negative control that passes against the wrong thing means the test is not testing. The seven bundle counterexamples stay as historical models in `nomos-xtask`; they assert the bad outcomes the old prose admitted. New tests call the kernel and assert the corrected behavior.
- **Result records.** An experiment ends with `results/<experiment>.md` in the verification-record form below. A record is written after the run, never before. No cell in the verification matrix says *passed* for planned work; a timeout stays *inconclusive*.
- **Determinism applies to harnesses.** Property tests record their seeds. Generated graphs and fixtures are reproducible from a seed and a version. A flaky harness is a finding about the harness.
- **Vocabulary and invariants.** Code and documents use spec §3 terms as expanded by ADR 0005. A change that touches N1–N12 adds the test for it. A new term or a changed invariant is an ADR.
- **Specifications are protected.** AGENTS.md rule 11. An `assume` in a harness is allowed when explicit, justified, reviewed separately, and shown non-vacuous.
- **Tooling is Rust.** Anything that regenerates a record is a `cargo xtask` command or a Rust test.

## Pull Requests

Five focused pull requests, in order. Each names its exit condition, and the exit is a behavior a test demonstrates, not a file that exists.

### PR 1: Align the Specification and Make the Gates Real

*Landed 2026-09-28.* ADRs 0004 to 0006, the spec amendments, the corrected formal documents, strict whole-snapshot verification shared by the CLI and the tests, `cargo xtask check-layers` over the declared and resolved graphs with fixture cases for direct, renamed, optional, target-specific, build, and dev dependencies, an adapter naming two ports, a dependency on a bin crate, and core depending on a port, plus one allowed workspace so the checker cannot pass by rejecting everything. `cargo xtask research frozen --base <ref>` is the Git comparison checksums cannot replace. CI runs `--locked`, `verify-all`, and `check-layers` on every push, and `frozen` on pull requests. Exceptions to the dependency rule are none; the mock adapter does not become a dev-dependency of `nomos-app`.

Hardened the same day to the owner's PR 1 specification: one `verify_snapshot` with no weaker mode and a stable code on every failure, negative controls asserted by reason, the matching-port rule for adapters, `nomos-core`, dev, build, and tooling-crate cases, the real crate names in the fixtures, `cargo check --locked` in CI, and `--locked` in the `cargo xtask` alias.

Exit, met: a forbidden dependency and a corrupted snapshot each fail the required check for the stated reason, the allowed workspace passes, and restoring the valid input passes again. The owner scoped PR 1 to specification alignment, snapshot integrity, layer enforcement, and CI. The `agent-proof-gate` experiment and mutation runs over `nomos-xtask` therefore move to their own pull request after PR 1.

### PR 2: The Assessment Algebra

One resource family first: file presence and content requirements. In `nomos-core`: validated `Condition` types with private fields, `Observation` with provenance, collection window, and collection outcome, per-Condition `Assessment` with `Satisfied`, `Variance`, and `Indeterminate` carrying its reason, and a report that keeps every Assessment.

Tests: exhaustive truth tables for pairs, property tests for longer conjunctions, compile-fail cases for the type boundaries in the table below, and the targeted mutations in the negative-control table.

Exit: malformed values cannot enter through any supported construction path, and uncertainty is never converted into satisfaction or Variance.

### PR 3: Warp's Decision Rules

In `nomos-warp`: the activation truth table with group-level `on_change` resolution, satisfaction anchors for `requires`, `after`, and `on_change`, deterministic order, cycle witnesses restricted to cyclic components, and conflict-aware selection over `Reserved`. An independently written reference evaluator for small graphs, used differentially.

Exit: the one-changed-one-unchanged predecessor case activates; the $A \leftrightarrow B$, $B \to C$ graph yields one witness and reports $C$ as blocked; conflicting Actions are never selected together; order is invariant under insertion-order permutation.

### PR 4: The Transition Kernel and Recovery Simulator

In `nomos-core` and `nomos-app`: `step(snapshot, input) = decision` as ADR 0006 states, driven by the application through ports and by the simulator with scripted inputs. The mock service gets separate disk and loaded configuration revisions. Crashes are dropped receipts; delays are reordered inputs; supersession is a second authority input. The first TLA+ model of the same transitions, with its counterexample traces replayed through `step`.

Exit: a crash after configuration replacement cannot lose the refresh Obligation; a timeout cannot free an unsettled conflicting reservation; the final permitted successful execution is reported `Converged`; the model's traces and the kernel's decisions agree on the modeled transitions.

### PR 5: The Inert Canon Boundary

On the validated types: normalization, the encoding profile chosen by `canonical-encoding`, decoding through untrusted data-transfer objects, migrations, unknown variants, and two isolated builds compared byte for byte. Closes the remainder of the `canon-artifact` candidate on top of ADR 0004.

Exit: artifact acceptance cannot bypass domain validation or silently change executable intent, and identical declared inputs produce identical IR bytes with every undeclared input denied or recorded.

Only after PR 5 does the first Linux adapter start, with one narrowly scoped filesystem operation and the same conformance suite the mock passes.

## Three Boundaries, Three Checks

"Ports and adapters are enforced" is three properties.

**Structural: who may depend on whom.** The layer checker in PR 1. Cargo metadata exposes declared dependency kinds, optionality, target conditions, and renames; its resolved graph omits inactive optional dependencies. Checking only one of the two leaves a hole.

**Type: what callers can construct or request.** Compile-fail tests with `trybuild`, which works for ordinary APIs and checks the intended diagnostic. Used for promises Nomos makes, not for every wrong argument type.

| Promise | Misuse that must fail to compile |
| --- | --- |
| A validated Canon cannot be fabricated | Constructing its private fields directly |
| An absent file cannot specify contents | Constructing the contradictory combination |
| Trace receives observation capability only | Calling a mutation operation through the interface Trace is given |
| Success requires verification evidence | Manufacturing a `Succeeded` result outside the verification path |
| Cipher material cannot enter ordinary serialization | Passing the secret wrapper through a prohibited serialization path |

These establish API restrictions. They do not prove an adapter's Observation is truthful.

**Behavioral: does an adapter honor its contract.** One conformance suite per port, run against each implementation, at a composition boundary (`crates/bin/nomos-cell/tests/` first). For `nomos-substrate`: absence differs from denied observation, Trace issues no mutation requests, postconditions are assessed by core, and an unresolved effect keeps its reservation. The workspace is virtual: a test belongs to a package target, and the root `tests/` directory holds only fixture data those targets load.

## The Test Stack

Organized by the question each layer answers, introduced when there is something to ask it of.

| Category | Question | Introduced |
| --- | --- | --- |
| Unit and exhaustive truth tables | Does each finite decision rule handle every case? | PR 2 |
| Property tests | Do the algebraic laws hold across generated Conditions, Observations, and graphs? | PR 2 |
| Compile-fail tests | Can callers bypass the intended type boundaries? | PR 2 |
| Architecture fixtures | Can a forbidden dependency or policy bypass enter the workspace? | PR 1 |
| Port conformance tests | Do adapters honor the same observable contract? | PR 4 |
| Differential and metamorphic tests | Does the implementation agree with an independent reference and preserve semantics under permitted transformations? | PR 3 |
| Mutation tests | Would the tests notice a plausible wrong implementation? | PR 1 for tooling, then each kernel slice |
| Deterministic simulation | What happens under delayed results, crashes, duplicates, and supersession? | PR 4 |
| Executable formal models | Can modeled interleavings violate safety or conditional liveness? | PR 4 |
| Fuzzing and real integration | Do untrusted inputs and actual OS behavior violate the boundaries? | PR 5 for decoding; Linux adapter for the OS |

**Property laws first in line.** $\mathrm{Normalize}(\mathrm{Normalize}(C)) = \mathrm{Normalize}(C)$. Planning results invariant under permitted resource and map insertion-order permutations. Unknown evidence cannot manufacture satisfaction or a known mismatch. A report keeps a Variance beside an unrelated Indeterminate rather than collapsing. Warp generators cover valid dependencies, missing references, cycles, multiple activation sources, and overlapping footprints.

**Seeds are not enough.** `proptest` persists failing seeds, and a changed generator can make an old seed produce a different value. Keep the seed, and promote important minimized failures into concrete regression fixtures. CI runs a reproducible seed bank as the required path and a broader exploration separately, recording its seeds and failing traces.

**Mutation tests have teeth or they are decoration.** `cargo-mutants` on changed kernel modules, plus these semantic negative controls, each of which must be caught by a named test:

| Deliberate defect | Test that must catch it |
| --- | --- |
| Remove the changed-source activation requirement | Unchanged configuration must not activate refresh |
| Change *any* activation to *all* | One changed and one unchanged predecessor must activate |
| Drop a reservation on timeout | A delayed old effect must block conflicting admission |
| Reuse a semantic digest as a permanent idempotency key | Later drift must still be repaired |
| Return convergence from empty Variance alone | A pending Obligation must prevent completion |
| Skip manifest verification | A corrupted non-graph snapshot file must fail |
| Ignore an optional dependency in the layer checker | A forbidden feature-gated edge must fail |

Record surviving mutants, invalid mutations, exclusions, and inconclusive runs separately. A score is not the point; every surviving mutant on a safety contract is investigated.

## Formal Modeling, Early and Small

The first executable model arrives with PR 4, not after it. Two authority inputs can be simulated without a network coordinator. The model holds one Action, its reservation, an effect outcome, an Obligation, and a superseding authority input, and checks:

- **Admission safety.** Conflicting effects never both hold permission to proceed.
- **Uncertainty preservation.** A timeout establishes neither success, failure, nor settlement.
- **Recovery safety.** An effect request is not forgotten because the process that requested it restarted.
- **Completion soundness.** Convergence requires satisfied Conditions, discharged Obligations, and Settled relevant effects.

The TLA+ model checker (TLC) checks the specified model under its bounds. The record says so, and never calls it a proof of the Rust or of Linux. One Kani harness on one pure predicate, admission or dependency resolution, learns the cost before any wider adoption; it includes `kani::cover` checks, because a satisfied assertion on an unreachable path is vacuous. The `loom` model checker waits for an actual shared-memory synchronization point; a component with the same name is not a reason.

## The Verification Record

Every important property gets a record with these fields, and `docs/formal/verification-matrix.md` is filled from records only.

| Field | Purpose |
| --- | --- |
| Property ID and specification revision | Exactly what was checked |
| Production function and harness | Where the model meets the implementation |
| Toolchain, dependency lock, target, tool version | Reproduction context |
| Bounds, seeds, fixtures, assumptions | Scope of exploration |
| Positive and negative-control results | Whether the harness tells correct from incorrect |
| Unchecked behavior | What the result does not cover |

## Resequenced Deferrals

The earlier plan deferred three areas because their concrete integrations do not exist. Their logical contracts do not need them.

- **Secrets.** Test secret-wrapper redaction and serialization restrictions now, with fake values (PR 2's compile-fail row and a sentinel test). Vault integration waits.
- **Fencing.** Model competing authority, atomic acceptance, and effect admission now (PR 4's model). Loom transport waits.
- **Event durability.** Model the ordering of intent, dispatch, completion, and acknowledgment now against a fault-injectable in-memory store (PR 4's simulator). Physical crash and fsync testing waits for the storage adapter.

## Experiments by Pull Request

The research snapshot's experiments remain the evidence units. The cards below carry their question, rival, harness, negative control, measurements, and decision rule.

| Experiment | Pull request |
| --- | --- |
| `layer-policy` | PR 1 |
| `agent-proof-gate` | Its own pull request, after PR 1 |
| `assessment-algebra` | PR 2 |
| `warp-truth-table` | PR 3 |
| `bounded-convergence`, `effect-recovery`, `refresh-recovery`, `scheduler-admission`, `kernel-conformance` | PR 4 |
| `typed-validation`, `canonical-encoding`, `compatibility-matrix`, `build-hermeticity` | PR 5 |
| `controller-composition` | After PR 4, before the Linux adapter |

### layer-policy

- **Question.** Does the checker reject a forbidden workspace edge in both the declared and the resolved graph, including behind a feature, a rename, a target condition, or a dev or build dependency?
- **Rival.** Conditional dependencies escape the check, or the checker succeeds by rejecting everything.
- **Harness.** `cargo xtask check-layers` over fixture workspaces under `tests/fixtures/layer-policy/`, one per dependency kind, plus one allowed workspace.
- **Negative control.** Each forbidden fixture; the allowed fixture is the positive control.
- **Measurements.** Forbidden edges detected per kind; feature and target combinations covered.
- **Decision rule.** Every forbidden fixture rejected, the allowed one accepted, and ordinary compilation never reported as policy conformance.

### agent-proof-gate

- **Question.** Do the review rules catch a patch that passes a check by weakening what the check asserts?
- **Rival.** The patch loosens a postcondition, adds an `assume`, `admit`, or `#[ignore]`, or moves code out of the verifier's view, and the green result is accepted.
- **Harness.** Negative-control patches under `tests/fixtures/agent-proof/`; a `cargo xtask` command that diffs specification and assumption changes separately from proof annotations; a review checklist. A regular expression cannot detect semantic weakening and the record says so.
- **Decision rule.** Every known invalid shortcut fails the gate; semantic weakening remains a human review item under AGENTS.md rule 11.

### assessment-algebra

- **Question.** Are three Assessment outcomes per Condition, kept unaggregated, enough for the first resource family?
- **Rival.** An aggregate shortcut collapses unknown, absent, stale, or contradictory evidence into one of the two old outcomes.
- **Harness.** PR 2's types; exhaustive pair tables; `proptest` conjunctions; Observation fixtures for present, absent, stale, denied, and conflicting.
- **Negative control.** `partial-assessment`: a Variance beside an Indeterminate must still be reported. A denied read must never assess as Satisfied or Variance. An empty error list must not mean Satisfied.
- **Measurements.** Truth-table agreement; mutation admissions from an Indeterminate (target zero); evaluation bound for any bounded typed bindings.
- **Decision rule.** No Indeterminate becomes Variance or Satisfied, and no unrelated Indeterminate hides a Variance.

### warp-truth-table

- **Question.** Do Startable, group-level Activated, and satisfaction anchors fully specify v0 dependencies?
- **Rival.** Terminal, changed, and succeeded are conflated; an unchanged source vetoes a changed one; a satisfied prerequisite without an Action breaks the graph.
- **Harness.** PR 3's rules; exhaustive predecessor-outcome tables; random DAGs with insertion-order permutations; the reference evaluator, differentially.
- **Negative control.** `activation-missing`; the one-changed-one-unchanged case; the $A \leftrightarrow B$, $B \to C$ witness case; a failed partial write must not activate its dependent.
- **Measurements.** Activation correctness; order determinism (N12); witness validity.
- **Decision rule.** Every case resolves to exactly one state with no fallthrough. Feeds ADR `warp-gates`.

### bounded-convergence

- **Question.** Does the loop report the right outcome at the bound without hiding effects still in flight, and does it distinguish Indeterminate from Converged and Failed?
- **Rival.** The last successful execution is misreported, slow completion is classified as oscillation, or an Indeterminate run is reported as converged.
- **Harness.** PR 4's kernel against the mock with scripted resources: converge on attempt $k$, never converge, complete one observation late, oscillate against a scripted writer, deny a read, change unrelated telemetry every observation.
- **Negative control.** `final-check`; a slow completion must not be reported as oscillation; a denied read with nothing else to do must end `Indeterminate`.
- **Measurements.** Outcome classification per script; observation count; effects left unsettled at exit.
- **Decision rule.** Converged, bound reached, Indeterminate, known failure, and unknown outcome are distinguished, and Converged is never reported with an effect unsettled.

### effect-recovery

- **Question.** Do run-scoped idempotency keys deduplicate retries within one execution while allowing later repairs?
- **Rival.** A content-derived key suppresses drift repair, or a retry overlaps a still-live operation.
- **Harness.** PR 4's simulator: duplicate receipts before, during, and after completion; new drift after completion followed by a new run with the same semantic Action; one scripted effect with no readable postcondition.
- **Negative control.** `dedup-scope`.
- **Measurements.** Duplicate live effects; suppressed repairs; unsafe retries; effects left explicitly unresolved.
- **Decision rule.** No silent re-execution and no silent suppression; an effect without settlement evidence stays unresolved (N10).

### refresh-recovery

- **Question.** Which design keeps a service refresh across a crash between file replacement and restart: a durable Obligation recorded before the replacement, a loaded-revision Condition, or the draft's transient `on_change`?
- **Rival.** The file comparison after recovery sees Satisfied and the refresh is lost, or a restart receipt is mistaken for evidence that the new revision loaded.
- **Harness.** PR 4's mock service with separate disk and loaded revisions; a crash at every step boundary; all three designs against the same script.
- **Negative control.** `lost-refresh`: the transient design must lose the refresh, and the harness must see it.
- **Measurements.** Lost refreshes; unnecessary refreshes; unresolved outcomes, per design.
- **Decision rule.** No declared Obligation disappears; a service that cannot report its loaded revision gets a documented uncertainty and retry policy, not an exactly-once claim.

### scheduler-admission

- **Question.** Is serialized, reservation-based greedy selection safe under declared footprints, budgets, and involuntary failures, with reservations held until settlement?
- **Rival.** Concurrent admission, aliased keys, implicit effects, or unsettled effects bypass the reservations.
- **Harness.** PR 3's selection over `Reserved`; generated ready sets and footprints; interleavings of reserve, dispatch, timeout, verify, settle, and release explored in the PR 4 simulator; scripted involuntary failures and repeated arrivals.
- **Negative control.** `budget-not-world`; a reservation released at `Running` exit must allow an overlap with a timed-out Action, and the corrected reservation must not; two admissions against one stale snapshot must not both pass.
- **Measurements.** Conflicting overlaps; budget violations; starvation under the stated fairness premise.
- **Decision rule.** No modeled conflict or oversubscription; maximality and fairness claimed only under stated premises.

### kernel-conformance

- **Question.** Can the PR 4 kernel be modeled in TLA+ with explicit bounds, and do its counterexample traces replay through `step`?
- **Rival.** The model omits an implementation transition or assumes more of the adapters than they promise.
- **Harness.** `formal/tla/` models of the transitions in the Formal Modeling section; TLC traces exported and replayed through `step`; one Kani harness with cover checks.
- **Negative control.** A known-bad transition inserted into the kernel must produce a trace the model rejects; a vacuous harness must be exposed by its cover check.
- **Measurements.** Transitions mapped; negative-control failures; model bounds; check time.
- **Decision rule.** The record names exactly which properties were checked, under which bounds, with which adapters trusted, and never calls a finite-model check a proof.

### typed-validation

- **Question.** Do construction, decoding, and migration enforce the same domain invariants?
- **Rival.** Derived deserialization, a public field, or a migration constructs a value the constructor would reject.
- **Harness.** PR 5 on PR 2's types; generated valid and malformed inputs; `trybuild` compile-fail cases; the absent-with-contents countercase.
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
- **Harness.** Two mock controllers sharing a file or a sysctl, in shared, single-owner, and disjoint configurations, on the PR 4 simulator.
- **Negative control.** The shared-ownership configuration must oscillate and be detected or rejected.
- **Decision rule.** One safe composition demonstrated and one counterexample detected before any whole-host claim.

## Deferred Beyond the Milestone

| Experiment | Phase | Reason |
| --- | --- | --- |
| `substrate-contract` | Linux adapter | Needs Linux: symlink races, aliases, foreign writers |
| `event-crash-replay` | 2 | The logical ordering is modeled in PR 4; physical fsync and crash testing needs the storage adapter |
| `fence-interleavings` | 3–4 | The authority model is in PR 4; transport races need Loom |
| `identity-rollback` | 3–4 | Needs enrollment, snapshots, and clones |
| `secret-nondisclosure` | 6 | Wrapper and serialization checks are in PR 2; provider paths need an adapter |
| `incremental-ablation` | after milestone | Needs the full recomputation path as the reference |
| `plan-witness` | after milestone | Needs a baseline planner to check against |

## Exit Criteria

1. Five pull requests merged, each with its stated exit demonstrated by a named test.
2. A result record under `results/` for every experiment in the table above, none claiming more than it ran.
3. `docs/formal/verification-matrix.md` filled from records, with *not run* and *inconclusive* wherever true.
4. ADRs closed: `warp-gates`, `recovery-authority` (the Cell part), `verification-gates`, and the remainder of `canon-artifact`.
5. Spec §62 updated: edge semantics closed, the encoding question closed.
6. The required CI path is small and green, with `--locked`, the layer checker, and the snapshot freeze in it.
