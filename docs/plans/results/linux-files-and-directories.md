# Result: linux-families, files and directories

- **Experiment.** `linux-families`, the part milestone `09-file-and-directory` of the [Phase 1 plan](../phase-1-masterless-cell.md) owns: files, with their owner, group, and mode, and directories.
- **Question.** Does each family on Linux pass the same suite as the mock, under failure injection?
- **Outcome.** Yes, for files and directories, on Debian 12 and Debian 13 in a container with systemd as PID 1, and on the development host. Every Substrate clause passed for both families, the three failure injections the plan names behaved as [substrate-contract.md](../../formal/substrate-contract.md) states, and content reached a host from the Cell's store while a tampered blob was refused.

## Context

| Field | Value |
| --- | --- |
| Commit | `ce59d29`, where every receipt was recorded |
| Hosts | Debian GNU/Linux 12 (bookworm), systemd `running`; Debian GNU/Linux 13 (trixie), systemd `degraded`; both on kernel 6.18.44 in a privileged container of `tests/fixtures/debian/Dockerfile`, run by `cargo xtask debian`; and the development host, Ubuntu 24.04.4 |
| Oracle | The per-family suite: requirements and starting points from the tables of [resource-families.md](../../formal/resource-families.md), judged by core on a new Observation |
| Harness | `crates/bin/nomos-cell/tests/substrate_conformance.rs` and `conformance/families.rs`; `crates/adapters/nomos-store-fs` |
| Receipts | `verification/receipts/2026-09-30-file-and-directory.ndjson` |

## The Suite on Linux

`the_linux_adapter_passes_the_suite_for_files_and_directories` ran S1 to S9 for both families: 29 tests of `substrate_conformance` passed on each Debian release and on the development host. S5 converged each requirement from each starting point: absent, present as `root` with mode `0644` or `0755`, and present as `app` with a group the databases do not name. Requirements named owners and groups from the scratch root's own `/etc/passwd` and `/etc/group`. S7 refused a requirement of another family, the other kind of resource at the path, a missing parent, an unknown account, a refresh, content the source does not have, and a directory with an entry asked to be absent.

The test drops root's capabilities to override permissions, so that a denied read is real. So the starting points owned by `app` grant read to others; a mode that grants none is set correctly by the adapter and then cannot be read back by the test, which the first run showed.

## Failure Injection

| Test | Injected | Result |
| --- | --- | --- |
| `a_directory_replaced_by_a_file_is_not_touched` | A managed directory replaced by a file | Observed as unsupported, never absent; creating and removing the directory are refused; the file is unchanged |
| `a_symbolic_link_in_place_of_a_directory_is_not_followed` | A symbolic link where a directory is required | Observed as unsupported; both operations refused; the linked directory's mode unchanged |
| `a_foreign_mode_change_before_verification_is_seen` | Mode `0666` set right after each execution, before verification | The run does not end Converged; without the writer, the next run converges and core judges the file Satisfied |
| `content_comes_from_the_store_and_a_changed_blob_is_refused` | A blob changed on the store's disk | The good blob is written; the changed one is refused, and nothing is written |

## Semantic Mutants

| Mutant | Wrong behavior | Caught by |
| --- | --- | --- |
| `SM-SUBSTRATE-006` | A file replaced without its metadata | `the_linux_adapter_passes_the_suite_for_files_and_directories` |
| `SM-SUBSTRATE-007` | No look for entries before removing a directory | the same |
| `SM-SUBSTRATE-008` | The leaf examined following a symbolic link | `a_symbolic_link_in_place_of_a_directory_is_not_followed` |
| `SM-STORE-001` | Stored bytes returned without their digest checked | `tests::a_changed_blob_is_corruption` |

All 42 active mutants of the corpus were caught; `SM-SUBSTRATE-002` and `004` were regenerated against the rewritten adapter.

## What This Does Not Establish

- A Debian host that is not a container, or a kernel other than 6.18.44; CI repeats the run on GitHub's runners.
- Supplementary groups, access control lists, extended attributes, and file systems other than the container's overlay.
- Races between the examination of a leaf and the operation on it, beyond the removal of a directory that gains an entry, which fails.
- Units, kernel parameters, users, and packages on Linux: `11` to `13`.

## Decision Fed

Files and directories are served on Debian. ADR 0017's first acceptance criterion is met; the second is `10-durable-cell`'s.
