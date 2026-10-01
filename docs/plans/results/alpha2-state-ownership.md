# Result: alpha2-state-ownership

- **Milestone.** `17-alpha2-state-ownership`, the Cell's half of [issue #29](https://github.com/kmosoti/Nomos/issues/29), `v0.1.0-alpha.2`: issues #30 (one owner of a state directory), #36 (a read-only Trace), and #37 (a state directory the Cell can trust). The contract is [cell-commands.md](../../formal/cell-commands.md), State Directory, written before the code.
- **Question.** Can two Cell processes mutate one state directory, can `trace` change the Cell's own state, and can the Cell be made to trust a state directory it should not?
- **Outcome.** Not after this change, on Debian 12 and 13 and on the development host. A second `enforce` or `import` of a state directory another process holds ends with status 1 and `already in use`, having appended nothing, imported nothing, and issued no effect; a killed holder gives the directory up with no cleanup. `trace` and `events` left a normal, absent, torn, and corrupted journal byte for byte as they found it. A state directory, lock file, journal, content directory, or blob that is open to group or others, a link, or owned by another user was refused before the journal was replayed.

## Context

| Field | Value |
| --- | --- |
| Commit | `feb0d75` for every receipt but the last three, and `1a73c59` for the second `cargo-test`, `mutants-semantic`, and `receipts-validate`, the commit that regenerated the `SM-STORE-001` patch |
| Hosts | The development host, Ubuntu 24.04.4, Linux 6.18.44; Debian GNU/Linux 12 and 13 in privileged containers of `tests/fixtures/debian/Dockerfile`, run by `cargo xtask debian`, on that same kernel |
| Oracle | The State Directory section of cell-commands.md; the journal's bytes and modification times, read with `std::fs`, never through the Cell |
| Harness | `crates/bin/nomos-cell/tests/state_ownership.rs` (16 tests); the unit tests of `crates/adapters/nomos-store-fs/src/state.rs` and `lib.rs` |
| Receipts | `verification/receipts/2026-10-01-alpha2-state-ownership.ndjson` |

## What Was Built

- **The lease.** `StateLease` in `nomos-store-fs`: `flock(LOCK_EX | LOCK_NB)` on `lock` in the state directory, taken by `enforce` and `import` before the journal is replayed, content is imported, a Plan is accepted, or an effect is issued, and held until the process ends. The lock belongs to the open file, so the kernel releases it when its holder dies.
- **Read-only commands.** `FileLog::inspect` opens the journal for reading only, never creating it and never truncating a torn tail. `trace` and `events` use it, report a torn tail on standard error as left in place, and take no lease.
- **The safety rule.** One check, applied to the state directory, `lock`, the journal, `content/` and its directories, and every blob read: the right type, not a link (opened with `O_NOFOLLOW`), owned by the effective user, and no permission bit for group or others. The root is created `0700` and fixed with `fchmod`, so the umask cannot weaken it; `lock` and the journal are created `0600`.

## What the Tests Establish

| Property | Test | Oracle |
| --- | --- | --- |
| A second `enforce` of an owned state directory is refused and does nothing | `enforce_is_refused_while_another_process_holds_the_state`; `a_separate_process_running_enforce_is_refused_as_the_service_would_be` | The specification; a child process holds the lease, so the contenders are real processes |
| `import` is refused likewise | `import_is_refused_while_another_process_holds_the_state` | The specification |
| The lease is held until the run ends | `the_lease_is_held_for_the_whole_run` | A second process polling for the lease during a run of 30 Actions never took it while the journal was still growing |
| A killed holder is released without cleanup, to a waiting contender | `a_killed_holder_releases_the_state_to_a_waiting_contender` | The kernel's release of a lock when its process dies |
| Different directories are independent | `different_state_directories_proceed_independently` | The specification |
| `trace` and `events` change no byte or time of a normal, absent, torn, or corrupted state | `trace_and_events_leave_a_normal_journal_as_they_found_it`, `trace_and_events_create_nothing_when_there_is_no_journal`, `a_torn_journal_is_not_repaired_by_trace_or_events`, `a_corrupted_journal_is_an_error_to_trace_and_events_and_is_not_touched` | A fingerprint of every file taken before and after, with the host's own `std::fs` |
| A read-only command reads a directory another process owns | `trace_and_events_read_a_state_directory_another_process_owns` | The specification |
| An unsafe root, lock file, journal, content directory, or blob is refused before replay, by every command | `an_unsafe_state_directory_is_refused_by_every_command`, `an_unsafe_journal_or_content_store_is_refused_before_replay`, and the unit tests of `state` and the content store | Modes, owner, and links set by the test with `std::fs` |
| A custom state directory works as the packaged one does, and is private | `a_custom_state_directory_the_cell_made_is_private_and_works` | The modes `0700` and `0600` the specification names |

