# Substrate Contract

What every adapter of the `nomos-substrate` port promises, how the conformance suite checks it, and the first operation the Linux adapter performs. [ADR 0006](../adr/0006-kernel-contract.md) keeps semantics in core: an adapter obtains evidence and performs operations, and never decides what satisfaction means. [ADR 0013](../adr/0013-trust-boundaries.md) splits the port into an observe and a mutate capability and resolves paths under a boundary. This document gives the working definitions milestone `07-substrate-conformance` implements and tests.

## The Port

The port has two capabilities. `Observe::observe(resources)` returns Observations. `Mutate::apply(request)` performs one execution, named by its idempotency key, and returns its receipts in order. Both are synchronous. Trace is given an `Observe` and nothing else.

## The Clauses

Every adapter, the mock and Linux alike, satisfies these. The conformance suite checks each clause by name.

| Clause | Statement |
| --- | --- |
| S1, coverage | For each requested resource, `observe` returns at least one Observation of that resource, and none of a resource not requested. |
| S2, truthful absence | An Observation is collected as absent only when the adapter established that no file exists at the resource. A read the adapter could not perform is a failed collection with its reason, never an absent file. |
| S3, evidence and not verdicts | A present file is reported with the digest of its content and its size. The adapter reports what it saw; the types give it no way to report Satisfied, a Variance, or Indeterminate. |
| S4, observation does not mutate | `observe` changes nothing in the managed-resource projection (N1): after any sequence of observations, every managed resource holds what it held before. |
| S5, postconditions are core's | A `Completed` receipt says that the execution finished and whether it changed the resource, and nothing more. Whether the Condition holds is decided by core's `assess` on a new Observation. The `changed` flag is true exactly when the resource differs from before. |
| S6, at most once per key | A request whose key the adapter has seen returns the receipts it returned then and changes nothing. |
| S7, refusal before effect | An operation the adapter cannot perform is `Refused`, and nothing changes. |
| S8, settlement | The effect causes no change after its settle-by instant. An adapter that completes an execution before `apply` returns ends every receipt sequence with a settling receipt: `Completed`, `Failed`, or `Refused`. |
| S9, one clock | The collection window of an Observation is on the clock the driver ticks the kernel with, so that the kernel's freshness comparisons are meaningful. |

The suite also checks one property of the kernel at the port: **an unresolved effect keeps its reservation**. When an adapter's receipts for an execution carry no settlement evidence, the kernel keeps the Action's reservation, and the run does not end `Converged` (N10). This is the kernel's rule, checked at the port because the port is where settlement evidence goes missing.

## The Clauses per Family

The clauses name files, as the first operation did; from milestone `08-resource-families` they hold for every family of [resource-families.md](resource-families.md), read this way:

- **S2.** Absence is family-specific evidence: no file or directory at the path, no account of the name, a package dpkg records as not installed. A unit systemd does not know and a sysctl the kernel does not have are failed collections, `unavailable`, never absence.
- **S3.** A family reports the evidence its table lists, and nothing that judges it.
- **S5.** `changed` is true exactly when the family's evidence would differ from before: a unit started, a value written that was not there, a package installed.
- **S7.** Each family's refusals are part of its contract: removing a directory with entries; a file where a directory is, or a directory where a file is; changing a user's class; changing or deleting an account whose numeric ID another account also holds; enabling or disabling a unit whose unit-file state is fixed; an operation on a unit or a kernel parameter the host does not know; a package operation whose simulation fails or would remove a package the requirement does not name; a requirement of another family than the resource's; a refresh of anything but a unit or the legacy service; and any operation on a family the adapter does not serve.

The conformance suite runs every clause for every family an adapter serves, from a list of requirements and starting points per family taken from its truth table; S8 is checked on every execution S5 and S7 perform. The mock serves all seven; the suite runs for the six Phase 1 families, and the legacy service keeps the file suite's and the transition kernel's checks. The Linux adapter serves a family from the milestone that implements it; until then, a key of the family is observed as a failed collection, `unsupported`, and every operation on it is refused.

## The Conformance Suite

The suite is one set of tests, generic over the adapter under test, at the composition boundary `crates/bin/nomos-cell/tests/`. Each adapter supplies a test subject: the adapter, plus a way to arrange the world outside it, as a foreign writer would. The subject can write a file's content, remove a file, deny reads of a file, and read the ground truth of a file without going through `observe`.

