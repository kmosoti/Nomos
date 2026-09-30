# Canon Artifact Fixtures

Evidence for milestones `06-canon-artifact` and `08-resource-families`. The IR itself is specified in [canon-ir.md](../../../docs/formal/canon-ir.md).

| Path | What it is | Written by |
| --- | --- | --- |
| `golden/<name>.v3.cbor`, `golden/<name>.v3.json` | Seven Canons in the schema-3 artifact, in each profile: the five of `06` and two only schema 3 can say, `families` and `text3` | The independent encoders, `ciborium` with keys sorted by their encodings and `serde_json_canonicalizer`, from the Canons in `crates/core/nomos-canon/tests/support/mod.rs` |
| `golden/<name>.v2.cbor`, `golden/<name>.v2.json` | The five Canons of `06` in the schema-2 artifact, unchanged since `06` | The same encoders |
| `golden/<name>.v1.cbor`, `golden/<name>.v1.json` | The three of them that schema 1 can say | The same encoders |
| `golden/ids.txt` | Each schema-2 golden Canon's `CanonID` per profile, as `06` recorded it | `sha2` over the preimage of canon-ir.md |
| `golden/ids.v3.txt` | Each golden Canon's schema-3 `CanonID` per profile | The same |
| `compat/*.cbor`, `compat/*.json` | Artifacts a newer or broken writer could produce: in schema 2, an unknown kind, an unknown field, and an unknown state; schema 3 by number with a schema-2 body; schema 4; in schema 3, an unknown kind, an undefined `spec` field, and a `spec` value that is not text; and schema 1 with a schema-2 field | `crates/core/nomos-canon/tests/compatibility.rs`, from the values stated there |
| `authoring/` | A Canon authoring crate with its own lockfile: the generator under test, `telemetry`, and the negative control, `leaky` | By hand; built by `cargo xtask hermeticity` |
| `authoring-build-script/` | A crate with its own lockfile and a build script that does nothing: the negative control for `build-executes-only-the-toolchain` | By hand; built by `cargo xtask hermeticity --control build-script` |

The golden files are regenerated with `NOMOS_CANON_BLESS=1 cargo test -p nomos-canon --test encoding golden`, the compatibility files with `NOMOS_CANON_BLESS=1 cargo test -p nomos-canon --test compatibility compat_fixtures`. Without the variable the tests compare, and a difference fails. A regenerated fixture is a verifier change and is declared as one.

The `telemetry` generator's IR must equal `golden/telemetry.v3.*`, so the authoring build and the independent encoders agree on one Canon; until `08-resource-families` it was compared with `golden/telemetry.v2.*`, the schema then current.
