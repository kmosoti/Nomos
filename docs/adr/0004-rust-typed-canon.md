# ADR 0004: Canon Is Authored in Rust and Shipped as an Inert Canonical IR

- **Status.** Accepted
- **Date.** 2026-09-28

## Context

The specification described Canon as strict YAML compiled to a typed intermediate representation (IR), with the YAML-against-TOML choice left to an ablation (spec §5, §54). It also drew a line: Canon does not embed a general-purpose programming language, because restricted expression forms in configuration files have a way of growing into one.

The 2026-09-28 research snapshot argued for a different authoring surface: a Rust crate that builds a typed `Canon` value and emits an inert artifact, with Loom and Cell accepting only the artifact ([evaluation](../research/2026-09-28-typed-core/README.md), recommendation `typed-canon`). The snapshot also named the cost. Rust authoring is code execution, so the build that produces the artifact needs isolation, bounded inputs, and no host credentials, and typed source does not by itself make the output reproducible (finding `build-not-pure`).

The [grounding plan](../research/2026-09-28-typed-core/grounding-plan.md) recorded this as decision D1 and left it to the project owner. On 2026-09-28 the owner chose Rust.

## Decision

This ADR and [ADR 0005](0005-assessment-vocabulary.md) together record six decisions:

| # | Decision | Where |
| --- | --- | --- |
| 1 | Canon is authored with typed Rust models | §1 below |
| 2 | Runtime Nomos consumes inert, normalized Canon artifacts | §2 and §4 below |
| 3 | Conditions express requirements | ADR 0005 |
| 4 | Observations represent evidence | ADR 0005 |
| 5 | Assessment distinguishes Satisfied, Variance, and Indeterminate | ADR 0005 |
| 6 | Executing Rust source is outside Loom and Cell runtime semantics | §2 and §3 below |

### 1. Rust Is the Authoring Surface

A Canon is a Rust crate that depends on `nomos-canon`, constructs a `Canon` value from typed resource specifications, and emits the Canonical IR. The type checker, exhaustive matching, and ordinary Rust tests are what an author uses to keep a Canon correct. There is no second authoring language in v0. The form of the authoring API, builders or a macro layer, is not decided here.

The compilation stages are fixed: validated domain construction, semantic validation, normalization, the inert Canonical IR, and the Canon artifact that carries it (spec §6).

### 2. Hosts See Only the IR

Loom and Cell accept the Canon artifact and nothing else. They never compile Rust, load a native plugin, or evaluate source. `nomos-loom compile` validates an IR and reports its `CanonID`; it does not run `cargo`. The property the YAML design protected, that no programming language runs on a managed host, holds because the language runs once, elsewhere, and what reaches the host is data.

### 3. The Build Job Is Treated as Code Execution

The authoring crate runs in an isolated, bounded job: no network, no host-management credentials, and declared inputs only. Beside the IR the job writes a provenance record with the toolchain, the lockfile digest, the `nomos-canon` version, the digests of declared inputs, and the digest of the IR. Provenance stays outside `CanonID`, which is computed from the normalized IR alone (spec §6). Two builds with identical declared inputs must produce identical IR bytes. That is tested, not assumed: grounding experiment `build-hermeticity` varies the clock, locale, environment, working directory, and network reachability and records which undeclared inputs are denied. Where the job runs, in a CI runner or a container, is deployment. What Nomos checks is the contract.

### 4. The IR Is Validated Data With Algebraic Types

Resource specifications are sum types that cannot express contradictions, for example a file that is `Absent` cannot carry required contents. The IR is validated when the author emits it and again when Loom or Cell decodes it: decoding produces untrusted data-transfer objects, and a fallible conversion into private-field domain types runs the same validator the authoring API uses. If artifacts are ever signed, a signed IR still goes through that conversion. Whether resource kinds are a closed set is not decided here; whichever model is chosen, an IR whose executable semantics the consumer does not know is rejected before any effect.

### 5. Explicitly Unresolved

