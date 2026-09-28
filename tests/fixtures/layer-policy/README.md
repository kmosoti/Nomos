# Layer-Policy Fixtures

Data for the `cargo xtask check-layers` tests in `nomos-xtask`. Cargo does not build anything here; the tests copy `base/` into a scratch directory, lay one case over it, and run `cargo metadata` there.

`base/` is a workspace laid out like Nomos, with two core crates, one port, one app, one adapter, and one bin, and every edge allowed. It is the positive control: a checker that rejected everything would fail on it.

Each directory under `cases/` holds only the manifests it overrides. Every case introduces one forbidden edge, in one way:

| Case | Edge | How it hides |
| --- | --- | --- |
| `direct` | core to adapter | It does not |
| `renamed` | core to adapter | `package = "fx-adapter"` under another name |
| `optional` | app to adapter | `optional = true`, off by default, so only the declared graph and an all-features resolution see it |
| `target-cfg` | app to adapter | `[target.'cfg(unix)'.dependencies]` |
| `build` | core to adapter | `[build-dependencies]` |
| `dev` | app to adapter | `[dev-dependencies]`, the "only for testing" excuse |
| `two-ports` | adapter to a second port | Adds a port crate; an adapter implements exactly one |
| `bin-dep` | core to bin | Nothing depends on a bin crate |
| `core-to-port` | core to port | Core depends only on core |

To add a case, add a directory with the overriding manifests and a test in `crates/bin/nomos-xtask/src/layers.rs` that names the edge it expects.
