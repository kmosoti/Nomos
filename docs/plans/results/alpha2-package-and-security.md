# Result: alpha2-package-and-security

- **Milestone.** `19-alpha2-package-and-security`, the rest of [issue #29](https://github.com/kmosoti/Nomos/issues/29), `v0.1.0-alpha.2`: issue #35 (declare and verify the Debian runtime dependencies) and issue #34 (the Phase 1 security model, and `SECURITY.md`).
- **Question.** Does installing the package through apt bring everything the Cell runs, and does the project's security documentation say what the Cell provides, assumes, and does not defend?
- **Outcome.** The package declares `Depends: systemd, dbus, passwd, apt`. On a base image of Debian 12 and 13 with only the package installed with `--no-install-recommends`, systemd booted from the package's own dependency, every program the Cell runs was present, and the demonstration Canon, which uses all six resource families, converged through the installed service and was Satisfied. Without the dependency on systemd the same host could not start systemd, and the control said so. [docs/security-model.md](../../security-model.md) is written, and [SECURITY.md](../../../SECURITY.md) supports the latest `0.1` release.

## Context

| Field | Value |
| --- | --- |
| Commit | `972e8f6` for every receipt but the last two, and `727ab79` for the second `check-trust-boundary` and for `receipts-validate` |
| Hosts | Debian GNU/Linux 12 (bookworm) and 13 (trixie), a base image and the package alone, in privileged containers of `tests/fixtures/debian-minimal/Dockerfile`, run by `cargo xtask debian-minimal`, on this sandbox's kernel (6.18.44) |
| Oracle | The base images themselves: `dpkg-query` for what is installed, the file system for what is present, and systemd's own state |
| Harness | `crates/bin/nomos-cell/tests/minimal_install.rs`; `crates/bin/nomos-xtask/src/{package,debian,release}.rs` |
| Receipts | `verification/receipts/2026-10-01-alpha2-hardening.ndjson` |

## The Dependency Inventory

The stock `debian:12` and `debian:13` base images hold `apt-get`, `useradd`, `usermod`, and `userdel`, and hold neither systemd nor D-Bus. The inventory is the adapter's own sources: the Cell runs exactly two programs through `Command`, by absolute path with a cleared environment, and reaches the service manager over D-Bus.

| What the Cell runs or needs | Owner on Debian 12 and 13 | Why |
| --- | --- | --- |
| `/usr/bin/apt-get` | `apt` | A package change, after a `--simulate` run |
| `/usr/sbin/useradd`, `usermod`, `userdel` | `passwd` | An account change |
| The system bus | `dbus` | Units are managed over it |
| systemd as the init system | `systemd` | The service and the timer, and the bus's other end |

`the_dependencies_account_for_everything_the_cell_runs` checks the inventory against the adapter's sources: it fails when the adapter stops naming a program in the inventory, and when a new place starts a program the inventory does not account for. `systemd-sysv` is deliberately not a dependency: it would replace the host's init. A host that does not boot with systemd as PID 1 is outside the contract, and the operator guide says so.

## What the Tests Establish

| Property | Test | Result, Debian 12 and 13 |
| --- | --- | --- |
| Everything the Cell runs is present, and each is declared | `minimal_install::the_package_alone_brings_everything_the_cell_runs` | Passed |
| The system bus answers, and systemd is `running` or `degraded` | the same | Passed |
| All six families converge through the installed service from nothing, and `trace` finds seven Conditions Satisfied | the same | Passed |

## Negative Control

`cargo xtask debian-minimal --control omit-systemd` builds the same binary into a package whose `Depends:` lacks systemd and passes only if the minimal host then cannot boot systemd. On both releases the container could not be created: `exec: "/lib/systemd/systemd": stat /lib/systemd/systemd: no such file or directory`. The control fired as expected.

## The Security Model

`docs/security-model.md` replaces a placeholder. It names the assets, the principals, and the trust boundaries of the masterless Cell as it is, and labels each claim **established** (a mechanism and a named test or record), **assumed** (of the operator), or a **non-goal**. The two sharpest limits are stated plainly: a Canon is unsigned and runs as root, so whoever can write `/etc/nomos` controls the host, and the journal's checksums detect accident, not an attacker, since they are not keyed. `the_security_documents_are_not_stale` fails when `SECURITY.md` says there are no releases, does not name the release line the workspace is on, or the model is a placeholder again.

## What This Does Not Establish

- **A host that is not a container.** Both releases ran in privileged containers booting systemd, on one kernel.
- **That the declared dependencies are minimal for every feature.** They are what the six families need; a family a later release adds extends the inventory, and its test will say so.
- **The Debian archive's own packages.** The minimal host installs `systemd`, `dbus`, `passwd`, and `apt` from the archive the image's apt reaches, and the Cell trusts them as the host's.
- **The security model's assumptions.** They are what an operator must keep true; no test can.

## Decision Fed

Every alpha.2 criterion of issue #29 has its evidence, except the repository rules of #31, which wait for the owner. `0.1.0-alpha.2` can be cut from `main` by the process in [release-process.md](../../release-process.md), once the owner applies the repository rules.
