# Result: substrate-contract

- **Experiment.** `substrate-contract`, milestone `07-substrate-conformance`.
- **Question.** Does the Linux adapter honor the `nomos-substrate` contract the mock honors: absence distinct from denial, no mutation through Trace, postconditions assessed by core?
- **Outcome.** Yes, for its first operation, regular files beneath a root directory, on one host. The mock and the Linux adapter pass one conformance suite of nine clauses. Its two negative controls, a mutating Trace and a denied read reported as absence, fail it on each adapter. The production driver converges a Canon through each adapter, and the Linux adapter meets symbolic links, a named pipe, a directory in place of a file, a foreign writer, a planted hard link, and a real permission denial without a false absence or an unauthorized change. Mutation testing found a real defect, an observation that could block on a named pipe, and four test gaps, all closed.

## Context

| Field | Value |
| --- | --- |
| Contract | [substrate-contract.md](../../../formal/substrate-contract.md), written before the code; ADR 0013's note, 2026-09-29 |
| Adapter | `nomos-substrate-linux`: `openat2` with `RESOLVE_BENEATH`, `RESOLVE_NO_SYMLINKS`, and `RESOLVE_NO_MAGICLINKS` relative to a root descriptor, through `rustix`; atomic replacement from a content store; no shell |
| Suite | `crates/bin/nomos-cell/tests/conformance/mod.rs`, one check per clause, generic over the adapter; `tests/substrate_conformance.rs` runs it and the Linux failure injection |
| Host | Ubuntu 24.04.4, Linux 6.18.44, an ext-family file system, in a container; the tests ran as root with the permission-override capabilities dropped per thread; Rust 1.98.1 |
| Receipts | `verification/receipts/2026-09-29-substrate-conformance.ndjson` |

## The Suite

| Clause | What the check does | Mock | Linux |
| --- | --- | --- | --- |
| S1, coverage | Observes a present file, a missing one, and one at the root; each has an Observation and nothing else does | passed | passed |
| S2, truthful absence | A denied read is `Failed(PermissionDenied)`; a missing file is `Collected(Absent)` | passed | passed |
| S3, evidence | An empty file, a small one, and one of 300,000 bytes, larger than a single read: the digest of the bytes and their count | passed | passed |
| S4, observation does not mutate | Three rounds of observation change neither the files nor the execution count | passed | passed |
| S5, postconditions are core's | Six requirement and starting-point pairs: core's `assess` on a new Observation is Satisfied, and `changed` is true exactly when the file changed | passed | passed |
| S6, at most once per key | A repeated key after a foreign write returns the first receipts and changes nothing | passed | passed |
| S7, refusal before effect | Each request the adapter cannot perform is `Refused` and changes nothing: a refresh of a plain file on the mock; on Linux a refresh, content not in the store, a missing parent, and a directory | passed | passed |
| S8, settlement | Every receipt sequence ends in a settling receipt | passed | passed |
| S9, one clock | Each window lies between two readings of the adapter's clock | passed | passed |

## Evidence