This is **process concurrency on the host's `flock`**. It is not the simulator's interleaving of kernel inputs (`bounded_convergence`, `refresh_recovery`), which show that the kernel's decisions do not depend on delivery order and say nothing of two Cells at once.

## Negative Controls

| Mutant | Wrong behavior | Caught by |
| --- | --- | --- |
| `SM-CELL-005` | `enforce` does not take the lease | `enforce_is_refused_while_another_process_holds_the_state` |
| `SM-CELL-006` | `enforce` takes the lease and drops it at once | `the_lease_is_held_for_the_whole_run` |
| `SM-CELL-007` | `trace` and `events` open the journal with the repairing open | `a_torn_journal_is_not_repaired_by_trace_or_events` |
| `SM-STORE-005` | The safety check ignores access by the owner's group | `state::tests::a_state_directory_open_to_group_or_others_is_refused` |
| `SM-STORE-006` | The state directory is opened following a link | `state::tests::a_state_directory_that_is_a_link_is_refused` |
| `SM-STORE-007` | The journal is opened without the safety check | `an_unsafe_journal_or_content_store_is_refused_before_replay` |

All 75 active mutants of the corpus were caught.

## Findings

- **A forked child shares the lock.** A first run of the suite failed one test with `already in use` for a directory no test held. `flock` locks belong to the open file description, and a process another test's thread forked shares every descriptor of the test binary until it execs, so a lease released a moment earlier could still look held. The Cell's lock file is close-on-exec, so no process it starts keeps it; the tests now run one at a time, and the suite passed five runs in a row.
- **The old `trace` could write.** Opening a journal with a torn tail truncated it, so `trace` could change the Cell's own state while leaving the host alone. The operator guide's "changes nothing" was true of the host only; it is now true of both.
- **The first receipt run failed twice, and said so.** `cargo-test` and `mutants-semantic` were recorded as `failed` at `feb0d75`: the content store's new safety check moved the lines `SM-STORE-001` patches, so its patch no longer applied and the corpus reported one mutant not caught, once through the corpus test and once through `cargo xtask mutants semantic`. The patch was regenerated with the same wrong behavior (`SM-STORE-001` is caught by the same test) in its own verifier commit, and the two checks were recorded again after it. The two failed receipts stay in the receipts file; they are what happened.
- **The lease needs a second process to test.** `flock` on two descriptors of one process also conflicts, so an in-process test shows contention, but only a separate process shows that a kill releases the lock, which is why the holder is a child process.

## What This Does Not Establish

- Fencing between Cells on different machines, and ownership conflicts between Nomos and other configuration-management systems: both are out of scope of #30.
- The directories above the state directory. They are the operator's, and an assumption of the security model, not something the Cell checks.
- A file system without `flock`, such as some network file systems: the state directory is expected to be local disk.
- A user with write access to the directory while the Cell is not running: the lease is advisory, and the safety rule refuses what it can see afterward but cannot stop the write.
- Read-only commands racing an `enforce`: they may see a final record the writer has not finished, and read it as cut short.

## Decision Fed

The state directory has one owner at a time and is trusted only if it is private. The remaining alpha.2 issues build on it: #34, the security model, describes this as the mechanism it relies on.
