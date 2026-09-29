# Result: controller-composition

- **Experiment.** `controller-composition`, after milestone `05-transition-kernel` and before `07-substrate-conformance`.
- **Question.** Do explicit footprints with stated guarantees and reliance assumptions detect a harmful composition of two controllers that each converge alone?
- **Outcome.** On the mock host, yes, for the footprints the harness declared. The composition check rejects the shared configuration with a shared write on the file, before anything executes. Run without the check, the same pair oscillates: every sweep rewrites the file twice, and the joint projection repeats at the second sweep. The rival holds for the kernel's own diagnostic: all four sequential runs end Converged, and across 25 start offsets no run ends NonConvergent. The disjoint and single-owner configurations are accepted and reach a joint fixed point in two sweeps, in either order and at every tested offset. The check reads declarations, not effects: a write left out of a footprint passes it.

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1 |
| Check | `nomos_core::footprint::compose`, in `crates/core/nomos-core/src/footprint.rs`, under the working definitions of [ADR 0008](../../../adr/0008-ownership-and-identity.md)'s 2026-09-29 note and [reconciliation.md](../../../formal/reconciliation.md#composition) |
| Kernel | `nomos_app::kernel::step`, one instance per controller, each with its own Canon, Event Log, and fence |
| Host | `nomos-substrate-mock`: three files at revision 1, and a stopped service whose configuration is the shared file |
| Harness | `crates/bin/nomos-cell/tests/controller_composition.rs`, with its own driver for several kernels on one host; the single-kernel simulator in `tests/support/mod.rs` is not used |
| Receipts | `verification/receipts/2026-09-29-controller-composition.ndjson` |

## What Was Built

**The footprint.** A `Property` is text of the form `<kind>:<name>`, compared as text. A `Footprint` names one controller and three sets: the guarantee (what it writes), the reliance (what it relies on no other controller writing), and the reads. `reliance()` is the guarantee joined with the declared reliance, because a controller that converges alone on a property does so as its only writer. `compose` takes any number of footprints and returns `Ok` or every `Interference`, sorted: `SharedWrite`, `RelianceViolated`, or `DuplicateController`. It is pure and lives in `nomos-core`; no crate, port, or dependency was added.

**The harness.** A controller is one transition kernel with its own queue of deliveries, and every controller's reads and writes reach one `MockHost`. The harness carries requests and results as the milestone's simulator does and decides nothing a kernel decides. It adds three things the kernel does not have:

- **Sweeps.** Each controller in turn presents a new Plan (bound 3) and runs it to its end while the others are idle.
- **The joint projection.** After each sweep, every resource of every composed Canon is observed. A sweep in which every run ends Converged and nothing executes is a *joint fixed point*. A sweep that executed and returned the joint projection to one seen after an earlier sweep is *oscillation*. The harness stops at either, or after six sweeps.
- **An execution audit.** Every execution the mock performed is attributed to its controller and checked against that controller's guarantee.

Concurrent runs start controller A, start B a number of harness steps later, and deliver one item per controller in turn until both runs end. The offsets run from 0 to 24; at offsets 0 to 9, B starts before A's run has ended.

| Configuration | A's Canon | B's Canon | B declares beyond its Canon | Check |
| --- | --- | --- | --- | --- |
| Disjoint | `/etc/a.conf` holds revision 2 | `/etc/b.conf` holds revision 3 | nothing | Accepted |
| Single-owner | `/etc/shared.conf` holds revision 2 | `/run/svc` runs; starting it loads `/etc/shared.conf` | reads `file:/etc/shared.conf#content` | Accepted |
| Shared | `/etc/shared.conf` holds revision 2 | `/etc/shared.conf` holds revision 3 | nothing | Rejected: `SharedWrite` |
| Declared reliance | as single-owner | as single-owner | relies on `file:/etc/shared.conf#content` | Rejected: `RelianceViolated` |
| Undeclared write | as shared | as shared | a guarantee naming `/etc/b.conf` only | Accepted |

Each guarantee is derived from the controller's Canon, one property per managed resource, except in the undeclared-write case, where the harness replaces B's footprint to lie.

## Evidence

Two oracles sit outside the check. The truth table was written from the working definitions, not from the code. The oscillation and the fixed points are computed from Observations of the mock host, which the check never sees.

