# Layer-Policy Fixtures

Data for the `cargo xtask check-layers` tests in `nomos-xtask`. Cargo builds nothing here. The tests copy `base/` into a scratch directory, lay one case over it, and run `cargo metadata` there, so no lockfile or build output lands in the repository.

`base/` is a workspace laid out like Nomos, with the real crate names so the `nomos-core` rule is exercised, and every edge allowed: a core service to `nomos-core`, ports to core, the application to core and ports, each adapter to core and the port it implements, a composition root to the application and adapters, and a tooling crate nothing depends on. It is the positive control. A checker that rejected everything would fail on it.

Each directory under `cases/` holds only the manifests it overrides, and introduces one forbidden edge, in one way:

| Case | Edge | Rule | How it hides |
| --- | --- | --- | --- |
| `direct-core-adapter` | `nomos-canon` to `nomos-substrate-linux` | `core-depends-outside-core` | It does not |
| `direct-port-adapter` | `nomos-cipher` to `nomos-substrate-linux` | `port-depends-outside-core` | It does not |
| `direct-app-adapter` | `nomos-app` to `nomos-substrate-mock` | `app-depends-outside-core-and-ports` | It does not |
| `renamed` | `nomos-canon` to `nomos-substrate-linux` | `core-depends-outside-core` | Declared as `linux_backend = { package = "nomos-substrate-linux", ... }` |
| `optional` | `nomos-app` to `nomos-substrate-linux` | `app-depends-outside-core-and-ports` | `optional = true`, off by default; only the declared graph and an all-features resolution see it |
| `target-cfg` | `nomos-app` to `nomos-cipher-vault` | `app-depends-outside-core-and-ports` | Under `[target.'cfg(unix)'.dependencies]` |
| `build` | `nomos-canon` to `nomos-substrate-linux` | `core-depends-outside-core` | A build dependency |
| `dev` | `nomos-app` to `nomos-substrate-mock` | `app-depends-outside-core-and-ports` | A dev dependency, "only for testing" |
| `root-core` | `nomos-core` to `nomos-ids` | `root-core-has-workspace-dependency` | Core to core, allowed for every core crate except `nomos-core` |
| `adapter-extra-port` | `nomos-substrate-linux` to `nomos-cipher` | `adapter-depends-on-foreign-port` | The adapter also keeps its own port |
| `adapter-wrong-port` | `nomos-substrate-linux` to `nomos-cipher` | `adapter-depends-on-foreign-port`, `adapter-missing-its-port` | Exactly one port, the wrong one |
| `adapter-own-port-optional` | `nomos-substrate-linux` to `nomos-substrate` | `adapter-missing-its-port` | The adapter's own port is `optional = true`, so a default build lacks it |
| `adapter-own-port-target` | `nomos-substrate-linux` to `nomos-substrate` | `adapter-missing-its-port` | The adapter's own port is declared only for `cfg(windows)` |
| `adapter-names-no-port` | `nomos-telemetry-otel` | `adapter-names-no-port` | Adds an adapter whose name names no port |
| `app-to-tooling` | `nomos-app` to `nomos-xtask` | `depends-on-bin` | A production crate on the tooling crate |
| `cell-to-tooling` | `nomos-cell` to `nomos-xtask` | `depends-on-bin` | A composition root on the tooling crate |

Dependency kinds have no exemptions: a dev or build dependency is held to the same rule as a normal one, and the `build` and `dev` cases test exactly that. The reverse also holds for an adapter's own port: only a normal, non-optional, unconditional dependency counts as depending on it, and the two `adapter-own-port` cases test that. Each test asserts the complete set of rules the case breaks, so a case that starts failing for a different reason fails its test.

To add a case, add a directory with the overriding manifests and a test in `crates/bin/nomos-xtask/src/layers.rs` that names it. A test fails if a case directory has no test.
