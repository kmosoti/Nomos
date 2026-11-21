# ADR 0020: Reusable Parameterized NixOS Modules

- **Status.** Accepted
- **Date.** 2026-11-21

## Context

Phase 1 and mesh-enabled deployments require declarative NixOS system configuration for pure-IPv6 networking, WireGuard `wg0` interfaces sized below outer PMTU, secret credential provisioning, and automated rollback if a configuration switch breaks control-plane connectivity. Hardcoding site addresses, hostnames, SSH keys, or operator filesystem paths into modules prevents reuse across independent fleets.

## Decision

### 1. Publish Parameterized `services.nomos.*` Modules

Add `flake.nix` and modular NixOS definitions under `nix/modules/`:

- `base-ipv6.nix` (`services.nomos.ipv6.*`): configures pure-IPv6 networking, `net.ipv6.conf.<iface>.accept_ra = 2`, IPv6 Neighbor Discovery Protocol (NDP) and Path Maximum Transmission Unit (PMTU) ICMPv6 firewall allowances, and caller-supplied nameservers or documentation defaults (`2001:db8::/32`).
- `wireguard-wg0.nix` (`services.nomos.wireguard.*`): configures the `wg0` WireGuard interface with derived `mtu = outerPmtu - 80`, pure-IPv6 peer definitions using Request for Comments (RFC) 4193 `fd00:db8::/64` documentation defaults, UDP listen port, and private key file path.
- `secrets.nix` (`services.nomos.secrets.*`): declares parameterized credential staging directories, systemd `LoadCredential=` bindings, and virtual Trusted Platform Module (vTPM) and `systemd-creds` prerequisites without embedding keys or home-directory paths.
- `deadman-rollback.nix` (`services.nomos.deadmanRollback.*`): installs a bounded systemd deadman service and timer that rolls back the NixOS generation (`nixos-rebuild switch --rollback`) when control-plane health or mesh reachability fails within the operator-configured grace window.

### 2. Enforce Zero Embedded Infrastructure Data

All examples, defaults, and tests across `nix/modules/` and `flake.nix` use RFC 3849 (`2001:db8::/32`), RFC 4193 (`fd00:db8::/64`), and neutral host labels (`example-node`). Real deployments supply host-specific keys, addresses, and peer endpoints purely via caller module configuration.

## Consequences

- Downstream flakes import `nixosModules.default` or individual `services.nomos.*` modules without patching module source.
- Host convergence combines deterministic `nomos-cell` execution with declarative WireGuard MTU (`outerPmtu - 80`) and automatic rollback safety.