**Negative controls.** Two wrappers break a clause on purpose, and the suite must fail each:

- **A mutating Trace.** Observation writes to the resource it observes. S4 must fail.
- **A denied read reported as absence.** A failed collection is turned into an absent file. S2 must fail.

**End to end.** For each adapter, the production driver enforces a Canon of one file with exact content and converges; enforcing it again executes nothing (N3); a denied read ends the run with the resource Indeterminate and no mutation (N13).

## The Linux Adapter's First Operation

The adapter manages regular files beneath one root directory. The root is `/` in production and a scratch directory in tests; a resource path names a file relative to it.

**Resolution.** Every path is resolved at use, relative to an open descriptor of the root, with `openat2` and the resolve flags `RESOLVE_BENEATH`, `RESOLVE_NO_SYMLINKS`, and `RESOLVE_NO_MAGICLINKS` (ADR 0013 §2). No symbolic link is followed, at the leaf or in any parent, and nothing resolves outside the root.

**Observation.** Each result of opening and reading the file maps to one collection:

| Result | Collection |
| --- | --- |
| The file is regular and its bytes were read | Collected, present, with the Secure Hash Algorithm (SHA) 256 digest of the bytes, their count, and the owner and group by number and the mode, as `fstat` reports them |
| No file at the path (`ENOENT`), or a parent that is not a directory (`ENOTDIR`) | Collected, absent |
| Permission denied (`EACCES`, `EPERM`) | Failed, permission denied |
| A symbolic link on the path (`ELOOP`), a path that would leave the root (`EXDEV`), or something other than a regular file | Failed, unsupported |
| Any other error | Failed, input and output (`Io`) |

The collection window is the monotonic clock's reading before the open and after the read.

**Mutation.** `Converge` with a file requirement makes the file satisfy it, and nothing else is supported:

| Requirement | Effect |
| --- | --- |
| Absent | Remove the file if it exists. Completed, changed exactly when a file was removed |
| Present, any content | Create an empty file if none exists. Completed, changed exactly when one was created |
| Present, exact content | Write the content whose digest the requirement names, atomically: a temporary file in the same directory, written and synced, given mode `0644`, renamed over the target, and the directory synced (spec §11). Completed, changed exactly when the content differs from before |

The adapter refuses (S7), changing nothing, when: the operation is a refresh, which waits for the systemd resource; the resource is neither a file nor a directory; the content of an exact requirement is not in the content source; the parent directory does not exist, since directories are a separate resource; or something other than a regular file is at the path. If the rename fails after the temporary file was written, the temporary file is removed and the receipt is `Failed`.

**Content.** An exact requirement carries a digest, not bytes. The adapter reads bytes from a content source, a trait of its port that the composition root backs with the Cell's content store ([ADR 0017](../adr/0017-local-store.md)); a source that returns bytes whose digest is not the one asked for is treated as not having them.

**Execution.** Each execution completes before `apply` returns (S8). The adapter remembers the receipts of every key it has seen (S6).

## Files and Directories on Linux

From `09-file-and-directory` the adapter serves the `directory` family and manages a file's metadata, as [resource-families.md](resource-families.md) states them.

**Accounts.** An owner or group is named in a requirement and numbered on disk. The adapter reads `/etc/passwd` and `/etc/group` beneath its root, at use, with the same resolution as every other path, and never through the C library's name service. Evidence reports an ID by the first name the database gives it, and by number when it has none. A requirement that names an account the database does not have is refused before any effect.

**Observation.** The parent is opened beneath the root, and the leaf is examined with `fstatat` and `AT_SYMLINK_NOFOLLOW`, so the leaf's type is known before anything is opened:

| Leaf | File key | Directory key |
| --- | --- | --- |
| None, or a parent that is missing or not a directory | Collected, absent | Collected, absent |
| A regular file | Its digest, size, and metadata, read through a descriptor opened with `O_NOFOLLOW` | Failed, unsupported |
| A directory | Failed, unsupported | Its metadata |
| A symbolic link or anything else | Failed, unsupported | Failed, unsupported |

Permission and other errors map as in the first operation's table.

**Mutation.** Metadata is set on a descriptor, never on a path, and before the rename (spec §11). A stated field is set; an unstated one is kept from the resource that was there, or is the default for a new one: owned by the adapter's user and group, mode `0644` for a file and `0755` for a directory.