| Check | Harness | Result |
| --- | --- | --- |
| The suite | `the_mock_passes_the_suite`, `the_linux_adapter_passes_the_suite` | Both pass all nine clauses |
| Negative controls | `a_mutating_trace_fails_the_suite_on_{the_mock,linux}`, `a_denied_read_reported_as_absence_fails_the_suite_on_{the_mock,linux}` | Each wrapper fails its clause on each adapter: S4 and S2 |
| End to end | `the_driver_converges_{the_mock,linux}`: the production driver enforces one file with exact content | Converged; a second Enforce executes nothing (N3); after a foreign write and a denial, the run ends `Indeterminate(PermissionDenied)` with no execution (N13) |
| Unresolved effect | `an_unresolved_effect_keeps_its_reservation_on_{the_mock,linux}`: receipts stripped of settlement | The kernel keeps the reservation and the run does not end `Converged` (N10) |
| Symbolic links | At the leaf; in a parent, relative and inside the root; pointing outside the root | Each observation is `Unsupported` and each replacement `Refused`; the linked files are unchanged |
| Other file types | A directory, a named pipe | `Unsupported`, returned at once; replacement refused |
| Paths that cannot exist | A path under a regular file | `Absent`, truthfully (`ENOTDIR`) |
| Foreign writer | A write between execution and verification | Core assesses the file as it is: a Variance |
| Atomic replacement | After a replacement; with a hard link planted at the temporary name; with another execution's temporary file left behind | No temporary file remains; the planted link fails the execution and the linked file is unchanged; the stale file does not block the later execution |
| Opening | A root that is not a directory | Refused, rather than making every path look absent |
| The denial is real | `the_denial_is_real` | The test thread cannot read the file through the standard library either |
| Semantic mutants | `SM-SUBSTRATE-001` to `004` | Each caught by its named test; `SM-SUBSTRATE-002` after its test was strengthened |

## Mutation

`cargo mutants -p nomos-substrate-linux --test-package nomos-cell`, three recorded runs. The first two found:

- **A real defect.** Observation opened the resource without `O_NONBLOCK`, so a named pipe at a managed path would block the adapter until a writer appeared. `a_named_pipe_is_unsupported_and_does_not_block` hung for 60 seconds against the old flags and passes now.
- **Test gaps, closed.** The suite did not check a file's size, and the mock reported 1 for every file; it now keeps the size a foreign writer gives it. Nothing compared the execution count with a known start. A root that is not a directory was not refused. Nothing depended on `O_EXCL` for the temporary file, or on the temporary name differing between executions.

The final run, at commit `230503c`: 76 mutants, 31 caught, 18 unviable, 3 timeouts, and 24 survivors, each classified.

| Survivors | Class | Why |
| --- | --- | --- |
| 16 replacements of `\|` by `^` in open flags | Equivalent | The flags are disjoint bits |
| Dropping `O_CLOEXEC` (3) | Equivalent here | A descriptor leaking into an executed child cannot be observed in process; the adapter executes nothing |
| Dropping `O_NOFOLLOW` from observation (2) or from the temporary file (1) | Equivalent | `RESOLVE_NO_SYMLINKS` refuses a link at the leaf, and `O_CREAT` with `O_EXCL` never follows one |
| Dropping `O_DIRECTORY` from the parent (2) | Equivalent in result | A parent that is not a directory fails the next call with `ENOTDIR`, and the request is refused the same way |
| The read buffer's size arithmetic | Equivalent | Performance only |

The three timeouts are a dropped `O_NONBLOCK` twice and an endless read loop; each hangs a test, which fails the run.

## Negative Controls

- **A mutating Trace** removes what it is asked to observe. S4 fails on both adapters.
- **A denied read reported as absence.** S2 fails on both adapters.
- **The symbolic-link test of a parent** first survived `SM-SUBSTRATE-002`: its link was absolute, which `RESOLVE_BENEATH` already refuses. The test now uses a relative link that only `RESOLVE_NO_SYMLINKS` refuses, and checks first that the standard library follows it.

## Unchecked

- One host: one kernel, one file system, one container, as root. Other kernels, file systems, network file systems, and an unprivileged service account are not run.
- Races: a link swapped in between two system calls of one execution is argued in the contract, not provoked.
- Crash consistency of the rename sequence, and whether the directory sync reaches the disk.
- File metadata beyond mode `0644`, directories, and every resource but regular files; the service refresh waits for `systemd_unit`.
- Content delivery: the content store stands in for it.
- The mock's size for a file it wrote itself is 1, since it holds digests and not bytes; the suite checks sizes of files a foreign writer placed.

## Decision

The Linux adapter's first operation honors the contract the mock honors, on this host. The environment column of the verification matrix gains its first entries, each naming this host. ADR 0013 stays Proposed: its §3 and §4 have not been exercised.
