# Result: build-hermeticity

- **Experiment.** `build-hermeticity`, milestone `06-canon-artifact`.
- **Question.** Do two isolated builds with the same declared inputs produce the same IR, and is every undeclared input denied or recorded? The contract is [ADR 0004](../../../adr/0004-rust-typed-canon.md) §3.
- **Outcome.** Yes, for one Canon authoring crate on one machine. Two builds from copies of the declared inputs at different paths, with different home, temporary, target, and working directories, at different times, produced the same IR bytes in both profiles, equal to the golden fixtures. The network and the environment were denied. Every file, network, randomness, working-directory, and clock access the jobs made was recorded and classified. The negative-control generator reads each of these inputs, and the harness saw every one. Run from inside the repository, the harness caught Cargo reading the workspace's own `.cargo/config.toml`, an undeclared input.

## Context

| Field | Value |
| --- | --- |
| Toolchain | Rust 1.98.1, `rustc 1.98.1 (48a229cea 2026-09-01)`; `strace`; `unshare` from util-linux; Valgrind with `--tool=none --trace-syscalls=yes` |
| Crate built | `tests/fixtures/canon/authoring`: its own workspace and lockfile, depending on `nomos-canon` and `nomos-core` by path and on nothing from a registry |
| Generators | `telemetry`, the generator under test, which builds the telemetry Canon with the typed builder; `leaky`, the negative control, which reads the clock, the locale and time zone, the temporary directory, the working directory, `/etc/hostname`, the network, and a hash seed, and writes each into its Canon |
| Harness | `cargo xtask hermeticity`, `crates/bin/nomos-xtask/src/hermeticity.rs`; its trace parser has unit tests |
| Record | [build-hermeticity.json](build-hermeticity.json), the harness's output at commit `e77575b`; its digest is the `stdout_sha256` of the `hermeticity` receipt |
| Receipts | `verification/receipts/2026-09-28-canon-artifact.ndjson` |

## The Job

The harness copies the declared inputs, as `git ls-files` lists them, into two roots whose paths differ in length: the authoring crate, `nomos-core`, `nomos-canon`, the workspace manifest they inherit from, and the toolchain file, 60 files. For each root it runs `cargo build --release --offline --locked` and then each generator in each profile, every command as

`unshare -rn` → `strace -ff -e trace=%file,%network,getrandom,getcwd` → `env -i PATH HOME CARGO_HOME TMPDIR` → the command,

from a working directory inside the root, and runs each generator once more under Valgrind, which hides the virtual dynamic shared object (vDSO) so that every clock read is a system call `strace` cannot otherwise see. The harness's own locale, time zone, and a noise variable differ between the roots and are outside `env -i`. The second root starts at least 1.1 seconds after the first.

| Undeclared input | Handling | Evidence |
| --- | --- | --- |
| Network | Denied: a network namespace with no route | The control's `connect` to a documentation address fails with `ENETUNREACH` and is recorded; the builds made no Internet-family socket call |
| Environment: locale, time zone, arbitrary variables | Denied: `env -i` with four declared variables | The control writes `locale:none` and `tz:none` in both roots, though the roots' ambient values differ |
| Files outside the declared inputs | Recorded, by class | The control's read of `/etc/hostname` is recorded; the generator under test opened only the dynamic loader's cache, `libgcc_s`, `libc`, `/proc/self/maps`, and its own binary |
| Randomness | Recorded, as `getrandom` | The generator under test makes one call, the C library's at start; the control makes two, the second its hash seed |
| Working directory | Varied and recorded, as `getcwd` | The control reads it; the generator under test does not |
| Clock | Varied and recorded, under Valgrind | The control makes one clock call; the generator under test makes none |
| Temporary, home, target directories and source path | Varied | The generator's IR is identical across the roots |
| Code run at build time | Recorded, as `execve` | The builds ran only `cargo`, `rustc`, the linker driver `cc`, `collect2`, and the toolchain's `rust-lld`: no build script and no procedural macro, the only user code that could read an input while building |

## Evidence