| Requirement | Effect |
| --- | --- |
| File present, content differs or absent | A temporary file in the same directory, created mode `0600`, written, given its owner, group, and mode, synced, renamed over the target, and the directory synced |
| File present, content equal, metadata differs | The file opened with `O_NOFOLLOW`, its owner, group, and mode set on the descriptor, and synced |
| Directory present, absent before | Created with `mkdirat` mode `0700`, opened with `O_DIRECTORY` and `O_NOFOLLOW`, given its owner, group, and mode, and the parent synced |
| Directory present, metadata differs | Opened as above and its metadata set |
| Directory absent | Removed with `unlinkat` and `AT_REMOVEDIR` when it has no entries |

`changed` is true exactly when the content, the owner, the group, or the mode differs from before, or the resource was created or removed. The adapter refuses (S7), changing nothing, when the parent does not exist; a directory is where a file is required or a file where a directory is; a directory to remove has entries, read before any effect; or a requirement names an unknown account. An entry that appears between the check and the removal fails the removal, and the receipt is `Failed`.

## Units on Linux

From `11-systemd-unit` the adapter serves the `unit` family, as [resource-families.md](resource-families.md) states it, through systemd's D-Bus API: the manager `org.freedesktop.systemd1` on the system bus, over `zbus`'s blocking client (Phase 1 plan, decision 2). It never runs `systemctl` (AGENTS.md rule 5). Units are served only by a host given a connection to the bus; a host without one observes a unit as a failed collection, `unsupported`, and refuses every operation on it, as before.

**Observation.** systemd unloads a unit that is not running and that nothing references, so a unit it does not report as loaded may still have a unit file.

| systemd answers | Evidence |
| --- | --- |
| `GetUnit` names a unit whose `LoadState` is `loaded` or `masked` | Its `ActiveState` and `UnitFileState` |
| `GetUnit` names a unit whose `LoadState` is anything else, such as `not-found`, `bad-setting`, or `error` | Failed, `unavailable` |
| `GetUnit` fails with `NoSuchUnit`, and `GetUnitFileState` returns a state | `inactive`, with that state |
| `GetUnit` fails with `NoSuchUnit`, and `GetUnitFileState` fails with `FileNotFound` or `NoSuchUnit` | Failed, `unavailable` |
| An `ActiveState` other than the six of the family, such as `maintenance` or `refreshing` | Failed, `unsupported` |
| `AccessDenied` from the bus | Failed, permission denied |
| `NoReply` or `Timeout` from the bus | Failed, timed out |
| No bus, or any other error | Failed, `io` |

A `UnitFileState` of `enabled`, `disabled`, or `static` is that state; `masked` and `masked-runtime` are `masked`; anything else, such as `enabled-runtime`, `linked`, `indirect`, `generated`, or `transient`, is `other`.

**Mutation.** The adapter first observes the unit as above. It refuses, changing nothing (S7), when the observation is not collected, when the requirement is of another family, or when the requirement states an enablement and the unit-file state is neither `enabled` nor `disabled`. Otherwise, in order:

| Step | When | Call |
| --- | --- | --- |
| Enablement | The requirement states an enablement the unit-file state does not have | `EnableUnitFiles([name], false, false)` or `DisableUnitFiles([name], false)`, then `Reload` |
| Activity, converge | The requirement says `active` and the unit is not `active` or `reloading` | `StartUnit(name, "replace")` |
| Activity, converge | The requirement says `inactive` and the unit is not `inactive` or `failed` | `StopUnit(name, "replace")` |
| Activity, refresh | The requirement says `inactive` | `StopUnit` when the unit is not `inactive` or `failed` |
| Activity, refresh | Otherwise | `RestartUnit(name, "replace")` |

**Settlement.** A job is the settlement evidence. After a start, stop, or restart, the adapter polls the unit's `Job` property until no job is pending, and then reads the unit again. A start or restart succeeded when the unit is `active` or `reloading`, and a stop when it is `inactive` or `failed`; otherwise the receipt is `Failed`, which leaves the Action Failed and not Converged. The adapter waits until the request's settle-by instant, on its own monotonic clock (S9), and at most 90 seconds. A job still pending then is cancelled, and the receipts end without a settling receipt: whether the unit changed is unknown, and stays unknown (N10).

