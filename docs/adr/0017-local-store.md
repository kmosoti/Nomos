# ADR 0017: The Cell's Local Store

- **Status.** Accepted 2026-09-30, when `10-durable-cell` met the second acceptance criterion
- **Date.** 2026-09-30
- **Provenance.** Decision 3 of the [Phase 1 plan](../plans/phase-1-masterless-cell.md), settled by the project owner on 2026-09-29: content comes from "a kind of internal artifactory managed by Nomos". [ADR 0012](0012-event-history.md) §1 and §5 for the Event Log. Milestones `09-file-and-directory` and `10-durable-cell`.

## Context

A file Condition names its content by digest; the Canon carries the digest, never the bytes ([ADR 0011](0011-canon-artifact-encoding.md)). Until now the Linux adapter held a content map filled by whoever constructed it, which only a test can do. An operator needs a way to hand a Cell the bytes a Canon names, and the Cell needs to trust them.

The transition kernel's snapshot is a fold of its Event Log ([event-log.md](../formal/event-log.md)), and the refresh Obligation of ADR 0009 §4 must outlive a crash between a configuration change and its restart. So far the log has been in memory. A Cell that forgets its Obligations on restart loses exactly the refresh the kernel was built to keep.

Both are state the Cell keeps on its own disk, written by Nomos and read by nothing else, and both need the same properties: a write is either whole or absent after a crash, and what is read back is what was written or a loud failure.

## Decision

### 1. One Adapter Crate for the Cell's Disk

`crates/adapters/nomos-store-fs` implements the ports below on the local file system, beneath one directory, `/var/lib/nomos` in production and a scratch directory in tests. It uses native system calls only, like the Linux Substrate, and depends on `nomos-core` and `nomos-store`.

### 2. The Content Store Is a Port

`nomos-store` gains a `ContentStore` trait: `get(digest)` returns the bytes whose SHA-256 digest is `digest`, or none; `put(bytes)` stores them and returns their digest. The store is addressed by content, so an entry never changes; a `put` of bytes already present is a no-op. Every `get` verifies the digest of what it read and treats a mismatch as absent, reported as corruption, never as content.

The Linux Substrate may depend only on core and its own port (ADR 0000), so `nomos-substrate` gains a narrower `ContentSource` trait, the bytes for a digest or none, and the composition root backs it with the store. An exact file requirement whose content the source does not have is refused before any effect, as today.

**Layout.** One file per blob, `content/sha256/<first two hex digits>/<64 hex digits>`, written through a temporary file in the same directory, synced, and renamed, then the directory synced (spec §11). Mode `0600`, owned by the Cell's user.

**Bundles.** Content reaches the store as a bundle beside the Canon artifact: a directory of files named by their digests, imported by `nomos-cell` before the artifact is enforced. Importing verifies each file's digest against its name and refuses the bundle on the first mismatch. In Phase 3, Loom serves the same store; the port does not change.

### 3. The Durable Log Is the Same Crate, and It Journals Inputs

`nomos-store-fs` implements `EventLog` over one append-only file of records. A record is one batch: a length, a check of the length (the first four bytes of its SHA-256 digest), the batch's encoding, and the SHA-256 digest of the length and the encoding, written with one `write` and one `fdatasync` before `append` returns. A batch is therefore whole or absent after a crash. The check of the length is what tells a torn tail from corruption: without it, a damaged length that points past the end of the file would make every whole record after it look like a record cut short, and recovery would truncate acknowledged batches.

**What is journaled.** The Cell's durable log holds the kernel's inputs, not its Events. `step` is a pure function (ADR 0006), so the Events of every Decision, and the snapshot they fold into, are recomputed by stepping the journaled inputs from the initial snapshot. Storing the Events themselves would mean decoding them, and an Event can carry a `Verified`, the proof that a postcondition held, which only core's `verify` may produce (`05-transition-kernel`'s compile-fail tests forbid building one anywhere else). A decoder outside core would have to forge it. A journaled Observation is instead verified again when it is replayed. The in-memory Event Log of the tests is unchanged, and the order of ADR 0012 §1 holds: the input is on disk before any effect of its Decision is issued.

**Encoding.** The input type is the application's, so `nomos-store` gains a `Record` trait, to bytes and back, which the application implements: deterministic CBOR in the Canon IR's profile, with a version field. An integer above the profile's largest, $2^{53}-1$, is written as its canonical decimal text, and decoding bounds nesting at 32 levels, so a hostile or damaged record fails to decode rather than exhausting the stack. The adapter stores bytes it does not interpret.

**Recovery.** Opening the log reads every record. A final record cut short, which a crash during a write leaves behind, is truncated away and reported: its batch was never acknowledged. A record is cut short only when its header is whole and its length checks, or when fewer bytes than a header remain. Any other malformed record, a length that does not check, a digest mismatch, or a length past the end in the middle of the file, fails the open loudly, and the Cell does not start: silently skipping a record would forget an Obligation. A record the application cannot decode fails the open the same way.

**Upgrades.** Recomputation uses the running kernel. A later Cell replays an earlier Cell's inputs under its own rules, so its snapshot is what it would have concluded from the same facts; an effect the earlier Cell issued under a key the later one would not produce is reported by its receipt as unknown, and `Recovered` treats every unsettled effect as unknown (ADR 0010).

**Full.** An append that cannot be written or synced returns `Full`, so the kernel drops the Decision whole and issues no effect (ADR 0012 §5).

### 4. What Is Not Decided Here

The storage engine for Loom's materialized state (the persistence ablation of `nomos-store`), retention and compaction of the log, and signing of bundles.

## Alternatives

- **The content inside the Canon artifact.** Rejected. ADR 0011 keeps the artifact inert and small, and the same bytes named by many Canons would be copied into each.
- **An embedded database for the log.** Not now. A single append-only file with checksums is enough for one Cell and has one crash behavior to test; the ablation between redb, SQLite, and LMDB stays open for Loom.
- **A content-addressed store keyed by path.** Rejected. A path does not say what the bytes are; a digest does, and the Canon already names content by digest.

## Acceptance Criteria

- `09-file-and-directory`: the Linux adapter's file suite passes with content from the store; a blob whose bytes were changed on disk is refused, not written; a bundle with a misnamed file is refused whole.
- `10-durable-cell`, experiment `durable-log`: a Cell killed at every step recovers the snapshot, every acknowledged Event and Obligation included, by replaying its journal; a torn final record is truncated and reported; a corrupted middle record fails the open.

## Note, 2026-09-30: The First Criterion

`09-file-and-directory` met the first acceptance criterion ([linux-files-and-directories](../plans/results/linux-files-and-directories.md)): the Linux file suite passes on Debian 12 and 13 with content from the store, a changed blob is refused, and a bundle with a bad entry is refused whole. The ADR stays Proposed until `10-durable-cell` meets the second.

## Note, 2026-09-30: The Second Criterion

`10-durable-cell` met the second acceptance criterion ([durable-log](../plans/results/durable-log.md)): a Cell killed after each of the 11 inputs of the refresh scenario, 8 of them with a refresh owed, recovered its snapshot and Obligations from the journal and converged; a torn final record was truncated and reported; a changed byte anywhere in a whole record failed the open. The Cell was killed in-process, not by power loss, which stays with Phase 2. The ADR is Accepted.

## Consequences

- The Cell has state on disk that must be backed up with, or rebuilt from, its Canon and bundles. Losing the log loses pending Obligations; losing the store makes exact file requirements refused until the bundle is imported again. Neither silently changes intent.
- `nomos-cell` gains an import step for bundles.
