# Substrate Contract

What every adapter of the `nomos-substrate` port promises, how the conformance suite checks it, and the first operation the Linux adapter performs. [ADR 0006](../adr/0006-kernel-contract.md) keeps semantics in core: an adapter obtains evidence and performs operations, and never decides what satisfaction means. [ADR 0013](../adr/0013-trust-boundaries.md) splits the port into an observe and a mutate capability and resolves paths under a boundary. This document gives the working definitions milestone `07-substrate-conformance` implements and tests.

## The Port

The port has two capabilities. `Observe::observe(resources)` returns Observations. `Mutate::apply(request)` performs one execution, named by its idempotency key, and returns its receipts in order. Both are synchronous. Trace is given an `Observe` and nothing else.

## The Clauses

Every adapter, the mock and Linux alike, satisfies these. The conformance suite checks each clause by name.

| Clause | Statement |
| --- | --- |
| S1, coverage | For each requested resource, `observe` returns at least one Observation of that resource, and none of a resource not requested. |
| S2, truthful absence | An Observation is collected as absent only when the adapter established that no file exists at the resource. A read the adapter could not perform is a failed collection with its reason, never an absent file. |
| S3, evidence and not verdicts | A present file is reported with the digest of its content and its size. The adapter reports what it saw; the types give it no way to report Satisfied, a Variance, or Indeterminate. |
| S4, observation does not mutate | `observe` changes nothing in the managed-resource projection (N1): after any sequence of observations, every managed resource holds what it held before. |
| S5, postconditions are core's | A `Completed` receipt says that the execution finished and whether it changed the resource, and nothing more. Whether the Condition holds is decided by core's `assess` on a new Observation. The `changed` flag is true exactly when the resource differs from before. |
| S6, at most once per key | A request whose key the adapter has seen returns the receipts it returned then and changes nothing. |
| S7, refusal before effect | An operation the adapter cannot perform is `Refused`, and nothing changes. |
| S8, settlement | The effect causes no change after its settle-by instant. An adapter that completes an execution before `apply` returns ends every receipt sequence with a settling receipt: `Completed`, `Failed`, or `Refused`. |
| S9, one clock | The collection window of an Observation is on the clock the driver ticks the kernel with, so that the kernel's freshness comparisons are meaningful. |

The suite also checks one property of the kernel at the port: **an unresolved effect keeps its reservation**. When an adapter's receipts for an execution carry no settlement evidence, the kernel keeps the Action's reservation, and the run does not end `Converged` (N10). This is the kernel's rule, checked at the port because the port is where settlement evidence goes missing.

## The Clauses per Family

The clauses name files, as the first operation did; from milestone `08-resource-families` they hold for every family of [resource-families.md](resource-families.md), read this way:

- **S2.** Absence is family-specific evidence: no file or directory at the path, no account of the name, a package dpkg records as not installed. A unit systemd does not know and a sysctl the kernel does not have are failed collections, `unavailable`, never absence.
- **S3.** A family reports the evidence its table lists, and nothing that judges it.
- **S5.** `changed` is true exactly when the family's evidence would differ from before: a unit started, a value written that was not there, a package installed.
- **S7.** Each family's refusals are part of its contract: removing a directory with entries; a file where a directory is, or a directory where a file is; changing a user's class; enabling or disabling a unit whose unit-file state is fixed; an operation on a unit or a kernel parameter the host does not know; a requirement of another family than the resource's; a refresh of anything but a unit or the legacy service; and any operation on a family the adapter does not serve.

The conformance suite runs every clause for every family an adapter serves, from a list of requirements and starting points per family taken from its truth table; S8 is checked on every execution S5 and S7 perform. The mock serves all seven; the suite runs for the six Phase 1 families, and the legacy service keeps the file suite's and the transition kernel's checks. The Linux adapter serves a family from the milestone that implements it; until then, a key of the family is observed as a failed collection, `unsupported`, and every operation on it is refused.

## The Conformance Suite

