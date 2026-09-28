# ADR 0011: Canon Artifact Encoding, Identity, and Versioning

- **Status.** Proposed
- **Date.** 2026-09-28
- **Candidate.** `canon-artifact` in the 2026-09-28 research snapshot (recommendations `canonical-profile`, `compatibility`, `validated-boundary`). The authoring surface and the inert-artifact boundary are accepted in [ADR 0004](0004-rust-typed-canon.md).

## Context

ADR 0004 fixed where Rust runs and what Loom and Cell accept. It deliberately left the bytes open. A deterministic encoder is not a content-addressing scheme (finding `canonical-not-wire`). A closed Rust enum and an extensible wire format are different promises (finding `enum-wire`). And a validated type is only as strong as its decoder (finding `decode-validation`). Research open questions `encoding` and `extensions` end here.

## Decision

Proposed. Nothing on this page is chosen until the experiments below have run.

### 1. Identity Covers Meaning, Not Provenance

`CanonID` hashes a domain tag, the IR schema version, and the normalized IR, and nothing else. The build's provenance record sits beside the artifact and outside the hash. Semantically equal Canons have equal IDs, and semantically different Canons never share one in the test corpus.

### 2. The Encoding Is the Simpler Profile That Passes

The candidates are a restricted deterministic profile of Concise Binary Object Representation (CBOR), as Request for Comments (RFC) 8949 §4.2 defines it, and the JSON Canonicalization Scheme (JCS) of RFC 8785. The equivalences are defined first: field order, defaults, integer ranges, path bytes, and no floating point. The chosen profile is the simpler one that passes every semantic test. Throughput decides nothing. Rust memory layout and transport encodings are never hashed.

### 3. Versions Are Separate From Rust APIs

The IR has its own schema version, and resource capabilities have their own versions. A consumer rejects an unknown executable variant before any effect. Unknown data may be preserved only on paths that never execute it. A migration produces a new `CanonID` and records the lineage.

### Still Open After This ADR

- The hash algorithm itself.
- Artifact signing, and what a consumer verifies.
- The extensibility model, beyond §3's rule for unknown variants.

### Acceptance Criteria

Milestone `06-canon-artifact`:

- `canonical-encoding` passes golden and adversarial vectors across two builds and an independent decoder.
- `compatibility-matrix` passes old and new readers, unknown variants, and migrations.
- `typed-validation` shows that no malformed input becomes a validated Canon by any path.
- `build-hermeticity` shows identical declared inputs produce identical IR bytes, with every undeclared input denied or recorded.

## Note, 2026-09-28: Working Definitions for the Canon Artifact

Milestone `06-canon-artifact` implements this ADR against the working definitions in [canon-ir.md](../formal/canon-ir.md), written before the code: the restricted data model both profiles carry, schema versions 1 and 2, semantic equivalence and normalization, strict decoding in four stages with the canonical-form check, readers that name their schema versions and resource kinds, archival inspection that returns no Canon, migration with a lineage record, and `CanonID` over a profile tag, the schema version, and the canonical bytes. The hash is SHA-256, from the Secure Hash Algorithm (SHA) 2 family, as a working definition, implemented in `nomos-canon` because the audited `sha2` crate reaches `libc`, which the core purity policy denies. §2's choice between the two profiles is left to experiment `canonical-encoding`, which runs the same semantic tests against both.

## Consequences

- The artifact file extension follows the chosen encoding and is recorded when this ADR is accepted.
- **Revisit trigger.** Reopen if neither profile passes the equivalence tests, or if signing requires a container the chosen encoding cannot carry.
