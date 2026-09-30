# Result: canon-schema-3

- **Record of.** The Canon IR's schema 3 and its migrations, milestone `08-resource-families` of the [Phase 1 plan](../phase-1-masterless-cell.md), whose exit asks that artifacts of the previous schema decode and migrate. The experiments are `06`'s, `canonical-encoding` and `compatibility-matrix`, extended; their `06` records stand.
- **Outcome.** Every `06` fixture decodes as before, and its bytes are unchanged: the schema-1 and schema-2 golden vectors, the compatibility fixtures, and the schema-2 identities in `ids.txt` are reproduced byte for byte by the new code. Schema-1 and schema-2 artifacts migrate to schema 3 with lineage. Seven Canons in schema 3 agree with the independent encoders, and every cell of a 24-artifact, 5-reader matrix does what [canon-ir.md](../../formal/canon-ir.md) says, in both profiles.

## Context

| Field | Value |
| --- | --- |
| Commit | `6b0d1e4`, where every receipt was recorded |
| Oracles | `ciborium` and `serde_json_canonicalizer` for the encodings, `sha2` for the identities, and the matrix stated in the test before it runs |
| Harness | `crates/core/nomos-canon/tests/encoding.rs`, `compatibility.rs`, `families.rs`; `crates/bin/nomos-cell/tests/canon_artifact.rs` |
| Fixtures | `tests/fixtures/canon/`, whose README lists what `08` added |
| Receipts | `verification/receipts/2026-09-30-resource-families.ndjson` |

## Encodings

`golden_vectors_agree_with_the_independent_encoders` checks 30 vectors: seven Canons at schema 3 in two profiles, the five of `06` at schema 2, and three at schema 1. The two new Canons are `families`, every family with every optional field stated somewhere and left out somewhere, with relations across families, and `text3`, text in a path, an account name, and a home directory. Schema-3 identities are in `ids.v3.txt`; `ids.txt` still holds the schema-2 identities `06` recorded, now computed by `canon_id_v2`.

`generated_schema3_canons_round_trip` decodes 512 generated schema-3 Canons from their own encodings in both profiles and refuses each with its resources reversed as non-canonical. `generated_migrations_agree_with_direct_reading` migrates 278 generated schema-1 artifacts, and the schema-2 form of every generated Canon, to the Canon read directly, with lineage to its schema-3 identity.

## The Matrix

Five readers: schema 1 with files; `06`'s current reader, schemas 1 and 2 with files and services; the current reader, schemas 1 to 3 with every kind; schemas 1 and 2 with files only; and a host reader, every schema and every kind but the legacy service.

| Artifacts | Result |
| --- | --- |
| The `06` schema-1 and schema-2 golden vectors | As `06` recorded for the old readers; the current and host readers accept and migrate, with lineage |
| The five `06` compatibility fixtures | As `06` recorded, except `schema-3`, a schema-2 body under the number 3, which the current and host readers now refuse as `UnknownField` on its resource |
| The seven schema-3 golden vectors | Accepted by the current reader; the host reader refuses the three with a service, at the service |
| `schema-4`, `v3-unknown-kind`, `v3-unknown-spec-field`, `v3-wrong-type` | `UnsupportedSchema`; `UnsupportedKind`; `UnknownField` on the spec; `WrongType` on the spec |

240 cells in all: 84 accepted as their golden Canon and 156 refused as stated. At the composition boundary, a schema-3 artifact with an unknown family reaches no effect, and the rival reader that skips it is seen acting on the rest, as `06` showed for schema 2.

## What Changed in the Oracle

- `06`'s adversarial encoding tests and the typed-validation paths now name schema 2 explicitly (`encode_v2`, `raw_value_v2`), since `encode` writes schema 3.
- The typed-validation oracle for a duplicate path whose first resource is a service accepts `Conflict` beside `DuplicatePath`: the builder writes schema 3, where a file and a service at one path are two keys that write one property.
- `SM-CANON-001`, `002`, and `005` were regenerated against the new code with the same wrong behavior; `SM-CANON-008` and `009` are new, for schema 3's unknown kind and dangling relation.
- The `telemetry` authoring crate's IR is compared with `telemetry.v3.*`, and its source uses the new file requirement's constructor. `cargo xtask hermeticity` passed at `6b0d1e4`: two isolated builds give identical IR, equal to `telemetry.v3.*` in both profiles.
- `generator-variance`, whose candidates target the file-only core, now runs in a worktree at `576d50f`, the commit of its record. Re-run at `6b0d1e4`, its output equals the committed record except for the order of names within the failed-test lists, which follows the order in which parallel tests finish.

## What This Does Not Establish

- A schema 3 from another writer. Every artifact here was written by this code or its independent encoders.
- Migration of a schema-2 artifact whose file requirement a host cannot serve; that is the reader's refusal, per kind, not the migration's.
