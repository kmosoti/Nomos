# Canon IR

What the Canonical IR is, when two Canons are the same, how an artifact is encoded and decoded, and how a Canon gets its identity. [ADR 0004](../adr/0004-rust-typed-canon.md) fixes the boundary: Rust runs once in the author's build, and Loom and Cell accept only the IR. [ADR 0011](../adr/0011-canon-artifact-encoding.md), accepted 2026-09-29, decides the identity and versioning rules and the encoding. This document gives the working definitions milestone `06-canon-artifact` implements and tests; the choices it leaves to an experiment say so.

## Scope

The IR carries what the kernel reads (ADR 0006 note): for each managed resource, its path, its requirement, its conflict keys, and the nodes its Action would disrupt; and the relations between resources. The requirement is a sum type with two resource kinds, the ones the kernel can plan today:

| Kind | Requirement | Meaning |
| --- | --- | --- |
| `file` | `absent` | No file at the path |
| `file` | `present-any` | A file with any content |
| `file` | `present-exact` with a digest | A file with exactly this content |
| `service` | `running` | The service runs; it is observed at its status path |
| `service` | `loaded` with a digest | The service runs the configuration with this digest; it is observed at the path where it reports the revision it loaded |

A file that is absent and carries a digest, or a service that is absent, has no representation. The other resource kinds of spec §10 join the IR with their families.

## The Data Model

Both candidate encodings carry the same restricted data model:

- unsigned integers up to $2^{53} - 1$;
- text strings in the 8-bit Unicode Transformation Format (UTF), UTF-8, at most 4,096 bytes;
- arrays of at most 65,536 items;
- maps whose keys are text strings, each key at most once, with at most 64 entries;
- nesting at most 8 levels deep;
- an artifact of at most 16 mebibytes (MiB), refused before parsing.

No floating point, no negative integers, no byte strings, no booleans, no null, no tags, no indefinite lengths. A digest is text: 64 lowercase hexadecimal characters. The integer bound is the largest range both encodings represent exactly; the IR uses integers only for the schema version.

## Schema Version 3

The current IR, from milestone `08-resource-families`: every family of [resource-families.md](resource-families.md). A map with exactly these keys:

| Key | Value |
| --- | --- |
| `schema` | `3` |
| `name` | The Canon's name, as in version 2 |
| `resources` | An array of resource maps, sorted by `kind`, then `name`, no key twice |
| `relations` | An array of relation maps, sorted by source, then target, then kind, no relation twice |

A resource map has exactly `kind`, `name`, `spec`, `keys`, and `disrupts`. `kind` is one of `file`, `directory`, `service`, `unit`, `sysctl`, `user`, and `package`; `name` is valid for the family; `keys` and `disrupts` are as in version 2. `spec` is a map whose fields depend on the kind, and an optional field is present exactly when the requirement states it, so a requirement has one encoding:

| Kind | `spec` fields |
| --- | --- |
| `file` | `state`, `absent` or `present`; when present, `content`, `any` or a digest, and optionally `owner`, `group`, `mode` |
| `directory` | `state`, `absent` or `present`; when present, optionally `owner`, `group`, `mode` |
| `service` | `state`, `running` or `loaded`; `digest` when loaded |
| `unit` | `active`, one of `active`, `inactive`, `any`; `enabled`, one of `enabled`, `disabled`, `any` |
| `sysctl` | `value`, normalized: tokens joined by one space, no leading or trailing space, not empty |
| `user` | `state`, `absent` or `present`; when present, `class`, `system` or `regular`, and optionally `home`, `shell`, each an absolute path |
| `package` | `state`, `absent` or `installed`; when installed, optionally `version` |

`owner` and `group` are account names as the `user` family validates them. `mode` is exactly four octal digits, at most `7777`. A relation's `source` and `target` are resource keys in their text form, `<kind>:<name>`. Two resources that write one property are rejected by the validator ([resource-families.md](resource-families.md#admission)).

## Schema Version 2

The IR of `06-canon-artifact`, now read and migrated. A map with exactly these keys:

| Key | Value |
| --- | --- |
| `schema` | `2` |
| `name` | The Canon's name: 1 to 64 characters from `a` to `z`, `0` to `9`, and `-`, starting with a letter |
| `resources` | An array of resource maps, sorted by path, no path twice |
| `relations` | An array of relation maps, sorted by source, then target, then kind, no relation twice |

A resource map has exactly `path`, `spec`, `keys`, and `disrupts`. `path` is a resource path as `nomos-core` validates it, at most 4,096 bytes. `keys` and `disrupts` are arrays of names, sorted, no name twice: 1 to 128 characters from `a` to `z`, `0` to `9`, and `.`, `_`, `:`, `/`, `-`. `spec` is a map with exactly `kind` and `state`, and `digest` when the state is `present-exact` or `loaded`.

A relation map has exactly `source`, `target`, and `kind`, where `kind` is `requires`, `after`, or `on_change`, and both ends name resources of the Canon. A relation from a resource to itself, and a cycle through relations of any kind, are invalid ([warp.md](warp.md)).

## Schema Version 1

The first IR, file resources only, kept so that migration can be tested. Exactly `schema` = `1`, `name`, `files`, and `relations`. A file map has exactly `path`, `state`, and `digest` when the state is `present-exact`. Version 1 has no conflict keys and no disruption.

## Semantic Equivalence

Two Canons are equivalent when they have the same name, the same set of resources, each with the same requirement, the same set of conflict keys, and the same set of disrupted nodes, and the same set of relations. Order is not meaning: resources, relations, keys, and nodes are sets. Everything else is meaning: a path differs from another path byte for byte, with no case folding and no Unicode normalization; `after` is not `requires`; `present-any` is not `present-exact`; an empty key set is not an absent one, because there is no absent one.

