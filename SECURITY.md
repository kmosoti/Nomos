# Security Policy

Nomos changes files, packages, services, users, and kernel parameters on the machines it manages. It is a remote root-management system, and we treat it like one. Security reports are the highest-priority work in the project.

## Supported Versions

Nomos has no releases yet. Until the first one, only `main` receives security fixes.

| Version | Supported |
| --- | --- |
| `main` | Yes |
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

- the Cell and its privilege boundary
- the Loom ↔ Cell protocol, including authentication, fencing, replay, and stale-Plan handling
- the Cipher port and its adapters, including any path by which secret plaintext could leak
- the Event Log's append-only guarantees

Out of scope:

- vulnerabilities in HashiCorp Vault, Headscale, systemd, or other third-party systems (report those upstream)
- attacks that require an already compromised Loom host, or root on the managed host

The security design lives in [docs/security-model.md](docs/security-model.md).