`changed` is true, for a convergence, exactly when the unit's evidence after the operation differs from its evidence before. A refresh that succeeds reports `changed: true`, as the mock does: a restart replaces the running service though its evidence reads the same, and the refresh is the Action's effect.

**Testing.** The unit suite changes the host's service manager, so it runs only where systemd is PID 1 in a disposable container: `cargo xtask debian` runs it, and a workspace test run on any other host lists it as ignored rather than passing it. The suite writes its own unit files under `/etc/systemd/system`, arranges each starting point as a foreign writer would, with `systemctl`, and reads the ground truth with `systemctl show`, never through the adapter. A denied read is arranged with a mandatory D-Bus policy that denies messages to the unit's object path. The suite's starting points on Linux are the stable ones, `active`, `inactive`, and `failed` with each unit-file state: a unit `activating`, `deactivating`, or `reloading` leaves that state on its own, so a clause that compares the ground truth before and after an operation cannot hold it still. Those three are covered on the mock by the suite, and on Linux by a test that holds a unit in each for a few seconds and observes and converges it.

## Kernel Parameters on Linux

From `12-sysctl-and-user` the adapter serves the `sysctl` family, as [resource-families.md](resource-families.md) states it, through the files of `/proc/sys` beneath its root: file I/O, with no program. A parameter's name maps to a path by replacing each dot with a slash, so `net.ipv4.ip_forward` is `proc/sys/net/ipv4/ip_forward`; a name has no `/`, so it cannot name a path outside `/proc/sys`. The path is resolved as every other path is (ADR 0013 §2).

| The file | Evidence |
| --- | --- |
| Read | The value, normalized: its whitespace-separated tokens joined with one space |
| Does not exist, or a component of its path does not | Failed, `unavailable`: the running kernel does not have the parameter |
| Is a directory | Failed, `unsupported` |
| Read denied | Failed, permission denied |
| Any other error | Failed, `io` |

A convergence writes the requested value, followed by a newline, in one `write` to the file opened for writing with truncation, as a shell's redirection opens it, and reads it back. The receipt is `Completed` when the value read back equals the value asked for, `changed` when it differs from the value before; it is `Failed` when the write fails or the kernel keeps another value, as it does for a value it clamps or rejects. A parameter the kernel does not have is refused before any effect (S7). The value lasts until the next boot.

## Users on Linux

From `12-sysctl-and-user` the adapter serves the `user` family.

**Observation.** The user database, `/etc/passwd` beneath the root, read at use as for owners and groups, never through the C library's name service. An account is present with its numeric ID, primary group ID, home directory, and shell as the first entry of its name records them, and absent when no entry has the name.

**Mutation.** Through the distribution's own tools, decided in [ADR 0013](../adr/0013-trust-boundaries.md) §4: `useradd`, `usermod`, and `userdel`, each run by a direct `execve` of its absolute path under `/usr/sbin`, with a fixed argument vector per operation, an environment of only `PATH=/usr/sbin:/usr/bin:/sbin:/bin` and `LC_ALL=C`, standard input from `/dev/null`, and no shell. An account name cannot begin with `-` and a path begins with `/`, so neither can be read as an option; the name follows `--` all the same. When the root is not `/`, every tool is given `--prefix` and the root, so a test manages the databases of a scratch tree.

| Requirement | Evidence | Command |
| --- | --- | --- |
| `present` | `absent` | `useradd [--system] --no-create-home [--home-dir H] [--shell S] -- name`, `--system` for the `system` class |
| `present` | `present`, home or shell differs | `usermod [--home H] [--shell S] -- name`, naming only the fields that differ; the home directory is not moved |
| `absent` | `present` | `userdel -- name`, which leaves the home directory and the files the account owns |

The tools own the locking of the databases and the consistency of the shadow files; Nomos never writes them itself. A tool that exits with any status but 0 gives the receipt `Failed`, whatever it did; the account is then read again from a new Observation. The adapter refuses, before any effect (S7):

- an account in the other class than the requirement's, since renumbering changes the ownership of every file the account has;
- an account whose numeric ID another entry of the database also holds, since the name managed and the number that owns files no longer name one identity;
- a requirement of another family, or any refresh.

`changed` is true exactly when the account's evidence after the command differs from before.

