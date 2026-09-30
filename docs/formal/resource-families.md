# Resource Families

- **Status.** Working definition for milestone `08-resource-families`, written before the code (2026-09-30). It extends the file family of the [assessment algebra](../research/2026-09-28-typed-core/results/assessment-algebra.md) to the six resources of spec §10, and keeps the abstract `service` of `05-transition-kernel` as a seventh, legacy family that only the mock serves.
- **Governs.** `nomos-core` resource keys, Conditions, Observations, Assessments, and Operations; the Canon IR's schema 3 ([canon-ir.md](canon-ir.md)); and the per-family clauses of the [Substrate contract](substrate-contract.md).

A family fixes four things: how a resource is named, what may be required of it, what a collector may report about it, and what an Action does to it. The assessment rule of N13 holds in every family unchanged: a failed collection is Indeterminate, never a Variance, and only sufficient evidence yields Satisfied or a Variance.

## Resource Keys

A resource key is a family and a name. Two keys are equal only when both are. Keys order by name, then family, so every map and report is deterministic: a path begins with `/` and sorts before every other name, so the resources a path names keep the order of their paths, whatever their family, as schema 2 and the transition kernel's scenarios ordered them; one name in two families, a package and a user both called `nginx`, orders by the family's name. The text form is `<family>:<name>`, for example `unit:nginx.service`.

| Family | Name | Valid when |
| --- | --- | --- |
| `file` | An absolute path | As today: absolute, normalized, no zero byte, no trailing separator, at most 4096 bytes |
| `directory` | An absolute path | As for `file`; the root `/` is not a managed directory |
| `service` | An absolute path | As for `file`; legacy, served only by the mock |
| `unit` | A systemd unit name | 1 to 255 bytes of `[A-Za-z0-9:_.\@-]`, ending in `.service`, `.socket`, `.timer`, `.target`, `.path`, or `.mount`, with a non-empty stem |
| `sysctl` | A kernel parameter | Dot-separated segments of `[a-z0-9_-]`, at least two, at most 255 bytes; no `/`, so it cannot name a path outside `/proc/sys` |
| `user` | An account name | Debian's default: `[a-z_][a-z0-9_-]*`, optionally ending in `$`, at most 32 bytes |
| `package` | A Debian package name | Debian policy: `[a-z0-9][a-z0-9+.-]+`, at least 2 bytes, at most 128 |

A path names at most one resource of a Canon: a file, a directory, and a service at one path collide, and the Canon is rejected at admission (below).

## Families

Each table lists every requirement against every evidence, which is the truth table the implementation is tested against exhaustively. `—` in a requirement means any value is accepted.

### File

- **Requirement.** `absent`, or `present` with a content requirement (`any`, or exactly a digest) and optional metadata: owner, group, and mode.
- **Evidence.** `absent`, or `present` with the content digest, its size, and the observed owner, group, and mode. An owner or group is reported by name when the host names it, and by number otherwise.
- **Operation.** Make the file satisfy the requirement: remove it, or write it atomically with its metadata set before the rename (spec §11).

| Requirement | Evidence `absent` | Evidence `present` |
| --- | --- | --- |
| `absent` | Satisfied | Variance `unexpected` |
| `present` | Variance `missing` | Satisfied when the content requirement holds and every stated metadata field equals the observed one; otherwise Variance `content-differs`, or `metadata-differs` naming the first differing field in the order owner, group, mode |

Content is judged before metadata, so a file with wrong bytes reports `content-differs` whatever its mode. A metadata field the requirement does not state is never a Variance.

### Directory

- **Requirement.** `absent`, or `present` with optional owner, group, and mode.
- **Evidence.** `absent`, or `present` with the observed owner, group, and mode.
- **Operation.** Create the directory with its metadata, set the metadata of an existing one, or remove an empty one. Removing a directory that has entries is refused before any effect (clause S7): Nomos does not delete what it did not describe.

| Requirement | Evidence `absent` | Evidence `present` |
| --- | --- | --- |
| `absent` | Satisfied | Variance `unexpected` |
| `present` | Variance `missing` | Satisfied when every stated field equals the observed one; otherwise `metadata-differs` naming the first |

### Unit

- **Requirement.** An activity, `active`, `inactive`, or —, and an enablement, `enabled`, `disabled`, or —.
- **Evidence.** The unit's active state (`active`, `inactive`, `failed`, `activating`, `deactivating`, `reloading`) and its unit-file state (`enabled`, `disabled`, `static`, `masked`, or another). A unit systemd does not know or cannot load is a failed collection, `unavailable`, and so Indeterminate: its unit file may be written by an earlier Action of the same run.
- **Operation.** Start or stop the unit and enable or disable it, as the requirement states; or refresh it, when an `on_change` source of it acts in the round or it owes an Obligation: set its enablement as required and restart it, or stop it if the requirement says `inactive`. Enabling or disabling a unit whose unit-file state is `static`, `masked`, or another state the adapter cannot change is refused before any effect (clause S7).

