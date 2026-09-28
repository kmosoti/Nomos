# Result: canonical-encoding

- **Experiment.** `canonical-encoding`, milestone `06-canon-artifact`.
- **Question.** Which restricted profile makes semantic equivalence and byte equality coincide on the IR: deterministic Concise Binary Object Representation (CBOR) per Request for Comments (RFC) 8949 §4.2, or the JSON Canonicalization Scheme (JCS) of RFC 8785?
- **Outcome.** Both do, on the restricted data model of [canon-ir.md](../../../formal/canon-ir.md). Deterministic CBOR is the simpler of the two, so the decision rule picks it. The recommendation goes to [ADR 0011](../../../adr/0011-canon-artifact-encoding.md), which stays Proposed until the owner accepts it.

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1; proptest 1.11 |
| Crate | `nomos-canon`, `no_std` with `alloc`, depends on `nomos-core` only; no runtime dependency for either profile or for the hash |
| Independent references | `ciborium` 0.2.2, with map keys sorted by their encodings as RFC 8949 §4.2.1 says; `serde_json` 1 with `serde_json_canonicalizer` 0.3.2; `sha2` 0.10 for `CanonID`. Dev-dependencies only |
| Harness | `crates/core/nomos-canon/tests/encoding.rs`; generators in `tests/support/mod.rs` |
| Fixtures | `tests/fixtures/canon/golden/`, the references' output ([README](../../../../tests/fixtures/canon/README.md)) |
| Receipts | `verification/receipts/2026-09-28-canon-artifact.ndjson` |

## Equivalence, Defined First

canon-ir.md was committed before the encoders. Two Canons are equivalent when they have the same name, the same resources by path with the same requirements, key sets, and node sets, and the same set of relations. Resource order, relation order, and duplicate set members mean nothing, and nothing else is normalized: paths are bytes, so a composed and a decomposed accent are two paths. The laws are $\mathrm{enc}_P(c_1) = \mathrm{enc}_P(c_2) \Leftrightarrow c_1 \equiv c_2$, and $\mathrm{dec}_P(b) = c \Rightarrow \mathrm{enc}_P(c) = b$.

## Evidence

| Check | Harness | Result |
| --- | --- | --- |
| Golden vectors | `golden_vectors_agree_with_the_independent_encoders`: five Canons (empty, a telemetry node, text with escapes, a control character, composed and decomposed accents, and a four-byte character, schema-1 files with relations, a loaded service), schema 2 in both profiles and schema 1 where it can say them | 16 vectors: the encoder under test and the reference produce the same bytes, the committed fixture is those bytes, the reference decoder reads the value back, and all 10 identities agree with `sha2` |
| Decoding | `every_golden_vector_decodes_to_its_canon` | Each golden vector decodes to its Canon in both profiles |
| Law 1, metamorphic | Seed `4e3601`, 512 cases: the same Canon written in rotated and reversed order with duplicated keys and relations | Passed: one encoding and one identity per profile |
| Law 2, the equivalence | Seed `4e3602`, 1,024 pairs of generated Canons | Passed: equal exactly when the encodings are and exactly when the identities are; 9 pairs were equal |
| Law 3, injectivity | Seed `4e3603`, 1,024 Canons with one to three bytes overwritten, both profiles | Passed: 19 mutated artifacts decoded, each the canonical encoding of what it decoded to; 2,029 rejected |
| Law 4, independent agreement | Seed `4e3604`, 512 generated Canons | Passed: bytes, value, and identity agree with the references |
| The hash | `sha256_agrees_with_sha2`, seed `4e3605`, 512 inputs of 0 to 300 bytes; the Federal Information Processing Standards (FIPS) 180-4 examples in the unit tests | Passed |
| Limits | `limits_hold_at_their_boundaries`: the largest integer, text, array, map, and nesting inside each limit, and the smallest outside, in both profiles, and an artifact one byte over its size limit | Passed: inside decodes, outside is `LimitExceeded` |
| No panic | Seed `4e3606`: 8,192 inputs, random bytes and truncated artifacts followed by random bytes, into both decoders and archival inspection | No panic |
| Two builds | `cargo xtask hermeticity` ([build-hermeticity](build-hermeticity.md)) | The generator's IR is identical across two isolated builds and equals the golden fixture, in both profiles |

## Negative Controls

