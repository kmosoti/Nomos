# Result: durable-log

- **Experiment.** `durable-log`, owned by milestone `10-durable-cell` of the [Phase 1 plan](../phase-1-masterless-cell.md).
- **Question.** Does recovery from disk restore every Obligation and Event that was acknowledged, and refuse a torn or corrupted log?
- **Outcome.** Yes, for a Cell killed in-process after each of the 11 inputs of the refresh scenario, 8 of them with a refresh owed: every reopened Cell held exactly the snapshot it had when it died, its Obligations included, and converged with the service running the new configuration. A torn final record was truncated and reported; a changed byte anywhere in a whole record, or a value that does not decode, failed the open.

## Context

| Field | Value |
| --- | --- |
| Commit | `c899a08`, where every receipt was recorded |
| Host | The development host, Ubuntu 24.04.4 on Linux 6.18.44; the journal on its file system |
| Oracle | The snapshot of an uninterrupted run after the same input, from the pure kernel `step` of `05-transition-kernel`; the Event Log of the in-memory driver; and ADR 0017 §3's recovery rules |
| Harness | `crates/bin/nomos-cell/tests/durable_log.rs`; `crates/adapters/nomos-store-fs/src/log.rs`; `crates/app/nomos-app/src/journal.rs` |
| Receipts | `verification/receipts/2026-09-30-durable-cell.ndjson` |

## What Was Built

The Cell journals the kernel's inputs, not its Events, as [ADR 0017](../../adr/0017-local-store.md) §3 now states: an Event can carry a `Verified`, which only core may produce, so a decoder outside core would have to forge one. `JournaledCell` steps an input, appends it to the journal, and only then issues its effects; opening it recomputes the snapshot by stepping every journaled input from the initial one. `FileLog` keeps the journal in one append-only file of records, each with a checked length and a SHA-256 digest, synced before `append` returns. The journal encodes each input as deterministic CBOR in the Canon IR's profile, with a version field.

## Kill at Every Input

`a_cell_killed_after_every_input_recovers_its_snapshot` first runs the refresh scenario of `05-transition-kernel` without interruption, one input at a time, and keeps the snapshot after each. Then, for each input, a fresh Cell runs the scenario to that input and is dropped: its queue and snapshot are gone, the mock host keeps what was done to it, and the file keeps what was appended. The Cell reopened from the file alone must hold the recorded snapshot and the same Obligations, then handle `Recovered`, let time pass every settle-by instant, and enforce again. Every one converged, with the service running the configuration on disk.

| Test | Oracle | Result |
| --- | --- | --- |
| `a_cell_killed_after_every_input_recovers_its_snapshot` | The uninterrupted run's snapshot after the same input | passed for all 11 kill points |
| `the_journal_recomputes_the_event_log` | The in-memory driver's Event Log, snapshot, and host | Equal |
| `every_journaled_input_reads_back_as_itself` | Round trip of every input of the scenario | passed |
| `journal::tests::every_input_shape_reads_back_as_itself` | Round trip of every input shape and every resource family; integers at and past the IR's largest, $2^{53}-1$, up to `u64::MAX` | passed |
| `journal::tests::anything_else_decodes_to_nothing` | No bytes, an encoding cut short, and another version; every single-bit flip of an encoding | The first three decode to nothing; every flip decodes without a panic, to an input or to nothing (the record's digest, not the codec, detects a flip) |
| `a_journal_that_refuses_stops_mutation` | ADR 0012 §5 | The input whose Decision would write is refused; no effect, no snapshot change |

## Damage to the Journal

| Test | Injected | Result |
| --- | --- | --- |
| `log::tests::a_torn_final_record_is_truncated_at_every_length` | The last record cut to every shorter length | Truncated and reported; the earlier records read back |
| `log::tests::a_corrupted_record_fails_the_open` | Each byte of a two-record log flipped in turn; a record whose value does not decode | `Corrupt` at the damaged record's offset for every byte; `Undecodable` |
| `a_changed_journal_is_refused_and_a_torn_one_is_repaired` | A byte flipped in the middle of a real journal; five bytes cut from its end | `Corrupt`; the journal reopens without its last, unacknowledged batch |

The length check was found while writing the second test: without it, flipping a bit of a record's length made the record appear to run past the end of the file, so recovery took every later record for a torn tail and truncated acknowledged batches. ADR 0017 §3 was amended before the code.

## Semantic Mutants

| Mutant | Wrong behavior | Caught by |
| --- | --- | --- |
| `SM-STORE-002` | No check of a record's length | `log::tests::a_corrupted_record_fails_the_open` |
| `SM-STORE-003` | A value that does not decode is skipped | the same |
| `SM-STORE-004` | The snapshot recomputed from the last input only | `a_cell_killed_after_every_input_recovers_its_snapshot` |
| `SM-TRANSITION-010` | Effects issued before the input is journaled | `a_journal_that_refuses_stops_mutation` |

All 46 active mutants of the corpus were caught.

## What This Does Not Establish

- Power loss. The Cell was killed by dropping it in the same process; what the file system and the disk do with a synced write when power fails is Phase 2's physical crash testing.
- A crash in the middle of one `write`: the torn tails were made by truncating a whole file, which is the shape such a crash leaves on a file system that does not reorder a file's appended bytes.
- The Linux Substrate under the journal: the host in the kill test is the mock. A real unit's restart after a crash is `11-systemd-unit`'s exit.
- Retention and compaction of the journal, which ADR 0017 §4 leaves open, and a journal written by an earlier version of Nomos.

## Decision Fed

The Cell's Obligations survive a restart. ADR 0017's second acceptance criterion is met, so the ADR is Accepted. `11-systemd-unit` runs the same kill against a real service manager.