| Requirement | Evidence |
| --- | --- |
| activity `active` | Satisfied on the activity axis when active; `reloading` counts as active; anything else is a Variance |
| activity `inactive` | Satisfied on the activity axis when inactive or failed |
| enablement `enabled` | Satisfied on the enablement axis when enabled |
| enablement `disabled` | Satisfied on the enablement axis when disabled; `static` and `masked` are a Variance, since the requirement names a state the unit is not in |

A unit is Satisfied when both axes are; otherwise the Variance `unit-differs` names each axis that differs. A unit that is `activating` or `deactivating` is a Variance on the activity axis: the run observes again before it concludes, and the transition kernel's bound applies.

### Sysctl

- **Requirement.** A value: text whose whitespace-separated tokens are compared after joining them with one space, which is how the kernel reports multi-valued parameters.
- **Evidence.** The value read from `/proc/sys`, normalized the same way. A parameter the running kernel does not have is a failed collection, `unavailable`.
- **Operation.** Write the value. It lasts until the next boot; the Cell's timer enforces it again, and an operator who wants it at boot manages a file in `/etc/sysctl.d/` too.

| Requirement | Evidence |
| --- | --- |
| a value | Satisfied when the normalized values are equal; otherwise Variance `value-differs` |

### User

- **Requirement.** `absent`, or `present` with a class, `system` or `regular`, and an optional home directory and login shell.
- **Evidence.** `absent`, or `present` with the numeric user ID, the primary group ID, the home directory, and the shell, as the user database records them.
- **Operation.** Create, modify, or delete the account with the distribution's tools (ADR 0013 §4, decision 1 of the Phase 1 plan). A `system` account has a numeric ID from 0 to 999, Debian's system range.

| Requirement | Evidence `absent` | Evidence `present` |
| --- | --- | --- |
| `absent` | Satisfied | Variance `unexpected` |
| `present` | Variance `missing` | Satisfied when the class matches the observed ID and every stated field equals the observed one; otherwise `account-differs` naming the first differing field in the order class, home, shell |

The class is judged, but never changed by an Action: an account in the wrong class is a Variance whose Action is refused before any effect, since renumbering an account changes the ownership of every file it has. An account whose numeric ID another account of the database also holds, an alias such as a second name for root, is refused the same way: the name the Canon manages and the number that owns files no longer name one identity.

### Package

- **Requirement.** `absent`, or `installed` with an optional exact version.
- **Evidence.** `not-installed`, `installed` with the version, or `broken` with dpkg's status, for a package dpkg holds in a half-installed, unpacked, or failed state. A package removed with its configuration files left behind is `not-installed`.
- **Operation.** Install a package, at the stated version when there is one, or remove it, with the distribution's tools.

| Requirement | Evidence `not-installed` | Evidence `installed` | Evidence `broken` |
| --- | --- | --- | --- |
| `absent` | Satisfied | Variance `unexpected` | Variance `broken` |
| `installed` | Variance `missing` | Satisfied when no version is stated or the versions are equal; otherwise `version-differs` | Variance `broken` |

### Service (Legacy)

The abstract service of `05-transition-kernel`: a path-named resource whose evidence is a file's, served by the mock so the transition kernel's scenarios keep their meaning. No host adapter serves it; a Canon that names one is refused by a reader that does not support the family, never skipped.

## Admission

A Canon is admitted only when no two of its resources write one property, the check of [reconciliation.md](reconciliation.md#composition) applied inside one Canon. The properties a resource writes:

| Family | Writes |
| --- | --- |
| `file`, `directory`, `service` | `path:<path>` |
| `unit` | `unit:<name>` |
| `sysctl` | `sysctl:<name>` |
| `user` | `user:<name>` |
| `package` | `package:<name>` |

A file and a directory at one path therefore collide, as do a service and a file, and the validator rejects the Canon with the colliding property named. Distinct keys of one family never collide, since the map of resources has one entry per key.

## Planning

The kernel plans every family alike (ADR 0009, [warp.md](warp.md)): a resource with a Variance or an Obligation gets an Action, and a resource that is the `on_change` target of an Action gets one too. A Variance of the resource's own is a reason to run, as an Obligation is, so an unchanged `on_change` source never makes a resource that differs from its requirement Skip: a unit that is stopped when it must run is started though its configuration file is Satisfied. The Action's operation is a refresh for a unit asked to refresh and for the legacy service, and converges the resource otherwise.

## What This Leaves Open

- Groups as a family, and supplementary group membership.
- A directory's unmanaged entries: they are tolerated, never removed.
- The user's numeric ID as a requirement.
- Package pinning beyond one exact version, repositories, and keys.
- Sockets, timers, and targets as anything but units to start, stop, enable, or disable.
