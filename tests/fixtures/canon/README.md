# Canon Artifact Fixtures

Evidence for milestone `06-canon-artifact`. The IR itself is specified in [canon-ir.md](../../../docs/formal/canon-ir.md).

| Path | What it is | Written by |
| --- | --- | --- |
| `golden/<name>.v2.cbor`, `golden/<name>.v2.json` | Five Canons in the schema-2 artifact, in each profile | The independent encoders, `ciborium` with keys sorted by their encodings and `serde_json_canonicalizer`, from the Canons in `crates/core/nomos-canon/tests/support/mod.rs` |
| `golden/<name>.v1.cbor`, `golden/<name>.v1.json` | The three of them that schema 1 can say | The same encoders |
| `golden/ids.txt` | Each golden Canon's `CanonID` per profile | `sha2` over the preimage of canon-ir.md |
| `compat/*.cbor`, `compat/*.json` | Artifacts a newer or broken writer could produce: an unknown kind, an unknown field, an unknown state, schema 3, and schema 1 with a schema-2 field | `crates/core/nomos-canon/tests/compatibility.rs`, from the values stated there |
| `authoring/` | A Canon authoring crate with its own lockfile: the generator under test, `telemetry`, and the negative control, `leaky` | By hand; built by `cargo xtask hermeticity` |

The golden files are regenerated with `NOMOS_CANON_BLESS=1 cargo test -p nomos-canon --test encoding golden`, the compatibility files with `NOMOS_CANON_BLESS=1 cargo test -p nomos-canon --test compatibility compat_fixtures`. Without the variable the tests compare, and a difference fails. A regenerated fixture is a verifier change and is declared as one.

The `telemetry` generator's IR must equal `golden/telemetry.v2.*`, so the authoring build and the independent encoders agree on one Canon.
