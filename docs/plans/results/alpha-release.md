# Result: alpha-release

- **Milestone.** `16-alpha-release` of the [Phase 1 plan](../phase-1-masterless-cell.md): the Cell as a Debian package, a Canon authoring kit, and the documents an operator reads first.
- **Question.** Does the package install, upgrade, and purge cleanly on fresh Debian 12 and 13 containers booting systemd, and does the demonstration Canon of `15` converge from each enumerated starting state through the installed service?
- **Outcome.** Yes, on Debian 12 and 13 in containers. The package installed with its timer enabled and its state directory `0700`; the service was skipped without a Canon; with one, each of the sixteen starting states of [debian-convergence](debian-convergence.md) was converged by one run of the service, and the installed binary's own `trace` then found all seven Conditions Satisfied. An upgrade kept the timer and the journal, removal kept the state and the Canon, and purge removed the state and left the Canon and what the Cell had enforced.

## Context

| Field | Value |
| --- | --- |
| Commit | `482195c`, where every receipt was recorded |
| Hosts | Debian GNU/Linux 12 (bookworm) and 13 (trixie) in privileged containers of `tests/fixtures/debian/Dockerfile`, run by `cargo xtask debian`; the development host, Ubuntu 24.04.4, for the package build, the authoring kit, and the guide's commands |
| Oracle | The plan's exit; systemd's own record of each run (`Result`, `ExecMainStatus`, `ConditionResult`); `dpkg-query` and the file system for install, upgrade, removal, and purge; the Assessments of the installed binary's `trace` |
| Harness | `crates/bin/nomos-cell/tests/package_install.rs`, which runs the installed binary as its own process and the service through `systemctl`; `crates/bin/nomos-cell/tests/authoring_kit.rs`; `crates/bin/nomos-xtask/src/package.rs` and `canon_build.rs` |
| Receipts | `verification/receipts/2026-09-30-alpha-release.ndjson` |

## What Was Built

- **The package.** `cargo xtask package` builds `nomos-cell` for `x86_64-unknown-linux-musl`, statically linked, and assembles `nomos-cell_0.1.0~alpha.1_amd64.deb`: the binary at `/usr/bin/nomos-cell`, a oneshot `nomos-cell.service` that runs `enforce` on `/etc/nomos/canon.cbor` with the bundle at `/etc/nomos/bundle` and is skipped while there is no Canon, and `nomos-cell.timer`, two minutes after boot and fifteen after each run. The maintainer scripts create `/var/lib/nomos` with mode `0700` and enable the timer, disable it on removal, and delete the state on purge, never `/etc/nomos`.
- **The authoring kit.** `kit/canon`, a crate with its own workspace and lockfile, writes `out/canon.cbor` and `out/bundle/` for an example Canon. `authoring_kit` builds it and traces its artifact with the Cell.
- **The Canon build rule.** [canon-ir.md](../../formal/canon-ir.md), Provenance, now states that a Canon crate's build runs no procedural macro and no build script, the gap [build-hermeticity](../../research/2026-09-28-typed-core/results/build-hermeticity.md) left. `cargo xtask check-canon-build` reads the crate's resolved graph and refuses each such package; CI runs it on the kit.
- **The documents.** The [operator guide](../../operator-guide.md), each claim naming its test, and the [release notes](../../releases/v0.1.0-alpha.1.md), each claim naming its record.
- **The release.** A tag `v<version>` runs a CI job, after the required checks, that builds the package and attaches it with `SHA256SUMS` to a GitHub pre-release whose notes are `docs/releases/<tag>.md`.

## On Debian

| Step | Result, Debian 12 and 13 |
| --- | --- |
| Install | `/usr/bin/nomos-cell` present; the timer enabled and active; `/var/lib/nomos` mode `700`, owned by `root` |
| No Canon | The service's run skipped: `ConditionResult=no`, `Result=success` |
| Sixteen starting states | Each converged by one run: `Result=success`, `ExecMainStatus=0`; the installed `trace` then printed `7 satisfied, 0 variance, 0 indeterminate; 0 actions planned`, and the service ran the Canon's configuration |
| Upgrade | To `0.1.0~alpha.1+upgrade1`, the same binary; the timer still enabled; `events` still listed `plan-accepted cell generation 2`; the next run converged |
| Remove | The binary gone, the timer disabled; `/var/lib/nomos/journal` and `/etc/nomos/canon.cbor` kept |
| Purge | `/var/lib/nomos` gone, the package unknown to dpkg; the Canon kept, and the service's configuration still the Canon's |

**A finding.** The first run with all sixteen states was refused at the service's sixth start: systemd allows five starts in ten seconds by default, and the states converged faster than that. The run is a oneshot that is never restarted, so the limit protected nothing and would have refused an operator starting the service by hand; the package's service now lifts it, and `package`'s unit test checks that it does, and that the service has no `Restart=`.

**What a converged run costs.** After the sixteen states, four more runs of the service on the converged host, printed by the test and asserted of nothing. They were read from a run by hand, the same test binary with `--nocapture` in a container of each image, on the tree of `ba9365a`, since the test harness keeps a passing test's output to itself.

| Release | Journal growth per run | Run time, from systemd's timestamps |
| --- | --- | --- |
| Debian 12 | 1,914 bytes, each run, from 58,892 to 66,548 | 27 to 47 ms |
| Debian 13 | 1,914 bytes, each run, from 58,892 to 66,548 | 51 to 80 ms |

The journal is never compacted in this release, and each run replays it; at the timer's 96 runs a day, a converged host's journal grows by about 180 KB a day. Replay time over a long journal was not measured.

## Negative Controls

| Control | Code |
| --- | --- |
| `tests/fixtures/canon/authoring-proc-macro`, a crate depending on a procedural macro that defines nothing | `REFUSED [canon-build-proc-macro] canon-authoring-macro 0.0.0` |
| `tests/fixtures/canon/authoring-build-script`, the build-hermeticity control | `REFUSED [canon-build-script] canon-authoring-build-script 0.0.0` |

## Semantic Mutants

No new mutant: the package and the kit add no code under test in the Cell, and the check's negative controls are its fixtures. All 63 active mutants were caught.

## What This Does Not Establish

- The release job itself, until the tag runs it; the release is then its own evidence.
- A host that is not a container, and an installed Cell running for weeks: the journal's growth is measured above over a few runs, not observed over a long life.
- `apt-get` against Debian's archive: every package of the tests came from a local repository.
- Signed artifacts, Ciphers, and a second writer; the release notes list these.

## Decision Fed

Phase 1's exit criteria are met; `0.1.0-alpha.1` is tagged from the merge of this milestone.
