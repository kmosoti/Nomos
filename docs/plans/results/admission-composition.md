# Result: admission-composition

- **Experiment.** `admission-composition`, milestone `08-resource-families` of the [Phase 1 plan](../phase-1-masterless-cell.md).
- **Question.** Does admission reject every overlapping written property, and only those?
- **Outcome.** Yes, over every pair of resources from every pair of families, with names chosen so that one word appears in every family and one path in every path family. The validator and the decoder in both profiles agree with the property table of [resource-families.md](../../formal/resource-families.md#admission) on all 196 pairs; an admission that checks keys only would have admitted 12 of them.

## Context

| Field | Value |
| --- | --- |
| Commit | `6b0d1e4`, where every receipt was recorded |
| Oracle | `writes`, the table of resource-families.md restated in the test: `path:<path>` for a file, a directory, or the legacy service, and `<family>:<name>` for every other family. It is not the validator's `Property::written` |
| Harness | `crates/core/nomos-canon/tests/families.rs` |
| Receipts | `verification/receipts/2026-09-30-resource-families.ndjson` |

## The Pairs

Seven families, two names each, as a first and a second resource: $7 \times 7 \times 2 \times 2 = 196$ pairs. The names are `/etc/nginx` and `/etc/other` for the three path families, and `nginx` and `other` in each other family's form: `nginx.service`, `net.nginx`, a user `nginx`, and a package `nginx`.

| Expected | Pairs | Rule |
| --- | --- | --- |
| `DuplicatePath` at resource 1 | 14 | One key twice |
| `Conflict` at resource 1 | 12 | Two keys, one written property: a file, a directory, and a service at one path |
| Admitted | 170 | Everything else, including a user and a package both named `nginx` |

Each pair runs through `Canon::try_from` and through `decode` of its schema-3 artifact in deterministic CBOR and in the JSON Canonicalization Scheme, and all three agree with the table.

## The Negative Control

`the_harness_catches_an_admission_that_checks_keys_only` evaluates the rival over the same pairs, a map keyed by resource and nothing more, and finds the 12 pairs it admits and the table does not. `SM-ADMIT-001` makes the validator that rival, and `admission_rejects_exactly_the_overlapping_properties` catches it.

## What This Does Not Establish

- Composition across Canons. The footprint check between controllers is `07`'s `controller-composition`, still outside admission (ADR 0008 §1 stays Proposed).
- Properties a resource writes that its key does not name, such as the files a package installs or the unit files a package ships. The table is the family's own property only.

## Decision Fed

A Canon is admitted only when no two of its resources write one property. `09` to `13` add no family without a row in the table and a pair in this test.
