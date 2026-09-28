# Result: compatibility-matrix

- **Experiment.** `compatibility-matrix`, milestone `06-canon-artifact`.
- **Question.** Do explicit schema versions and migrations preserve accepted semantics across old and new readers?
- **Outcome.** Yes, for schema versions 1 and 2 and the file and service kinds. Every cell of a 13-artifact, 3-reader matrix did what canon-ir.md says, in both profiles. No accepted artifact changed intent, and every artifact a reader could not execute was refused before a Canon existed. At the composition boundary, an artifact with an unknown mutating kind reached no effect, and the rival reader that skips it was caught acting on the rest.

## Context

| Field | Value |
| --- | --- |
| Readers | `Reader::v1()`: schema 1, files. `Reader::current()`: schemas 1 and 2, files and services. A current-schema reader that knows files only, standing for a Cell without a capability |
| Artifacts | The golden fixtures in both schemas, and five from `tests/fixtures/canon/compat/`: an unknown kind (`sysctl`), an unknown field on a resource, an unknown state of a known kind, schema 3, and schema 1 carrying a schema-2 field |
| Harness | `crates/core/nomos-canon/tests/compatibility.rs`; `crates/bin/nomos-cell/tests/canon_artifact.rs` |
| Receipts | `verification/receipts/2026-09-28-canon-artifact.ndjson` |

## The Matrix

Each cell was stated in the test before it ran, from canon-ir.md. "Accept" means the decoded Canon equals the golden Canon the artifact was written from.

| Artifact | Schema-1 reader | Current reader | Files-only reader |
| --- | --- | --- | --- |
| `empty`, `text`, `legacy`, schema 1 | Accept, migrated | Accept, migrated | Accept, migrated |
| `empty`, `text`, `legacy`, schema 2 | `UnsupportedSchema` | Accept | Accept |
| `telemetry`, schema 2, with a service | `UnsupportedSchema` | Accept | `UnsupportedKind` at the service |
| `loaded`, schema 2, with a service | `UnsupportedSchema` | Accept | `UnsupportedKind` at the service |
| `unknown-kind` | `UnsupportedSchema` | `UnsupportedKind` | `UnsupportedKind` |
| `unknown-field` | `UnsupportedSchema` | `UnknownField` | `UnknownField` |
| `unknown-state` | `UnsupportedSchema` | `Invalid(UnknownRequirement)` | `Invalid(UnknownRequirement)` |
| `schema-3` | `UnsupportedSchema` | `UnsupportedSchema` | `UnsupportedSchema` |
| `v1-with-keys` | `UnknownField` | `UnknownField` | `UnknownField` |

78 cells over both profiles: 34 accepted as their golden Canon, 44 refused with the stated error, 0 otherwise.

## Evidence

| Check | Harness | Result |
| --- | --- | --- |
| The matrix | `every_reader_does_what_the_matrix_says` | 78 of 78 cells as stated |
| Lineage | Same test, on every migrated cell | The lineage names schema 1, the digest of the bytes read, and the `CanonID` of the schema-2 form; the migrated Canon re-encodes to the golden schema-2 fixture |
| Migration adds nothing | `migration_preserves_meaning_and_identity` | A migrated Canon equals its schema-2 reading and has its identity; no resource gains a key or a node, and the relation count is unchanged |
| Generated migrations | `generated_migrations_agree_with_direct_reading`, seed `4e3615`, 1,024 Canons, both profiles | 278 generated schema-1 artifacts migrated to the Canon they were written from, with its identity, and with no silent change |
| Archival inspection | `archival_inspection_reads_what_execution_refuses` | All 26 artifacts inspected, schema 3 and unknown kinds included, with the right schema, a name, and the digest of the bytes; `Inspection` cannot become a Canon (compile-fail `decoded-from-inspection`) |
| Before any effect | `an_unknown_mutating_kind_is_refused_before_any_effect` in `nomos-cell` | The Cell's intake refuses the refresh artifact with a `sysctl` resource added, so no Plan is made and nothing reaches the host. The test's strength is the rival below: the same bytes through a skipping intake do reach it |
| Decoded artifact converges | `a_decoded_artifact_converges_on_the_host` in `nomos-cell` | The refresh scenario, from artifact bytes in both profiles, is the kernel Canon the refresh experiments build by hand, and converges with its refresh consumed |

## Measurements

| Measure | Value |
| --- | --- |
| Silent semantic changes | 0: every accepted artifact is its golden Canon, and `silent_change` finds none on any accepted artifact |
| Correct rejections | 44 of 44 matrix cells, each with its stated error |
| Identity consistency across migration | 3 golden and 278 generated migrations, each with the schema-2 identity |

## Negative Controls

- **A reader that skips unknown kinds.** The rival drops resources of a kind it does not know "for forward compatibility" and decodes the rest. On the `unknown-kind` artifact it returns a Canon, and `silent_change`, which reads the artifact as a value and not through the decoder, reports that a resource the artifact names is missing. In the Cell, the same rival enforces the rest of the Canon and the mock host is changed. Semantic mutant `SM-CANON-002` puts this rival into the decoder and is caught by the matrix test.
- **A migration that changes a default.** `SM-CANON-003` gives every migrated file a conflict key schema 1 never stated. `migration_preserves_meaning_and_identity` catches it.
- **A conversion that changes a relation.** `SM-CANON-007` turns `on_change` into `after` between the decoded Canon and the kernel's. `a_decoded_artifact_converges_on_the_host` catches it.

## Unchecked

- Schema versions beyond 2 other than as refusals, and the capability versions of resource kinds that ADR 0011 §3 names; a reader here names kinds, not versions of them.
- Readers of different builds, across a network, or over time. The readers here are one binary configured three ways.
- An unknown kind that a future registry would allow (canon-ir.md, Known Gaps). This experiment establishes the refusal only.
- Keeping the original bytes. The lineage records their digest, and keeping them is the caller's.

## Decision

Versioned artifacts, readers that name what they support, strict refusal of the unknown, and migrations with lineage preserve accepted semantics for schemas 1 and 2. ADR 0011 §3 holds as implemented.
