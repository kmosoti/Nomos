# Result: linux-families, kernel parameters and users

- **Experiment.** `linux-families`, the part milestone `12-sysctl-and-user` of the [Phase 1 plan](../phase-1-masterless-cell.md) owns: kernel parameters and users.
- **Question.** Does each family on Linux pass the same suite as the mock, and do the plan's two exit cases hold: a parameter the running kernel does not have is Indeterminate, not a Variance; a user whose numeric ID another account holds is refused before any effect?
- **Outcome.** Yes, on the development host against scratch roots, and on Debian 12 and 13 in containers, where the running kernel's parameters and the host's own user database were changed too. One finding: the kernel keeps the first 64 bytes of an over-long domain name without an error, so only reading the value back shows that it kept another value.

## Context

| Field | Value |
| --- | --- |
| Commit | `0fbf8d4`, where every receipt was recorded |
| Hosts | The development host, Ubuntu 24.04.4 on Linux 6.18.44, with shadow 4.13's tools; Debian GNU/Linux 12 (bookworm) and 13 (trixie) in privileged containers of `tests/fixtures/debian/Dockerfile`, run by `cargo xtask debian` |
| Oracle | The families' tables in [resource-families.md](../../formal/resource-families.md), judged by core on a new Observation; the ground truth read from the files, with `getent` on the real host, never through the adapter |
| Harness | `crates/bin/nomos-cell/tests/substrate_conformance.rs`, `conformance/families.rs`, and `debian_host.rs`; `crates/adapters/nomos-substrate-linux/src/users.rs` |
| Receipts | `verification/receipts/2026-09-30-sysctl-and-user.ndjson` |

## What Was Built

Kernel parameters are the files of `/proc/sys` beneath the adapter's root, read and written with no program ([substrate-contract.md](../../formal/substrate-contract.md), Kernel Parameters on Linux). A write is read back, and the receipt is `Failed` when the kernel kept another value. Accounts are read from the user database and changed by a direct `execve` of `useradd`, `usermod`, or `userdel`, with a fixed argument vector and environment and no shell, with `--prefix` when the root is a scratch tree ([ADR 0013](../../adr/0013-trust-boundaries.md) §4, now decided). An account in the other class, or one whose ID another account holds, is refused before any effect, on the mock and on Linux alike.

## The Suites

`the_linux_adapter_passes_the_suite_for_kernel_parameters_and_users` ran S1 to S9 for both families, each against a scratch root of its own, here and on both Debian releases. The kernel parameters are a tree of regular files there, which is how the adapter reads them. The accounts were created, changed, and deleted by the real tools under `--prefix`; tracing the run showed each `execve` with the argument vector the contract names. S2's denied read was a user database of mode `0000` and a parameter file of mode `0000`, read by a thread without the capability to override permissions. S7 refused a requirement of another family, a parameter the kernel does not have, a system account asked to be a regular one, and an account aliased by a second name for its ID.

## On the Running Host

| Test | Oracle | Result, Debian 12 and 13 |
| --- | --- | --- |
| `the_running_kernel_s_parameters_are_read_and_written` | `/proc/sys/kernel/domainname` read directly | Read as it was; written and read back, `changed` once and not again; a 100-byte value `Failed`, with the kernel holding its first 64 bytes; `kernel.nomos_nowhere` observed `unavailable` and refused |
| `an_account_is_managed_on_the_host` | `getent passwd` | A system account created below ID 1000 with its home and shell and no home directory made; its shell changed; refused while `nomos-alias` shared its ID; deleted |

`kernel.domainname` belongs to the container's own namespace for host and domain names, so the test changes nothing outside it.

## Semantic Mutants

| Mutant | Wrong behavior | Caught by |
| --- | --- | --- |
| `SM-SUBSTRATE-014` | A missing parameter reported as an empty value | `the_linux_adapter_passes_the_suite_for_kernel_parameters_and_users` |
| `SM-SUBSTRATE-015` | An aliased account changed | the same |
| `SM-SUBSTRATE-016` | A parameter write completed without reading the value back | `the_running_kernel_s_parameters_are_read_and_written`, on Debian 12 |
| `SM-SUBSTRATE-017` | Every account created without `--system` | `the_linux_adapter_passes_the_suite_for_kernel_parameters_and_users` |

All 55 active mutants of the corpus were caught; `SM-SUBSTRATE-005` was regenerated against the mock's new lines.

## What This Does Not Establish

- Parameters whose kernel handlers reject a value with an error rather than clamp it, beyond the domain name; and parameters that only root in the initial namespace may write.
- A lock on the user database held by another tool: the tools own it, and a held lock fails the tool, so the receipt is `Failed`; no test holds it.
- Groups, supplementary membership, and a numeric ID as a requirement, which the family leaves open.
- Packages on Linux: `13`.

## Decision Fed

Kernel parameters and users are served on Debian. ADR 0013 §4 is decided for Phase 1, and `13-package` runs `apt-get` under the same rules.