None of these is decided by this ADR, and none may be treated as decided until its own record exists:

- **Canonical artifact encoding.** A restricted deterministic profile of Concise Binary Object Representation (CBOR) or the JSON Canonicalization Scheme (JCS). Milestone 1 pull request 5, experiment `canonical-encoding`.
- **Artifact container and file extension.** Examples carry no extension.
- **Content hash algorithm and profile.** What $H$ is, and what domain separation and version tag it hashes over.
- **Artifact signing.** Whether artifacts are signed, by whom, and what a consumer verifies.
- **Extensibility model.** Whether resource kinds are closed or extensible, and how an unknown kind is preserved without being executed.
- **Logical Condition composition.** Whether Conditions compose beyond a set, and what a repair means for a composite.
- **Trait-dependent expressions.** How a Condition may depend on a Trait value. One constraint already binds any answer, because it follows from §2: what reaches the host is data, not a closure or a template, and its evaluation is bounded.
- **Schema versioning and migration rules.** Experiment `compatibility-matrix`.

[ADR 0011](0011-canon-artifact-encoding.md), proposed, carries the encoding, identity, and versioning decisions on top of this one.

### Assumptions

- Authors can run `cargo` in an isolated job. Managed hosts cannot be required to have `cargo` at all.
- The hermeticity contract in §3 can be enforced, or at least every undeclared input can be detected. If it cannot, see the revisit trigger.
- The six initial resources (spec §10) fit a typed algebraic model, whatever the extensibility model turns out to be.

### Alternatives

- **Strict YAML or TOML with a typed IR** (the previous design). Rejected. The restricted expression forms it needs grow into an undocumented language, the author gets no type checker, and the format choice was already a coin toss the ablation existed to settle.
- **A purpose-built declarative language.** Rejected. A new language is a parser, a type system, and an editor story to maintain, for a smaller feature set than Rust already provides.
- **Shipping Rust source or compiled plugins to hosts.** Rejected. It puts code execution on the managed host, which is the one place the design keeps free of it.
- **A Nix-style content-addressed store.** Not needed. Spec §6 already takes the useful part, content-derived identity, without the store.

### Evidence

Research findings `build-not-pure`, `decode-validation`, `sum-not-product`, and `canonical-not-wire` are documented mechanisms from primary sources and inspectable arguments. None has been exercised on Nomos code. Milestone 1 pull request 5 supplies the executable evidence, and the verification section below says what is owed.

## Consequences

- Spec §5, §6, §7, §34, §37, §51, §54, and §61 are amended in the same change as this ADR. The vocabulary of spec §3 is unchanged: the inert artifact *is* the Canonical IR that §6 already named.
- `nomos-canon` becomes the typed authoring API plus the IR encoder, decoder, and validator. It stays in `crates/core/` and performs no I/O; an authoring crate writes the bytes it returns.
- Invariant N12 is unchanged. It bounds compilation from a given IR. Reproducibility of the build that produces the IR is a separate obligation, stated in §3 above and tested by `build-hermeticity`.
- Trace output remains the human-readable view (spec §37). Humans read renderings of the IR and the Rust source; they do not hand-write the IR.
- Canon authoring gains a toolchain dependency. A Canon repository pins its Rust version the way this repository does, and the provenance record says which one built the IR.
- **Verification.** Grounding experiments `typed-validation`, `canonical-encoding`, `compatibility-matrix`, and `build-hermeticity`, none of which has run. The result records are the evidence this ADR is currently missing.
- **Failure behavior.** A consumer given bytes that are not a valid IR, carry an unsupported schema version, or hold an unknown executable variant rejects them before planning. There is no fallback to source, and a rejected IR produces no Plan.
- **Revisit trigger.** Reopen this decision if `build-hermeticity` finds an undeclared input that cannot be denied or recorded, if `canonical-encoding` finds no profile that passes the semantic equivalence tests, or if a second authoring surface is needed for authors who cannot run a Rust build.
