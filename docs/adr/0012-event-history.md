# ADR 0012: Event History, Durability, and Retention

- **Status.** Proposed
- **Date.** 2026-09-28
- **Candidate.** `event-history` in the 2026-09-28 research snapshot (recommendations `log-boundary`, `log-retention`)

## Context

A crash between an operating-system effect and its Event leaves started work with no record. Replaying the Event Log restores what the Cell knew, not what the host is (finding `replay-not-reality`). "Append-only" says nothing about disk space. The Event Log is Phase 2 work, but the kernel of milestone 1 must emit Events in an order that this design can make durable. Research open question `retention` ends here.

## Decision

Proposed. [event-log.md](../formal/event-log.md) follows it as a working definition until it is accepted.

### 1. Intent Before Effect

The decision Event and the dispatch intent are appended in one transaction before the effect starts, and the terminal Event follows it. A local transaction cannot contain a Linux mutation, so recovery treats an intent without a terminal Event as an unknown outcome (N10).

### 2. Replay Restores Control, Observation Restores Truth

Recovery replays the log into control state, then observes the host before planning anything.

### 3. Acknowledgment Means Committed

An acknowledgment is sent only after the append is committed under the Store port's durability contract. Loom acknowledges only a contiguous prefix of a writer's sequence. The same writer coordinate carrying a different payload is an integrity fault, never a duplicate.

### 4. Append-Only Is Logical

Events are immutable and the history is append-only. Projections and acknowledgment cursors are mutable. No deletion or compaction is approved by this ADR; retention and archival ownership are decided with the storage adapter.

### 5. A Full Disk Stops Mutation

When the log cannot accept an append, mutation admission fails closed, because an Action whose Event cannot be written is an Action whose outcome would be lost. Observation and Trace continue.

### Acceptance Criteria

- **Milestone 1 PR 4.** The logical ordering of intent, dispatch, completion, and acknowledgment is modeled against a fault-injectable in-memory store.
- **Phase 2, `event-crash-replay`.** Kill the Cell at each persistence and effect boundary, replay duplicates and gaps, send a conflicting payload at an existing coordinate, and exhaust the spool. No acknowledged Event may be lost and no false success reported.

## Consequences

- The Store port gains an explicit durability contract before it gains an adapter.
- A hash chain over the log remains a later, optional mode. Without externally retained checkpoints it does not expose a rewritten prefix.
- **Revisit trigger.** Reopen with the persistence ablation of spec §54, or if physical retention pressure forces a deletion policy.
