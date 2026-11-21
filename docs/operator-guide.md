# Operator Guide

This guide takes a Debian host from nothing to converged with the Nomos Cell `0.1.0-alpha.2`: install it, author a Canon, trace it, enforce it, read the events, and uninstall. Every Cell claim below is checked by a test, named beside it; the release notes say what this alpha does and does not establish. For fleet coordination on the `v0.2.0-alpha.1` development line, see [Headless Loom Coordinator](#headless-loom-coordinator) and [Reusable NixOS Modules](#reusable-nixos-modules).

The Cell runs on Debian 12 and 13, amd64, with systemd as the init system. It needs root, as the tools it drives do.

The package declares what it needs, `systemd`, `dbus`, `passwd`, and `apt`, so installing it with `apt-get` brings them. It does not replace your init system: a host that does not boot with systemd as PID 1 is outside the contract, and the Cell reports its units as Indeterminate there. `minimal_install` checks, on Debian 12 and 13, that a base image with only the package installed has everything the Cell runs, and converges the demonstration Canon on it.

## Install

Download the package from the release and install it:

```sh
sudo apt-get install ./nomos-cell_0.1.0-alpha.2_amd64.deb
```

The package holds one static binary and two systemd units. `package_install`, on Debian 12 and 13, checks each of these:

| Path | What it is |
| --- | --- |
| `/usr/bin/nomos-cell` | The Cell, statically linked; it needs no library from the host |
| `nomos-cell.service` | Runs `enforce` on `/etc/nomos/canon.cbor` with the bundle at `/etc/nomos/bundle`, once, as root. It is skipped, not failed, while there is no Canon |
| `nomos-cell.timer` | Starts the service two minutes after boot and fifteen minutes after each run ends. Enabled and started when the package is installed |
| `/var/lib/nomos` | The Cell's state, mode `0700`: the content store, the journal, and a lock file. Any directory given with `--state` is held to the same rule: private to the user running the Cell, or refused |
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

`trace` and `events` never write to the Cell's state either: a journal with a half-written last record, which only a crash leaves, is reported and left for the next `enforce` to repair, and a state directory that does not exist is not created. They read a directory another process owns. `state_ownership` checks this.

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

A second `enforce` on a converged host executes nothing. `debian_convergence` checks, on Debian 12 and 13, that the demonstration Canon converges from sixteen starting states and that a second `enforce` executes nothing after each. `package_install` converges the same sixteen through the installed service.

Only one `enforce` or `import` can own a state directory at a time. A second one, started by hand while the timer's run is going or the other way round, stops at once with status 1 and `already in use`, having changed nothing; wait for the first to end. A run killed or crashed gives the directory up by itself. `state_ownership` checks this with real processes, on Debian 12 and 13.

If the Cell stops partway, the next run recovers from its journal: a refresh it owed is still owed, and it happens. `systemd_units` checks this with the Cell killed after every step of a configuration change.

## Events

```sh
sudo nomos-cell events
```

prints every Event of every run, numbered, from the journal: each Plan accepted, each Action dispatched and how it ended, each refresh owed and discharged, and each outcome. `cell_commands` checks that the events of two runs show both Plans and their outcomes.

`nomos-cell traits` prints what the Cell knows about the host, each fact with its source and how long it holds.

## Headless Loom Coordinator

On the `v0.2.0-alpha.1` development line, `nomos-loom` provides a headless pure-IPv6 coordinator binary packaged as `nomos-loom_<version>_amd64.deb` alongside `nomos-cell`. It embeds the SQLite Write-Ahead Log (WAL) credential lease broker, the Pressure Stall Information (PSI) and control group (`cgroup`) resource governor, HTTP and Server-Sent Events (SSE) endpoints, a SHA-256 Ed25519 fingerprint-pinned QUIC Remote Procedure Call (RPC) listener, and optional static web asset serving.

Install the Debian package built by `cargo xtask package` (or download both packages from a tagged release after checking `SHA256SUMS`):

```sh
sha256sum --check SHA256SUMS
sudo dpkg -i nomos-loom_*_amd64.deb
```

The package installs `/usr/bin/nomos-loom` and enables `nomos-loom.service`, which runs headless on `[::1]:7440` (HTTP) and `[::1]:7443` (QUIC) with state under `/var/lib/nomos-loom/leases.sqlite`. Inspect `nomos-loom --help` or run it explicitly with static web assets when needed:

```sh
nomos-loom serve \
  --listen '[::1]:7440' \
  --quic-listen '[::1]:7443' \
  --db-path /var/lib/nomos-loom/leases.sqlite \
  --vault-dir /var/lib/nomos/vault \
  --web-assets-dir /usr/share/nomos/web
```

When `--web-assets-dir` is omitted, HTTP fallback requests return a compact JSON status summary instead of static HTML, and `/health` reports `"headless": true`. Available HTTP routes are `/health`, `/v1/leases`, `/v1/leases/{id}`, `/v1/leases/verify`, `/v1/governor`, `/v1/governor/sample`, `/v1/telemetry`, and `/v1/events`.

## Reusable NixOS Modules

`flake.nix` exports parameterized NixOS modules under `nixosModules` (`default`, `base-ipv6`, `wireguard-wg0`, `secrets`, and `deadman-rollback`) using RFC 3849 and RFC 4193 IPv6 defaults. Import `nomos.nixosModules.default` and configure host-specific endpoints and keys in your own flake:

```nix
{
  imports = [ nomos.nixosModules.default ];

  services.nomos = {
    ipv6 = {
      enable = true;
      primaryInterface = "eth0";
    };
    wireguard = {
      enable = true;
      outerPmtu = 1460;
      addresses = [ "fd00:db8::1/64" ];
      privateKeyFile = "/run/credentials/wireguard-wg0/private.key";
      peers = [
        {
          publicKey = "<base64-peer-public-key>";
          allowedIPs = [ "fd00:db8::2/128" ];
          endpoint = "[2001:db8::2]:51820";
        }
      ];
    };
    secrets = {
      enable = true;
      enableTpm2 = true;
    };
    deadmanRollback = {
      enable = true;
      healthUrl = "http://[::1]:7440/health";
      probePeerIpv6 = "fd00:db8::2";
    };
  };
}
```

The `wireguard-wg0` module derives the tunnel Maximum Transmission Unit (MTU) as `outerPmtu - 80` and fails evaluation if the resulting MTU falls below `1200` or if non-IPv6 addresses are supplied.

## Uninstall

```sh
sudo apt-get remove nomos-cell    # the binary and the timer go; the state stays
sudo apt-get purge nomos-cell     # the state in /var/lib/nomos goes too

sudo apt-get remove nomos-loom    # stops nomos-loom.service; lease database stays
sudo apt-get purge nomos-loom     # removes /var/lib/nomos-loom too
```

Your Canon in `/etc/nomos` is yours and stays in both cases. Nothing the Cell changed on the host is undone: removing Nomos does not reverse what it enforced. `package_install` checks Cell installation, removal, and purge on Debian 12 and 13.