**Normalization** produces the one representative of an equivalence class: every set sorted and without duplicates, every field present. The validated `Canon` type holds only normalized values, so encoding it needs no further step.

**The equivalence the encoding must preserve.** For Canons $c_1$ and $c_2$ and a profile $P$:

$$
c_1 \equiv c_2 \iff \mathrm{enc}_P(c_1) = \mathrm{enc}_P(c_2)
$$

and, for decoding, $\mathrm{dec}_P(b) = c \Rightarrow \mathrm{enc}_P(c) = b$. The second says that a Canon has one encoding: bytes that decode but are not the canonical encoding of what they decode to are rejected. Without it, two artifacts with different bytes could carry the same executable intent under different identities, or a non-canonical artifact could hide a difference behind a lenient parser.

## The Two Candidate Profiles

**Deterministic Concise Binary Object Representation (CBOR).** The core deterministic encoding requirements of Request for Comments (RFC) 8949 §4.2.1: integers and lengths in their shortest form, definite lengths only, and map keys sorted by the bytewise lexicographic order of their encodings, which for text keys means shorter keys first.

**JSON Canonicalization Scheme (JCS).** The scheme of RFC 8785 over the data model: no whitespace, object members sorted by the UTF-16 code units of their names, strings with only the escapes RFC 8785 requires, and integers in their shortest decimal form.

Experiment `canonical-encoding` implemented both, ran the same semantic tests against both, and found that both pass; by ADR 0011 §2's rule the simpler is chosen. **The artifact encoding is deterministic CBOR**, in `.cbor` files, and `CanonID` is taken under the CBOR tag. JCS stays implemented as the comparison and is not an artifact encoding.

## Strict Decoding

A consumer decodes in four stages, and any failure rejects the artifact before a Canon exists:

1. **Syntax.** The bytes parse under the profile into the data model. Anything outside it fails: a float, a tag, a negative integer, an indefinite length, invalid UTF-8, a duplicate map key, a limit exceeded, bytes after the value.
2. **Canonical form.** Re-encoding the parsed value reproduces the input exactly.
3. **Schema.** The schema version is one the reader supports; every map has exactly its fields; every `kind` is one the reader supports. An unknown field or kind is rejected, never skipped: skipping a resource a newer writer meant would silently change executable intent.
4. **Validation.** The untrusted data-transfer object is converted into the validated `Canon` by the one validator the authoring API also uses. The validator rejects a `state` its kind does not have, so the table of requirements exists once. The validated `Canon`, encoded again in the schema the artifact was written in, must reproduce the input, or the artifact is rejected as non-canonical. Stage 2 cannot see this: resources out of path order, or a key listed twice, are canonical as values, and the validator collapses a duplicate into one set member.

**Readers.** A reader names the schema versions and resource kinds it supports. The current reader supports versions 1, 2, and 3 and every kind; a host reader supports only the kinds its adapter serves, so the Linux Cell refuses a Canon with a `service`; the version 1 reader, which the compatibility tests use as an old reader, supports version 1 and files only.

**Archival inspection.** A separate function reports the schema version, the name, and the digest of any artifact that passes stages 1 and 2, whatever its version or kinds. It returns no `Canon`, so nothing it accepts can reach the kernel. Preserving an artifact never implies permission to execute it.

## Migration

A version 1 artifact read by the current reader is migrated to version 2: each file becomes a `file` resource with no conflict keys and no disruption, and relations are unchanged. A version 2 artifact is migrated to version 3: a `file` resource keeps its path as its name, `present-any` becomes `present` with `content` `any`, `present-exact` becomes `present` with the digest, a `service` keeps its state, and a relation's ends become the keys of the resources at those paths. A version 1 artifact migrates through version 2. The migrated Canon has its own `CanonID`, and the migration returns a lineage record: the version 1 artifact's digest and schema version, and the version 2 `CanonID`. The original bytes are the caller's to keep. A migration never adds a requirement, a key, a node, or a relation that the original did not state.

## Identity

$$
\mathit{CanonID} = \mathrm{SHA256}(\mathit{tag}_P \mathbin{\Vert} \texttt{0x00} \mathbin{\Vert} \mathrm{be}_{64}(v) \mathbin{\Vert} \mathrm{enc}_P(c))
$$

$\mathit{tag}_P$ is the ASCII text `nomos.canon-id` followed by `.cbor` or `.jcs`, so that the two profiles never share an identity; $v$ is the schema version as a 64-bit big-endian integer; $\mathrm{enc}_P(c)$ is the canonical encoding of the normalized Canon. Provenance is not hashed (ADR 0011 §1). SHA-256, from the Secure Hash Algorithm (SHA) 2 family, is a working definition: ADR 0011 leaves the hash algorithm open. It is implemented in `nomos-canon` without a dependency, because the audited `sha2` crate reaches `libc` through `cpufeatures`, which the core purity policy denies, and it is checked against the examples of Federal Information Processing Standards (FIPS) 180-4 and, differentially, against `sha2` as a test-only dependency.

## Provenance

The build job writes a provenance record beside the IR (ADR 0004 §3): the Rust toolchain, the lockfile digest, the `nomos-canon` version, the digests of the declared inputs, and the digest of the IR. Experiment `build-hermeticity` checks that two builds with the same declared inputs produce the same IR bytes and that every undeclared input the generator could reach is denied or recorded.

## Known Gaps

- **Signing.** Open; ADR 0011.
- **Extensibility beyond the version rule.** A resource kind a reader does not know is rejected; how a future kind is registered is open.
- **The hash algorithm.** SHA-256 is a working definition; ADR 0011 leaves the algorithm open.
- **Conditions beyond the file family and the service refresh.** They arrive with their resource families.
