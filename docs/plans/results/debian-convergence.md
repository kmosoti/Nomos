# Result: debian-fixed-point

- **Experiment.** `debian-fixed-point`, milestone `15-debian-convergence` of the [Phase 1 plan](../phase-1-masterless-cell.md), the Phase 1 exit.
- **Question.** From every enumerated starting state, does `enforce` converge the Debian host, does a second `enforce` execute nothing (spec §39, N3), and does `trace` afterward report every Condition Satisfied?
- **Outcome.** Yes, on Debian 12 and 13 in containers, for each of sixteen starting states of the demonstration Canon: from each, `enforce` converged, the service ran the configuration the Canon names, a second `enforce` executed nothing, and `trace` found all seven Conditions Satisfied.

## Context

| Field | Value |
| --- | --- |
| Commit | `c663483`, where every receipt was recorded |
| Hosts | Debian GNU/Linux 12 (bookworm) and 13 (trixie) in privileged containers of `tests/fixtures/debian/Dockerfile`, run by `cargo xtask debian` |
| Oracle | `enforce`'s outcome and `trace`'s Assessments, which are core's; the configuration the service itself copied when it last started; the package's version read with `dpkg-query` |
| Harness | `crates/bin/nomos-cell/tests/debian_convergence.rs`, running `nomos_cell::cli::run` with one state directory for every run, as an installed Cell keeps one |
| Receipts | `verification/receipts/2026-09-30-debian-convergence.ndjson` |

## The Demonstration Canon

A slice of spec §56, seven Conditions and five relations:

| Resource | Requirement |
| --- | --- |
| `user:nomos-demo` | A system account, home `/var/lib/nomos-demo`, shell `/usr/sbin/nologin` |
| `directory:/var/lib/nomos-demo` | Owned by `nomos-demo`, mode `0750`; requires the account |
| `directory:/etc/nomos-demo` | Owned by `root`, mode `0755` |
| `file:/etc/nomos-demo/demo.conf` | Exact content from the bundle, owned by `root`, mode `0644`; requires its directory |
| `sysctl:kernel.domainname` | `demo.nomos.example` |
| `package:nomos-demo-tool` | Installed at `1.0-1`, from a local repository offering `1.0-1` and `2.0-1` |
| `unit:nomos-demo.service` | Active and enabled; requires the state directory and the package; refreshed when the configuration file changes |

The service runs as the account and copies the configuration into its state directory when it starts, so the copy shows which configuration it runs. Its unit file is written by the test, not managed by the Canon.

## The Starting States

The plan asks for, per Condition, absent, present and wrong, and present and right. They were enumerated as follows, and every one converged on both releases with the executions below.

| Starting state | Executions |
| --- | --- |
| Nothing of the Canon's | 7 |
| Everything wrong | 7 |
| Converged | 0 |
| Account absent (the service stopped first, since an account in use cannot be deleted) | 3 |
| Account with the wrong shell | 1 |
| State directory absent (the service stopped first) | 2 |
| State directory owned by `root`, mode `0777` | 1 |
| Configuration directory absent, with its file | 3 |
| Configuration directory mode `0700` | 1 |
| Configuration file absent | 2 |
| Configuration file with other content | 2 |
| Kernel parameter with another value | 1 |
| Package not installed | 1 |
| Package at `2.0-1` | 1 |
| Service failed, killed after it started | 1 |
| Service stopped and disabled | 1 |

"Absent" for the kernel parameter has no meaning, since the kernel always has it, and for the service it is a failed unit, since its unit file is not the Canon's. Recreating the account gave it a new ID, so the state directory's owner differed too and was set again; a new configuration file led to the service's refresh. The starting states each family's suite injects as failures, a denied read, a symbolic link in place of a directory, a lock held on dpkg's database, are not states the Canon can converge from: they end Indeterminate or refused by design, and their suites cover them.

**A finding.** A first run failed at the eleventh state: the experiment restarts the service more often than systemd's default of five starts in ten seconds allows, and systemd refused the restart; Nomos reported the Action Failed and the run `Failed`, as it should. The test's unit lifts the limit with `StartLimitIntervalSec=0`. A real service that hits its limit fails its Action until the limit's window passes, and the Cell's next run retries.

## Semantic Mutants

No new mutant: the experiment adds no code under test, and exercises the adapters, the kernel, and the commands whose mutants are already in the corpus. All 63 active mutants were caught.

## What This Does Not Establish

- A Debian host that is not a container, a real archive package, and a service of the operator's.
- Combinations of wrong resources other than all of them at once.
- The installed binary, its service, and its timer: `16-alpha-release`.

## Decision Fed

Phase 1's exit is met on Debian 12 and 13 for the demonstration Canon. `16-alpha-release` packages the Cell and runs the same Canon through the installed service.
