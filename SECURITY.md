# Security policy

Nomos changes files, packages, services, users and kernel parameters on the
machines it manages. It is effectively a remote root-management system. We
treat security reports as the highest-priority work.

## Supported versions

Nomos has not had a release yet. Until the first release, only the `main`
branch receives security fixes.

| Version | Supported |
|---|---|
| `main` | yes |
| any other branch or fork | no |

## Reporting a vulnerability

**Do not report security vulnerabilities in public issues, discussions or
pull requests.**

Report privately through GitHub's
[private vulnerability reporting](https://github.com/kmosoti/nomos/security/advisories/new)
for this repository.

Please include:

- the affected component (Loom, Cell, Warp, Substrate, a specific port or adapter);
- the impact, for example privilege escalation, secret disclosure, execution of a stale or unauthorized Plan, or tampering with the Event Log;
- steps to reproduce, or a proof of concept;
- any safety invariant (N1–N12 in the [spec](docs/PROJECT-SPEC.md#58-core-safety-invariants)) you believe is violated.

## What to expect

- We acknowledge the report within **5 business days**.
- We aim to assess it and give you a remediation plan within **30 days**.
- We coordinate disclosure with you. By default we publish an advisory once a
  fix is available, and we credit you unless you ask us not to.

## Scope

These are in scope:

- the Cell and its privilege boundary;
- the Loom ↔ Cell protocol, including authentication, fencing, replay and stale-Plan handling;
- the Cipher port and its adapters, including any path by which secret plaintext could leak;
- the Event Log's append-only guarantees.

These are out of scope:

- vulnerabilities in HashiCorp Vault, Headscale, systemd or other third-party systems themselves (report those upstream);
- attacks that require an already-compromised Loom host or root on the managed host.

The security design is documented in
[docs/security-model.md](docs/security-model.md).
