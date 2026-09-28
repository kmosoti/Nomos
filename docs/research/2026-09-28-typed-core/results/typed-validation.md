# Result: typed-validation

- **Experiment.** `typed-validation`, milestone `06-canon-artifact`.
- **Question.** Do construction, decoding, and migration enforce the same domain invariants?
- **Outcome.** Yes. Every path to a Canon goes through one validator. On 2,048 generated malformed inputs no path accepted one; on 512 valid inputs every path accepted the same Canon; on 4,096 inputs of arbitrary field text no path disagreed with the validator. A `serde` derive over the same shape without the boundary accepted all 512 malformed inputs it was given, and the harness reported it.

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1; proptest 1.11; trybuild 1 |
| Validator | `impl TryFrom<RawCanon> for Canon` in `crates/core/nomos-canon/src/model.rs`, the only constructor of `Canon` |
| Paths | The validator; the typed builder `CanonBuilder`; the decoder in each profile; the schema-1 migration in each profile |
| Harness | `crates/core/nomos-canon/tests/validation.rs`; compile-fail cases in `tests/compile-fail/`; the generators and breaks in `tests/support/mod.rs` |
| Receipts | `verification/receipts/2026-09-28-canon-artifact.ndjson` |

## What Was Built

`RawCanon` is the untrusted data-transfer object: public fields, text everywhere, anything allowed. `Canon`, `Resource`, `Relation`, and `Name` have private fields, and `Canon::try_from(RawCanon)` is the only way to one. The builder writes a `RawCanon` from typed requirements and calls it; the decoder writes one from bytes and calls it; the migration writes one from a schema-1 value and calls it. A validated `Canon` is normalized by construction: resources are a map by path, and relations, keys, and nodes are sets. Errors are enums that name a position and never repeat the rejected text.

## Evidence

The oracle for malformed input is construction, not the validator: each malformed input is a valid generated Canon with one rule broken (`support::Break`), and the broken rule states the expected error.

| Check | Harness | Result |
| --- | --- | --- |
| Valid inputs | `every_path_accepts_a_valid_canon_as_the_same_canon`, seed `4e360b`, 512 Canons written in generated order with duplicates | Every path accepted every input it can say, as the validator's Canon: validator, builder, and both decoders 512 each; the migration in each profile 73, the inputs schema 1 can say |
| Malformed inputs | `no_path_accepts_a_malformed_canon`, seed `4e360c`, 2,048 inputs over twelve kinds of break: name, path, kind, state, missing and malformed digests, a digest where none belongs, label, duplicate path, dangling relation, self relation, relation kind, and cycle | No path accepted any input. Rejected with the predicted error: validator and both decoders 2,048 each, builder 1,195, migration 253 per profile. The builder could not say 853 of them and the migration 1,795, because their types have no place for the break |
| Arbitrary text | `arbitrary_fields_neither_panic_nor_split_the_paths`, seed `4e360d`, 4,096 Canons with every field drawn from a pool of 20 texts, valid and not | No panic, and no path disagreed with the validator; 218 were valid and accepted as one Canon by every path that can say them |
| The absent-with-contents countercase | `an_absent_file_with_contents_is_refused_everywhere` | Every path refuses a file that must be absent and names a digest; the builder cannot write it, and `nomos-core`'s compile-fail case `absent-with-content` shows why |
| Errors are secret-free | `errors_never_repeat_the_input`: a sentinel written into the name, a path, the kind, the state, the digest, a key, a node, a relation kind, and a relation end | The sentinel appears in none of the 27 errors, from the validator and both decoders, in `Debug` or `Display` |
| Type boundary | `a_canon_cannot_be_built_around_the_validator`, six `trybuild` cases | Each fails to compile with the intended diagnostic: a `Canon`, `Resource`, `Relation`, or `Name` written as a literal; a Canon changed through its accessors; an archival `Inspection` turned into a Canon |
| One validator, at the kernel | `authoring_and_decoding_reach_the_same_kernel_canon` in `nomos-cell` | The builder's Canon and the decoded artifact's are one kernel Canon, and their encodings are the same bytes |

## Measurements

| Measure | Value |
| --- | --- |
| Rejection parity | 2,048 of 2,048 malformed inputs rejected by every path that can say them, each with the error the break predicts. The decoders reject an unknown kind at the schema stage (`UnsupportedKind`) where the validator says `UnknownRequirement`; both name the same resource |
| Panic count | 0, over the 6,656 generated inputs here and the 8,192 decoder inputs of [canonical-encoding](canonical-encoding.md) |
| Smallest counterexample | None found |

## Negative Controls

- **A derive without the boundary.** A `serde`-derived struct of the same shape, public fields and no `try_from`, run through the same harness from JSON artifacts, accepted 512 of 512 malformed inputs; the decoder under the same harness accepted 0. `the_harness_catches_a_derive_without_the_boundary` fails if the derive's count is zero.
- **A validator that skips a check.** Semantic mutant `SM-CANON-001` drops the check that a relation's ends name resources. `no_path_accepts_a_malformed_canon` catches it.

## Unchecked

- Inputs beyond the generators: six paths, four labels, names from a fixed set, and the break catalogue. A rule nobody wrote a break for is not tested by this harness, though the arbitrary-text test samples outside the catalogue.
- The path rules themselves, which are `nomos-core`'s `ResourcePath` and tested there.
- What a caller does with a `RawCanon` it never validates. Nothing stops code from holding one, and nothing that executes accepts one.
- Deserialization through `serde`: the crate derives none. A future derive must go through `try_from`, which this experiment's negative control would catch only if it were run against it.

## Decision

The validated boundary holds for the Canon IR: construction, decoding, and migration enforce one set of invariants through one function, and the type system closes the ways around it that the compile-fail cases name.