| Check | Harness | Result |
| --- | --- | --- |
| Each converges alone | `each_controller_converges_alone`: every controller of the three configurations, alone on a fresh host | All six reach a fixed point at the second sweep, after one execution each, every run Converged |
| Disjoint together | `the_disjoint_configuration_is_accepted_and_converges_together`, orders A then B and B then A | Accepted; a joint fixed point at sweep 2 in both orders, two executions, every run Converged, each file holding its controller's revision |
| Single-owner together | `the_single_owner_configuration_is_accepted_and_converges_together`, both orders | Accepted; a joint fixed point at sweep 2 in both orders, two executions, every run Converged. With A first the service loaded revision 2; with B first it loaded revision 1, which neither Canon expresses |
| Accepted configurations, concurrent | `accepted_configurations_converge_at_every_start_offset`, 25 offsets each | 50 of 50 pairs of runs end Converged; each is followed by a joint fixed point at the first sweep; the audit finds every execution inside its guarantee |
| Shared, sequential | `the_shared_configuration_oscillates_and_the_check_rejects_it` | The check returns one `SharedWrite` on `file:/etc/shared.conf#content`, writers A and B, and the host is never reached. Without the check: oscillation at sweep 2, writes by A, B, A, B, all four runs Converged, each write inside its writer's guarantee |
| Shared, concurrent | `shared_runs_oscillate_at_every_start_offset`, 25 offsets | Table below. No run ends NonConvergent at any offset; every offset is followed by oscillation within two sweeps |
| Truth table | `footprint::tests::two_controllers_on_one_property_truth_table`: two controllers, one property, each in one of four roles (none, reads, relies, writes) | 16 of 16 rows as the table says: rejected exactly for writes and writes, relies and writes, and writes and relies |
| Metamorphic relations | `footprint::tests::order_and_disjoint_additions_do_not_change_the_verdict`: three controllers with two shared writes and one violated reliance, in five orders, with and without a fourth disjoint controller | The same three interferences, in the same order, every time |

## Measurements

| Configuration, concurrent | Offsets | Overlapping | A Failed | B Failed | Both Converged | NonConvergent | Joint result after |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Disjoint | 25 | 10 | 0 | 0 | 25 | 0 | fixed point, sweep 1 |
| Single-owner | 25 | 10 | 0 | 0 | 25 | 0 | fixed point, sweep 1 |
| Shared | 25 | 10 | 4 | 2 | 19 | 0 | oscillation, sweep 1 or 2 |

Every Failed run in the shared rows lists the shared file as a known failure, with no unknown outcome, and its log records the Action failing on its postcondition: the other controller wrote between the Action's completion and its verification. Four of the ten overlapping offsets ended with both runs Converged: each run executed once, so the other's write landed after its last Observation. Where the kernel notices the other writer at all, it reports a failed postcondition, not interference.

## Negative Controls

- **The shared configuration.** It must oscillate on the mock and be rejected by the check, and it does both. Semantic mutant `SM-COMPOSE-001` drops the guarantee from the reliance, so the check accepts two writers of one property; `the_shared_configuration_oscillates_and_the_check_rejects_it` catches it. The property is N2: each run's Converged is sound at its last Observation, and under a shared write the next run of the other controller makes it false. `cargo xtask mutants semantic` caught all 25 active mutants.
- **An undeclared write.** `an_undeclared_write_passes_the_check_and_the_execution_audit_sees_it`: the check accepts the lying pair, the pair oscillates at sweep 2, and only the audit sees B write outside its guarantee. The check is as good as its footprints.
- **A declared reliance.** `a_declared_reliance_rejects_a_pair_that_converges`: B relying on the shared file, rather than reading it, is rejected, and the same pair reaches a joint fixed point at sweep 2 without the check. The check is conservative about what a footprint says, not complete about what converges.

## Unchecked

- Everything here ran on the mock host and a harness driver. No Linux file, service, or sysctl was touched; the mock has no sysctl, so the sysctl half of the plan's harness was not run.
- One contested property, two controllers on the host. The check was exercised with up to four footprints in unit tests only.
- Interleavings: 25 start offsets of one round-robin schedule, at the granularity of one delivery per controller per step. Other schedules, including a foreign write inside a run, are covered only by `bounded-convergence`'s single-controller scripts.
- Both controllers are Nomos kernels. A writer outside Nomos, such as a tuning daemon, has no footprint, and ADR 0008 §2 leaves it to the per-run oscillation diagnostic, which this experiment shows cannot see a writer that takes turns with runs.
- Properties are compared as text. Two keys that reach one inode or one service are two properties to the check (ADR 0008 §3), and no case here aliases.
- Footprints are derived from Canons by the test, for the file and service kinds only. Nothing in production derives them, and Canon admission does not call the check.
- Conflict policies and contested properties (ADR 0008 §1 and §2) are not modeled.
- In the single-owner configuration, the order of the sweep decides which revision the service loaded. Neither Canon expresses that, so it is not an interference under the working definitions; a Canon that cares states it, as design 2 of [refresh-recovery](refresh-recovery.md) does.

## Decision

The decision rule is met on the simulator. Two safe compositions, disjoint and single-owner, are demonstrated, and one counterexample, the shared file, is detected: by the composition check before execution, and by the harness's joint projection when the check is bypassed. The kernel's own oscillation diagnostic does not detect it, because its fingerprint history lives in one run.

That supports ADR 0008 §1 as a declaration check at admission. It also finds a gap in §2's reliance on the per-run diagnostic for undeclared writers: a writer that acts between runs is invisible to it. Detecting one needs history across runs of the same Canon, which nothing implements. No whole-host claim follows from this record, and acceptance of ADR 0008 is the owner's.