| Check | Result |
| --- | --- |
| `telemetry` IR identical across the roots | Passed, both profiles: Concise Binary Object Representation (CBOR) `3fe38598…`, JSON Canonicalization Scheme (JCS) `ebf7cf07…` |
| `telemetry` IR equals `tests/fixtures/canon/golden/telemetry.v2.*` | Passed, both profiles: the authoring build and the independent encoders agree |
| `telemetry` touches no undeclared path, makes no network call, reads no clock, reads no working directory | Passed, in each of four runs |
| The builds open no undeclared file and execute only the toolchain | Passed. The undeclared paths the builds touched without opening them are recorded: 50 failed probes, Cargo's configuration search up the ancestors and certificate-bundle locations, and three successful `lstat` or `statx` calls on `/tmp`, the scratch root, and `/usr/local/share` |
| Every build network attempt denied | Passed: none was made; the seven socket calls were on local descriptors |
| The control: output differs, `/etc/hostname` recorded, network attempt recorded and denied, clock recorded, randomness recorded, working directory recorded, ambient environment denied | Each fired |

The provenance ADR 0004 §3 names is in the record: the toolchain, the lockfile's digest (`5afe22af…`), the `nomos-canon` version, the count and a digest of the 60 declared input files, and each profile's IR digest.

## Negative Controls

- **The leaky generator.** Every check above that concerns it fired; a control that does not fire fails the run.
- **An ancestor configuration.** Run with its scratch directory inside the repository, the harness failed `build-opens-no-undeclared-file`: Cargo opened `/home/user/Nomos/.cargo/config.toml`, found by searching the build's ancestors. The default scratch directory is outside the workspace for that reason, and a real build job must run from a root with no configuration above it, or pass its configuration explicitly.
- **A build script.** `cargo xtask hermeticity --control build-script` builds `tests/fixtures/canon/authoring-build-script`, a crate with its own workspace and lockfile whose `build.rs` does nothing, through the same job in both roots, and runs only the three build checks; the record lists the 15 generator checks as not applicable and names the one failure the control expects. It failed `build-executes-only-the-toolchain` and nothing else, printing `LEAK [build-executes-only-the-toolchain]`: in each root the build executed `$JOB/target/release/build/canon-authoring-build-script-<hash>/build-script-build`, classified as build output. The network and undeclared-file checks passed; the build probed for `.git` and `HEAD` in each ancestor and opened none. Unit tests show the same outcome on a synthetic trace, without `strace`. The run is recorded as failed in `verification/receipts/2026-09-29-verifier-hardening.ndjson`, as a negative control is.
- **The parser.** Its unit tests show that the harness's own `env` is not charged to the job, that relative paths resolve against the working directory after a `chdir`, that a path climbing out with `..` is not taken for a declared one (the first version was, and the test found it), and that only Internet-family sockets count as network.

## Findings

- **Binaries differ, IR does not.** The two roots' generator binaries differ. Cargo's metadata hash covers a path dependency's absolute path, and `--remap-path-prefix` does not change that, so binary identity cannot stand in for IR identity here. ADR 0004 promises identical IR, which held.
- **The build reads configuration from its ancestors.** Cargo reads `.cargo/config.toml` in every ancestor of its working directory, and in `CARGO_HOME`. A build job that does not control its ancestors has an undeclared input that can change flags. It is recorded here and not denied.
- **The clock is invisible to `strace`.** A generator reads the clock through the vDSO with no system call. Valgrind made it visible; `strace` alone would have recorded nothing and the control's clock read would have gone unseen.

## Unchecked

- Clock reads during the build. They are not traced: the builds ran no user code, so only the pinned toolchain could read the clock while building.
- Environment variables the toolchain reads at build time. `env -i` bounds them to the four declared, and those four differ between the roots except `PATH`.
- Other machines, kernels, file systems, and toolchain installations. Both roots were built on one machine with one toolchain.
- Timing-dependent generators whose behavior differs only rarely; two runs cannot see them, and the clock record is what would.
- Mount-level isolation. Files outside the declared inputs are recorded, not denied; a job that must deny them needs a mount namespace, which this harness does not build.
- Procedural macros at build time. `rustc` loads a procedural macro as a shared library and does not execute it, so `build-executes-only-the-toolchain` cannot see one; a probe outside the harness, with `rustc` 1.94.1 rather than the pinned toolchain, showed the macro's `.so` opened from the target directory and no `execve` of it. That the authoring crate runs none rests on its lockfile, which names only `nomos-canon` and `nomos-core`, neither a procedural macro; no check covers it, and no control trips one.

## Decision

Adopt the pipeline for the Canon build job, with the conditions the findings name: the job runs in a network namespace with no route, under `env -i` with declared variables only, from a root with no Cargo configuration above it, with file and clock access recorded as here. Files outside the declared inputs are recorded, not denied, and the record is kept with the IR's provenance.
