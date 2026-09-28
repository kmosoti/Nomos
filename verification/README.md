# Verification

The policies and records that let a status in the [verification matrix](../docs/formal/verification-matrix.md) be traced to a run. Everything here is a verifier path under [ADR 0015](../docs/adr/0015-generator-verifier-development-model.md): a change to it goes in its own commit with a `Trust-Boundary: verifier` line, and `cargo xtask check-trust-boundary` fails a commit that does otherwise.

| File | Purpose | Read by |
| --- | --- | --- |
| [`checks.toml`](checks.toml) | The registry of checks a receipt may name, with the command each runs | `cargo xtask receipts record`, `receipts validate` |
| [`receipt.schema.json`](receipt.schema.json) | The shape of one receipt line | `receipts validate` |
| [`receipts/`](receipts/README.md) | One NDJSON file per recording session, one line per executed check | `receipts validate`, people filling the matrix |
| [`trust-boundary.toml`](trust-boundary.toml) | The protected specification and verifier paths, the implementation paths, and the escape-hatch patterns | `check-trust-boundary` |

## Recording a Check

```sh
cargo xtask receipts record check-layers --out verification/receipts/2026-09-28-example.ndjson \
    --unchecked "semantics; the gate checks workspace edges only" \
    -- cargo xtask check-layers
```

The tool runs the command directly, never through a shell, and appends one line with the check, the exact command, the commit and whether the tree was dirty, the toolchain channel, the lockfile digest, the exit status, digests of both output streams, the duration, and the test counts when the command ran `cargo test`. `result` is `passed` only when the exit status was 0, and the command must begin with the registered one, so a receipt for `check-layers` cannot be earned by running `true`.

## Validating Receipts

```sh
cargo xtask receipts validate
```

Every line is one strict JSON object, satisfies the schema, names a registered check, and claims `passed` only with evidence of a zero exit. The semantic rules are stated in the schema and again in the validator, so weakening one does not weaken the other. Each rejection carries a stable code: `receipt-passed-without-evidence`, `receipt-result-contradicts-exit-status`, `receipt-not-run-with-evidence`, `receipt-unknown-check`, `receipt-command-mismatch`, `receipt-malformed`, `receipt-schema-violation`, `receipt-registry-unusable`, `receipt-unreadable`.

A receipt is a record, not a proof. It carries no signature, and whoever can edit the repository can write one by hand. The validator makes a false receipt costly to write and a missing one impossible to mistake for a pass; it does not make either impossible. Whether receipts should be signed is an open question in the [generator-verifier digest](../docs/research/2026-09-28-generator-verifier/open-questions.md).

## Declaring an Oracle Change

A commit that changes a specification path or a verifier path carries, anywhere in its message body:

```text
Trust-Boundary: specification
```

or `Trust-Boundary: verifier`, and a commit that adds an escape hatch, an ignored test, a skipped mutant, an allowed panic lint, an assumption in a harness, carries `Trust-Boundary: escape-hatch`. Such a commit touches no implementation path. The gate makes the change visible under its own name; a reviewer decides whether it is right (AGENTS.md rule 11).