The suite is one set of tests, generic over the adapter under test, at the composition boundary `crates/bin/nomos-cell/tests/`. Each adapter supplies a test subject: the adapter, plus a way to arrange the world outside it, as a foreign writer would. The subject can write a file's content, remove a file, deny reads of a file, and read the ground truth of a file without going through `observe`.

**Negative controls.** Two wrappers break a clause on purpose, and the suite must fail each:

- **A mutating Trace.** Observation writes to the resource it observes. S4 must fail.
- **A denied read reported as absence.** A failed collection is turned into an absent file. S2 must fail.

**End to end.** For each adapter, the production driver enforces a Canon of one file with exact content and converges; enforcing it again executes nothing (N3); a denied read ends the run with the resource Indeterminate and no mutation (N13).

## The Linux Adapter's First Operation

The adapter manages regular files beneath one root directory. The root is `/` in production and a scratch directory in tests; a resource path names a file relative to it.

**Resolution.** Every path is resolved at use, relative to an open descriptor of the root, with `openat2` and the resolve flags `RESOLVE_BENEATH`, `RESOLVE_NO_SYMLINKS`, and `RESOLVE_NO_MAGICLINKS` (ADR 0013 §2). No symbolic link is followed, at the leaf or in any parent, and nothing resolves outside the root.

**Observation.** Each result of opening and reading the file maps to one collection:

| Result | Collection |
| --- | --- |
| The file is regular and its bytes were read | Collected, present, with the Secure Hash Algorithm (SHA) 256 digest of the bytes, their count, and the owner and group by number and the mode, as `fstat` reports them |
| No file at the path (`ENOENT`), or a parent that is not a directory (`ENOTDIR`) | Collected, absent |
| Permission denied (`EACCES`, `EPERM`) | Failed, permission denied |
| A symbolic link on the path (`ELOOP`), a path that would leave the root (`EXDEV`), or something other than a regular file | Failed, unsupported |
| Any other error | Failed, input and output (`Io`) |

The collection window is the monotonic clock's reading before the open and after the read.

**Mutation.** `Converge` with a file requirement makes the file satisfy it, and nothing else is supported:

| Requirement | Effect |
| --- | --- |
| Absent | Remove the file if it exists. Completed, changed exactly when a file was removed |
| Present, any content | Create an empty file if none exists. Completed, changed exactly when one was created |
| Present, exact content | Write the content whose digest the requirement names, atomically: a temporary file in the same directory, written and synced, given mode `0644`, renamed over the target, and the directory synced (spec §11). Completed, changed exactly when the content differs from before |

The adapter refuses (S7), changing nothing, when: the operation is a refresh, which waits for the systemd resource; the resource is not a file; the requirement states owner, group, or mode, which `09-file-and-directory` manages; the content of an exact requirement is not in the adapter's content store; the parent directory does not exist, since directories are a separate resource; or something other than a regular file is at the path. If the rename fails after the temporary file was written, the temporary file is removed and the receipt is `Failed`.

**Content.** An exact requirement carries a digest, not bytes. The adapter holds a content store, a map from digest to bytes, filled by whoever constructs it; the store is how content reaches a Cell until artifact content delivery is specified.

**Execution.** Each execution completes before `apply` returns (S8). The adapter remembers the receipts of every key it has seen (S6).

## Failure Injection on Linux

Beyond the suite, the Linux adapter is tested against what a real file system can do to it: a symbolic link at the resource and one in a parent directory, a directory where a file is expected, a foreign writer that changes the file between an execution and its verification, and a read denied by file permissions. A test that must deny a read to a process with the capability to override file permissions drops that capability for its own thread first.

## Known Gaps

- **File metadata.** Owner, group, and mode are observed, by number, but not managed: a requirement that states one is refused until `09-file-and-directory`.
- **Directories, and every resource but regular files.** They arrive with their resource families; `systemd_unit` brings the refresh.
- **Content delivery.** How content reaches a Cell with its Canon is open; the content store stands in for it.
- **Privilege separation.** The mutation helper of spec §55 is Phase 5; the adapter here runs with the test's privileges.
- **Crash consistency of the rename.** The sequence follows spec §11, and a crash between its steps is not tested.
