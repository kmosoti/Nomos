# Security Policy

Nomos changes files, packages, services, users, and kernel parameters on the machines it manages. It is a remote root-management system, and we treat it like one. Security reports are the highest-priority work in the project.

## Supported Versions

Nomos is in alpha. The `0.2` line includes the masterless Cell (`nomos-cell`) and headless Loom coordinator (`nomos-loom`) on Debian 12 and 13, and only the latest release receives security fixes: an alpha is replaced by the next, not patched. What the Cell protects, and what it does not, is in the [security model](docs/security-model.md).

| Version | Supported |
| --- | --- |
| `main` | Yes |
| The latest `0.2` release | Yes |
| Earlier `0.2` and `0.1` releases | No: upgrade to the latest |
| Any other branch or fork | No |

## Reporting a Vulnerability

**Do not report vulnerabilities in public issues, discussions, or pull requests.**

Report privately through GitHub's [private vulnerability reporting](https://github.com/kmosoti/nomos/security/advisories/new) for this repository.

Include:

- the affected component: Loom, Cell, Warp, Substrate, or a specific port or adapter
- the impact, for example privilege escalation, secret disclosure, execution of a stale or unauthorized Plan, or tampering with the Event Log
- steps to reproduce, or a proof of concept
- any safety invariant ([N1–N12](docs/PROJECT-SPEC.md#58-core-safety-invariants)) you believe is violated

## What to Expect

- Acknowledgement within **5 business days**.
- An assessment and remediation plan within **30 days**.
- Coordinated disclosure. We publish an advisory once a fix is available and credit you, unless you prefer otherwise.

## Scope

In scope:

- the Cell and its privilege boundary, including its state directory and the release package
- the headless Loom coordinator package, its lease broker, and its pinned QUIC listener
- the release process: how a release is admitted, built, and published ([release-process.md](docs/release-process.md))
- the Event Log's append-only guarantees

Designed, and in scope once they are in a release:

- the full Loom ↔ Cell convergence protocol, including fleet-wide fencing, replay, and stale-Plan handling
- production secret-store integrations beyond the alpha `age` and Virtual Trusted Platform Module (vTPM) adapter

The `0.1` line had no Loom, no network protocol, and no secrets. Its [security model](docs/security-model.md) lists what the Phase 1 Cell assumes and what it does not try to defend.

Out of scope:

- vulnerabilities in HashiCorp Vault, Headscale, systemd, or other third-party systems (report those upstream)
- attacks that require root on the managed host, or write access to `/etc/nomos` or the state directory, which the security model assumes the operator withholds
- attacks that require an already compromised Loom host

The security design lives in [docs/security-model.md](docs/security-model.md).
