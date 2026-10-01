# Security Model

- **Status.** The model of the masterless Cell, the `0.1.x` alpha line (Phase 1). It describes what the Cell does today, not what later phases plan. Loom, Cipher, signed Canons, and Cell-to-Loom authentication are not here, and the [project specification](PROJECT-SPEC.md#32-identity) is where they are designed.
- **Reading it.** A claim is **established** when a mechanism enforces it and a named test or record shows it. It is **assumed** when it holds only if the operator keeps it true. It is a **non-goal** when the Cell does not try. Nothing is called secure without one of the three.

To report a vulnerability, follow [SECURITY.md](../SECURITY.md).

## What the Cell Is, for This Purpose

`nomos-cell` is one static binary that runs as **root** on one Debian host. It reads a compiled Canon, observes the host, plans, and changes files, directories, kernel parameters, accounts, systemd units, and packages until the host matches. It runs when the operator starts it, and from a systemd timer. It opens no network socket and listens for nothing; the only network use is `apt-get` reaching the archives the host already trusts. Whoever controls what the Cell is told to do controls the host: the model is mostly about who that can be.

## Assets

| Asset | Why it matters |
| --- | --- |
| The managed host | The Cell changes it as root |
| The Canon artifact and its bundle, in `/etc/nomos` | They are the instructions, and the content the Cell writes |
| The state directory, `/var/lib/nomos` | The journal decides which Obligations are owed; the content store supplies the bytes `enforce` writes |
| The release package | It is installed as root, and its maintainer scripts run as root |
| The Event Log | It is what an operator reads to learn what the Cell did |

## Principals

| Principal | Trusted to | Not trusted to |
| --- | --- | --- |
| The operator, as root | Choose the Canon, start the Cell, own `/etc/nomos` and the state directory | |
| The Cell process | Do what its Canon and its journal say | Be run twice at once on one state directory (see Ownership) |
| A local unprivileged user | Nothing | Write `/etc/nomos`, the state directory, or the paths the Canon manages, or influence the Cell's environment |
| Another program that writes what the Canon manages | Nothing: its writes are undone at the next run | Be detected: nothing reports two writers taking turns ([ADR 0008](adr/0008-ownership-and-identity.md)) |
| The Canon's author | Whatever the Canon says is applied as root | Be verified: Canons are unsigned |
| A remote party with no local account | Nothing | Reach the Cell: it listens on nothing |
| The upstream supply chain: GitHub Actions, crates, the Debian archive | Build and deliver code that runs as root | Be inspected in this release beyond what [Supply Chain](#supply-chain-and-the-release) lists |

## Trust Boundaries

### The Canon and the Bundle

**Assumed.** `/etc/nomos/canon.cbor` and `/etc/nomos/bundle` belong to root and no one else can write them. The Cell reads whatever path it is given and checks neither its owner nor its mode: an artifact anyone can write is a root command anyone can issue. The package creates `/etc/nomos/bundle` and never the Canon, and removal and purge leave both in place.

**Established.** An artifact is decoded strictly: schema 1 to 3, every kind but the legacy `service`, and one the reader refuses is an error, never a partial Canon ([canon-ir.md](formal/canon-ir.md); `canon_artifact`). A bundle is imported whole or not at all, each file named by the SHA-256 of its bytes, and a file whose bytes do not match its name is refused ([ADR 0017](adr/0017-local-store.md) §2; `a_bundle_with_a_changed_file_is_refused_whole`).

**Not a goal in this release.** Signed Canons. A hostile Canon author is not defended against: the artifact says what to do, and the Cell does it.

### The State Directory

**Established.** At most one process owns a state directory, for the whole of an `enforce` or `import`; a contender ends with status 1 and changes nothing, and a killed holder gives it up without cleanup. `trace` and `events` never write the state. Every object under the state directory, the directory itself, its lock file, the journal, the content store, and every blob read, is opened without following a link and trusted only if it is private to the user running the Cell: the right type, owned by that user, and no permission bit for group or others, with the same rule for the packaged path and for a path given by `--state`. A state directory that fails is refused before the journal is replayed ([cell-commands.md](formal/cell-commands.md), State Directory; the [alpha2-state-ownership record](plans/results/alpha2-state-ownership.md); semantic mutants `SM-CELL-005` to `007` and `SM-STORE-005` to `007`).

**Assumed.** The directories above the state directory are the operator's: the Cell creates only the last component and does not check the ones above it. The state directory is on a local file system with working `flock`.

**Journal integrity is not tamper evidence.** Each record carries a SHA-256 of its length and payload, so a torn write is recognized and truncated, and a damaged record fails the open rather than being skipped ([ADR 0017](adr/0017-local-store.md) §3; `durable_log`). The digest is not keyed. Someone who can write the journal can write a record with a valid digest, so the checksum detects accident, not an attacker. The protection against an attacker is the state directory's mode and owner, above.

### The Managed Host

**Established.** The Linux Substrate resolves every path at use, relative to an open descriptor of the root, with `openat2` and `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`: no link is followed and nothing resolves outside the root, so a symbolic link planted in a managed path is not followed to somewhere else. A file is replaced atomically: written to a temporary file in the target's directory, synced, renamed over the target, and the directory synced. A failed read is Indeterminate, never taken for absence, so an attacker who can make a read fail cannot make the Cell conclude a file is missing and recreate it (N13; `linux-files-and-directories`).

**Assumed.** The checks and the changes of one run are not one atomic step: a path a local user can write can change between the Cell's observation and its change, which is why the paths a Canon manages are expected to be writable only by root. The Cell does not defend a managed path against a user who can already write it.

### Programs the Cell Runs

The Cell runs `/usr/sbin/useradd`, `/usr/sbin/usermod`, `/usr/sbin/userdel`, and `/usr/bin/apt-get`, each by a direct `execve` of its absolute path, with a fixed argument vector built from validated values, never through a shell (AGENTS.md rule 5), and with a cleared environment holding only a fixed `PATH`, `LC_ALL=C`, and, for `apt-get`, `DEBIAN_FRONTEND=noninteractive`. A package change is simulated first and refused if the simulation would remove a package the requirement does not name ([linux-packages](plans/results/linux-packages.md)).

**Assumed.** Those four programs, and what they run, are the distribution's. `apt-get` runs a package's maintainer scripts as root, so a package pinned in a Canon is trusted code: the Cell trusts the host's configured archives and their keys, and does not verify a package beyond what `apt-get` does.

### The Service Manager

The Cell talks to systemd over the system bus, as root, through `zbus`. It trusts the bus and systemd as the host's. A unit it manages is changed only through that interface. A host with no answering bus has its units reported Indeterminate, never guessed.

## Secrets

There are none in this release. Cipher, the secret type of the specification, is not implemented ([ADR 0013](adr/0013-trust-boundaries.md) §3). File content in a bundle is stored in the content store, mode `0600` under a `0700` directory, as plain bytes, and is written to the host as given. **Do not put a secret in a Canon or a bundle.**

## Supply Chain and the Release

| Property | Status |
| --- | --- |
| A release is checked against the previous release, and only a commit `main` contains can be released | Established by `check-release-base`, with negative controls ([verification-strategy.md](formal/verification-strategy.md#release-admission)) |
| `main` and the `v*` tags cannot be moved, deleted, or force-pushed, and `main` accepts only merged pull requests with passing checks | **Assumed**: the rules are repository settings, imported by the owner from [`.github/rulesets/`](../.github/rulesets/); the repository cannot check that they are on ([release-process.md](release-process.md)) |
| The Debian package published is the one both suites installed | Established by `check-release-chain`; the release names its digest, and carries a provenance attestation for it |
| A third-party action cannot change under a release | Established by `check-workflow-pins`: every action is pinned to a full commit SHA |
| The evidence-bearing tests cannot be edited as if they were implementation | Established by the trust-boundary gate and `check-evidence-tests` |
| The Rust dependencies are what `Cargo.lock` says | Established for the build (`--locked`); not audited for vulnerabilities in this release |
| The package is signed | **Not in this release.** The package is not in an apt repository and carries no signature of its own; verify it with `SHA256SUMS` and `gh attestation verify` |

## Non-Goals

- A malicious root, or a compromised kernel or service manager.
- A hostile Canon author: Canons are unsigned.
- Two writers on one resource: a second writer's changes are undone at the next run, and not reported.
- Defending a path against a local user who can already write it.
- Confidentiality of anything in a Canon or bundle.
- Authentication between a Cell and a Loom, and any network protocol: the Cell has neither ([ADR 0008](adr/0008-ownership-and-identity.md) §3 and §4 wait for Phase 3).
- Tamper evidence of the journal against someone who can write it.

## What an Operator Should Do

- Keep `/etc/nomos` and `/var/lib/nomos` owned by root and closed to others. The package makes the second `0700`; the first is yours.
- Put nothing secret in a Canon.
- Manage a path with the Cell or with another tool, not both.
- Install the package from this repository's release, and check `SHA256SUMS` and the attestation.