- **A naive normalization.** `semantically_different_canons_keep_distinct_identities` takes ten pairs a careless normalization would merge: absent against present, any content against exact content, running against loaded, `requires` against `after`, `after` against `on_change`, the direction of a relation, no key against one key, a key against the same text as a node, path case, and composed against decomposed accents. Each keeps two encodings and two identities in both profiles. `SM-CANON-004`, which writes `after` as `requires`, is caught by it.
- **Adversarial CBOR**, each rejected at the stage that owns the rule: a non-shortest integer (`NonCanonical`), a trailing byte (`TrailingBytes`), a half-precision float and a tag (`OutsideModel`), a map in insertion order (`NonCanonical`), a duplicate key (`DuplicateKey`), resources out of path order and a key listed twice (`NonCanonical`, found only by re-encoding the validated Canon), and an uppercase digest (`InvalidDigest`). `SM-CANON-005`, which drops the final re-encoding check, is caught by it.
- **Adversarial JSON**: leading whitespace, whitespace after a comma, and an optional escape (`NonCanonical`); `2.0`, `2e0`, and `true` (`OutsideModel`); `02` and a byte order mark (`Malformed`); a duplicate member (`DuplicateKey`); members in insertion order (`NonCanonical`); and the whole artifact in uppercase (`MissingField`).
- **Domain separation.** `the_profiles_have_separate_identities` checks each identity against its own profile's tagged preimage, and against the untagged one and the other tag over the same bytes. Its first version compared the two profiles' identities only and **survived** `SM-CANON-006`, which drops the tag: the encodings differ, so the identities differed without it. The test was strengthened; the mutant was not changed.

## Measurements

| Measure | Deterministic CBOR | JCS |
| --- | --- | --- |
| Encoder and decoder, lines of code without comments and tests | 174 | 334 |
| Branch points (`if`, `match`, arms) | 33 | 83 |
| Runtime dependencies | 0 | 0 |
| Golden corpus, five Canons at schema 2 | 1,942 bytes | 2,448 bytes |
| Encode and decode per Canon, one release-build run of `measurements` on this machine, not recorded by a receipt | 5.8 µs and 13.4 µs | 5.4 µs and 13.6 µs |

Throughput decides nothing, and none of these is a threshold. The size of the code is the measure the decision rule names: JCS needs a number grammar with three forms outside the model, whitespace and escape handling, a sort by 16-bit Unicode Transformation Format (UTF) code units, and surrogate pairs, where CBOR needs a header per item and a sort by encoded key.

## Mutation

`cargo mutants -p nomos-canon`, over the whole crate at commit `e77575b`, the compile-fail test skipped for time: 479 mutants, 381 caught, 80 unviable, 7 timeouts, and 11 survivors. A first run, before the final tests existed, left 58 survivors; the tests they exposed were added before the recorded run: the limits at their boundaries and pinned to canon-ir.md's values, the JSON number grammar, uppercase and solidus escapes, empty containers, nested maps, the builder's shorthands, the path limit, and every error's text.

| Survivor | Class | Disposition |
| --- | --- | --- |
| Five `\|` to `^` in the CBOR header | Equivalent | The major type and the argument occupy disjoint bits |
| Three `^` to `\|` in the Secure Hash Algorithm (SHA) 256 choice and majority functions | Equivalent | The choice function's terms are disjoint, and the majority function is the same either way |
| `remaining` with `-` for `+` | Equivalent in result | The guard bounds an allocation before truncation is found; the error is the same |
| The escape loop's text check with `>` for `==` or `>=` | **Survived**: no test text had escapes | `limits_hold_at_their_boundaries` now decodes 4,096 and 4,097 bytes of escaped text; a re-run over `jcs.rs` at `0a1e3f2` caught both, with 146 caught, 12 unviable, 4 timeouts, and none missed |

The seven timeouts are loops made endless by a mutated index step; a hang fails the test run, so each is detected. The conversion from the decoded Canon to the kernel's, `cargo mutants -p nomos-app -f crates/app/nomos-app/src/kernel/artifact.rs --test-package nomos-cell`, gave 3 mutants: 1 caught, 2 unviable. `SM-CANON-007` covers the relation mapping, which the tool does not mutate.

## What the Rival Would Have Needed

The rival was that numbers, Unicode, map order, duplicate keys, or defaults break the equivalence in one of the profiles. None did, because the data model removes their sources: no floats, no negative numbers, no booleans or null, no defaults (every field is written), and map keys drawn from a fixed lower-case ASCII set. The member sort of JCS by UTF-16 code units is therefore never exercised on a non-ASCII key; the result holds for this model and says nothing about JCS over arbitrary JSON.

## Unchecked

- Artifacts beyond the generators' shapes: six paths, four labels, and up to eight relations, plus the five golden Canons. The limits are tested at their boundaries in both profiles by `limits_hold_at_their_boundaries`, not by generated artifacts near them.
- Timing side channels in the hash, which hashes public content only.
- Any decoder other than the two here and the two references.
- Coverage-guided fuzzing. The no-panic test draws random bytes and truncated artifacts; it does not search for inputs.
- Signing and the artifact container, which ADR 0011 leaves open.

## Decision

Deterministic CBOR, as the simpler profile that passes every semantic test. The file extension that follows from it is recorded when ADR 0011 is accepted.