**Testing.** The user suite runs here and on Debian against a scratch root, with `--prefix`, so it changes no account of the host. The kernel-parameter suite runs against a scratch root whose `proc/sys` is a tree of regular files, which is how the adapter reads it; a test in the Debian container reads and writes the running kernel's own `kernel.domainname`, which is private to the container's own namespace for host and domain names, and observes a parameter the kernel does not have.

## Packages on Linux

From `13-package` the adapter serves the `package` family, as [resource-families.md](resource-families.md) states it.

**Observation.** dpkg's status database, `/var/lib/dpkg/status` beneath the root, read directly and never through a program. dpkg replaces the file whole by a rename, so a read sees one state of it. A package's entry is the one whose `Architecture` is the host's own or `all`, preferring the host's own; a package with no such entry is not installed.

| `Status` of the entry | Evidence |
| --- | --- |
| `installed`, `triggers-awaited`, or `triggers-pending`, with no error flag | `installed`, at its `Version` |
| `not-installed` or `config-files`, or no entry | `not-installed` |
| `half-installed`, `unpacked`, or `half-configured`, or an error flag such as `reinstreq` | `broken` |
| The database cannot be read | Failed, as the error maps |

**The lock.** While another package manager holds dpkg's lock, the database may be between two states of one transaction, and what it says of a package is not evidence. Before reading, the adapter looks for a lock on `/var/lib/dpkg/lock-frontend` or `/var/lib/dpkg/lock` in the kernel's table of file locks, `/proc/locks` beneath the root, which it reads and never takes, so it never makes another package manager fail. It looks again every 100 milliseconds for at most 5 seconds; a lock still held then makes the collection Failed, timed out, and the package Indeterminate: never a false Variance.

**Mutation.** Through `apt-get`, run by a direct `execve` of `/usr/bin/apt-get` (ADR 0013 §4) with an environment of only `PATH=/usr/sbin:/usr/bin:/sbin:/bin`, `LC_ALL=C`, and `DEBIAN_FRONTEND=noninteractive`, standard input from `/dev/null`, and no shell. Packages are managed only on the host itself: a host whose root is not `/` refuses every package operation.

| Requirement | Command |
| --- | --- |
| `installed`, no version | `apt-get install --yes --no-install-recommends` with the name, when the package is not installed or is broken |
| `installed` at a version | `apt-get install --yes --no-install-recommends --allow-downgrades` with `name=version`, when another version or none is installed, or the package is broken |
| `absent` | `apt-get remove --yes` with the name, when the package is installed or broken; its configuration files stay, as the family's `not-installed` allows |

Every mutating command also carries `-o DPkg::Lock::Timeout=60`, so it waits for another package manager rather than failing at once, and `-o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold`, so a changed configuration file is kept without a question. Nomos does not refresh the package index; the distribution's own timer does, and a version the index does not offer is refused.

**Refusal before effect.** Each command is first run with `--simulate`, which takes no lock and changes nothing. The adapter refuses (S7) when the simulation fails, as it does for a package or version the index does not offer, or when it would remove any package the requirement does not name: installing a package that conflicts with another, or removing one that others depend on, would delete what the Canon did not describe. A package whose collection failed is refused too.

**Settlement.** `apt-get` returns when dpkg has finished. The receipt is `Completed`, `changed` when the package's evidence after differs from before, when `apt-get` exits with status 0; `Failed` otherwise, whatever it did, and the package is read again from a new Observation.

**Testing.** Observation and the lock run here against a scratch root holding a status database and a table of locks. The package suite changes the host's packages, so it runs only in the Debian container, against a local repository the test builds: dummy packages at versions `1.0-1` and `2.0-1`, built with `dpkg-deb`, with a hand-written index, trusted as a `file:` source. The lock is held there by the test itself, with a POSIX record lock on `lock-frontend`, the kind dpkg takes.

## Failure Injection on Linux

Beyond the suite, the Linux adapter is tested against what a real file system can do to it: a symbolic link at the resource and one in a parent directory, a directory where a file is expected, a foreign writer that changes the file between an execution and its verification, and a read denied by file permissions. A test that must deny a read to a process with the capability to override file permissions drops that capability for its own thread first.

## Known Gaps

- **Supplementary groups, access control lists, and extended attributes.** Not managed or compared.
- **Privilege separation.** The mutation helper of spec §55 is Phase 5; the adapter here runs with the test's privileges.
- **Crash consistency of the rename.** The sequence follows spec §11, and a crash between its steps is not tested.
