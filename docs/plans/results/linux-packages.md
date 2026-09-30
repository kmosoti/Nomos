# Result: linux-families, packages

- **Experiment.** `linux-families`, the part milestone `13-package` of the [Phase 1 plan](../phase-1-masterless-cell.md) owns: packages.
- **Question.** Do install, removal, and a pin converge on Debian; is a lock held by another package manager Indeterminate or a retry, never a false Variance; and does the fixed-point property (spec §39) hold after convergence?
- **Outcome.** Yes, on Debian 12 and 13 in containers, against a local repository of dummy packages the test builds. The package suite passed every clause. With dpkg's lock held, a package was Indeterminate with the collection timed out and nothing ran; released, a pin converged and a second Enforce executed nothing.

## Context

| Field | Value |
| --- | --- |
| Commit | `b73b5c2`, where every receipt was recorded |
| Hosts | Debian GNU/Linux 12 (bookworm) and 13 (trixie) in privileged containers of `tests/fixtures/debian/Dockerfile`, run by `cargo xtask debian`; the development host, Ubuntu 24.04.4, for the scratch-root tests |
| Oracle | The package table of [resource-families.md](../../formal/resource-families.md), judged by core on a new Observation; the ground truth read with `dpkg-query`, never through the adapter |
| Harness | `crates/bin/nomos-cell/tests/debian_host.rs` and `substrate_conformance.rs`; `crates/adapters/nomos-substrate-linux/src/packages.rs` |
| Receipts | `verification/receipts/2026-09-30-package.ndjson` |

## What Was Built

A package is read from dpkg's status database, never through a program, as [substrate-contract.md](../../formal/substrate-contract.md), Packages on Linux, states it. The host's own architecture is preferred over `all`, and a stanza of another architecture is not the package. Before reading, the adapter looks for a lock on dpkg's lock files in the kernel's lock table, which it reads and never takes; a lock still held after five seconds makes the collection time out. A package changes through `apt-get`, run by a direct `execve`, simulated first: a simulation that fails, or that would remove any package the requirement does not name, is refused before any effect.

## The Suite for Packages

`the_linux_adapter_passes_the_suite_for_packages` ran S1 to S9 on both releases. The test builds every package it names at versions `1.0-1` and `2.0-1` with `dpkg-deb`, writes the repository's index itself, and has apt read it as a trusted `file:` source; starting points are arranged with `dpkg --install`, `--unpack` for a broken package, and `--purge`. A denied read was a status database of mode `0000`, read by a thread without the capability to override permissions. S7 refused a requirement of another family, a refresh, the removal of a package another installed package depends on, and a version the index does not offer. A run on Debian 13 left 19 test packages installed, each the last state its key was arranged or converged to.

## The Milestone's Exit

| Test | Result, Debian 12 and 13 |
| --- | --- |
| `a_held_lock_is_indeterminate_and_a_pin_converges_to_a_fixed_point` | With a POSIX record lock on `lock-frontend` held by the test, the run ended `Indeterminate` with `CollectionFailed(TimedOut)` and nothing executed; released, a pin to `1.0-1` with `2.0-1` offered converged, and a second Enforce ended `Converged` with no execution |
| `a_held_dpkg_lock_times_the_collection_out` | Here, beneath a scratch root with a lock table naming the lock file: timed out after five seconds, then read `installed` once the table was empty; a package operation beneath a scratch root refused |

## Semantic Mutants

| Mutant | Wrong behavior | Caught by |
| --- | --- | --- |
| `SM-SUBSTRATE-018` | The database read without looking for a held lock | `a_held_dpkg_lock_times_the_collection_out` |
| `SM-SUBSTRATE-019` | `apt-get` run whatever its simulation would remove | `the_linux_adapter_passes_the_suite_for_packages`, on Debian 12 |
| `SM-SUBSTRATE-020` | A pinned package installed without its version | the same, on Debian 12 |
| `SM-SUBSTRATE-021` | An unpacked package read as installed | `packages::tests::the_status_database_reads_as_the_table_says` |

All 59 active mutants of the corpus were caught; `SM-SUBSTRATE-002` and `016` were regenerated against the adapter's new lines.

## What This Does Not Establish

- Packages from Debian's own archive, with real dependencies, maintainer scripts, and configuration files: the dummy packages have none, so `--force-confold` and a failing maintainer script were not exercised.
- A package index that is stale or missing: Nomos does not refresh it, and a version the index does not offer is refused.
- Held packages, multiple architectures installed at once, and a lock held for longer than `apt-get`'s 60-second wait during a mutation.

## Decision Fed

Every family of Phase 1 is served on Debian. `14-cell-commands` puts them behind the Cell's commands.
