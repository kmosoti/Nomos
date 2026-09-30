# Operator Guide

This guide takes a Debian host from nothing to converged with the Nomos Cell `0.1.0-alpha.1`: install it, author a Canon, trace it, enforce it, read the events, and uninstall. Every claim below is checked by a test, named beside it; the release notes say what this alpha does and does not establish.

The Cell runs on Debian 12 and 13, amd64, with systemd. It needs root, as the tools it drives do.

## Install

Download the package from the release and install it:

```sh
sudo apt-get install ./nomos-cell_0.1.0-alpha.1_amd64.deb
```

The package holds one static binary and two systemd units. `package_install`, on Debian 12 and 13, checks each of these:

| Path | What it is |
| --- | --- |
| `/usr/bin/nomos-cell` | The Cell, statically linked; it needs no library from the host |
| `nomos-cell.service` | Runs `enforce` on `/etc/nomos/canon.cbor` with the bundle at `/etc/nomos/bundle`, once, as root. It is skipped, not failed, while there is no Canon |
| `nomos-cell.timer` | Starts the service two minutes after boot and fifteen minutes after each run ends. Enabled and started when the package is installed |
| `/var/lib/nomos` | The Cell's state, mode `0700`: the content store and the journal |
| `/etc/nomos/bundle` | Where the content of a Canon's files goes |

## Author a Canon

A Canon is written in Rust and compiled once, off the host, into an inert artifact (ADR 0004). The kit in `kit/canon` is a starting point: copy it, describe the host in `src/main.rs` with `CanonBuilder`, put the content of every exact file in `files/`, and run it:

```sh
cargo run -- out
```

It writes `out/canon.cbor`, the artifact, and `out/bundle/`, one file per SHA-256 digest the Canon names. The artifact holds digests, never file content. `authoring_kit` checks that the kit's artifact is one the Cell reads.

A Canon can require:

| Family | Requirement |
| --- | --- |
| `file` | Absent, or present with any content or exact content, and an owner, group, and mode |
| `directory` | Absent, or present with an owner, group, and mode; a directory with entries is never removed |
| `unit` | Active or inactive, enabled or disabled, and refreshed when a file it depends on changes |
| `sysctl` | A value, until the next boot, when the Cell's next run sets it again |
| `user` | Absent, or present as a system or regular account with a home and a shell |
| `package` | Absent, or installed, at an exact version if one is named |

Relations order them: `requires` holds a resource until another has succeeded, `after` only orders them, and `on_change` refreshes a unit when its source changes. The tables in [resource-families.md](formal/resource-families.md) are the exact rules; each family's suite checks them on Debian.

Copy the artifact and the bundle to the host:

```sh
sudo cp out/canon.cbor /etc/nomos/canon.cbor
sudo cp out/bundle/* /etc/nomos/bundle/
```

## Trace

`trace` shows what `enforce` would do, and changes nothing:

```sh
sudo nomos-cell trace --canon /etc/nomos/canon.cbor
```

For each Condition it prints the resource, its Assessment, and the Action planned for it:

```text
file:/etc/nomos-demo/demo.conf
  VARIANCE missing
  action: converge

file:/etc/secret.conf
  INDETERMINATE collection-failed: permission-denied
  action: none
```

A resource the Cell cannot observe is Indeterminate, and nothing is planned for it: unknown evidence is never taken as a difference. `cell_commands` checks that `trace` leaves the host as it found it, on Debian 12 and 13, and that a denied read plans nothing.

It exits 0 when every Condition is Satisfied, 2 when one is Indeterminate, 3 when there is something to do, and 1 on an error.

## Enforce

The timer runs `enforce` for you. To run it now:

```sh
sudo systemctl start nomos-cell.service
```

or directly, with a bundle to import first:

```sh
sudo nomos-cell enforce --canon /etc/nomos/canon.cbor --bundle /etc/nomos/bundle
```

It plans Actions for every Variance, applies them in the order the relations require, observes again, and ends with one outcome:

| Outcome | Status | Meaning |
| --- | --- | --- |
| `converged` | 0 | Every Condition Satisfied, nothing owed, every effect settled |
| `indeterminate` | 2 | Nothing left to do that the Cell can see, and something it cannot see |
| `non-convergent` | 3 | Still differing after eight rounds, or changing back and forth |
| `failed` | 4 | An Action failed; the resources are named |

A second `enforce` on a converged host executes nothing. `debian_convergence` checks, on Debian 12 and 13, that the demonstration Canon converges from sixteen starting states and that a second `enforce` executes nothing after each.

If the Cell stops partway, the next run recovers from its journal: a refresh it owed is still owed, and it happens. `systemd_units` checks this with the Cell killed after every step of a configuration change.

## Events

```sh
sudo nomos-cell events
```

prints every Event of every run, numbered, from the journal: each Plan accepted, each Action dispatched and how it ended, each refresh owed and discharged, and each outcome. `cell_commands` checks that the events of two runs show both Plans and their outcomes.

`nomos-cell traits` prints what the Cell knows about the host, each fact with its source and how long it holds.

## Uninstall

```sh
sudo apt-get remove nomos-cell    # the binary and the timer go; the state stays
sudo apt-get purge nomos-cell     # the state in /var/lib/nomos goes too
```

Your Canon in `/etc/nomos` is yours and stays in both cases. Nothing the Cell changed on the host is undone: removing Nomos does not reverse what it enforced. `package_install` checks removal and purge on Debian 12 and 13.
