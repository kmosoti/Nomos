# Cell Commands

- **Status.** Working definition for milestone `14-cell-commands`, written before the code (2026-09-30). The state directory sections were added by `17-alpha2-state-ownership` (issues #30, #36, and #37), also before the code.
- **Governs.** The `nomos-cell` binary: its commands, its state on disk, what each command prints, and its exit status. Spec §4 (Traits), §37 (Trace), §38 (Enforce), §39 (the fixed point), and §51 (CLI).

The binary is the composition root of the Cell ([hexagon.md](../architecture/hexagon.md)). It decides nothing a kernel decides: every Assessment is core's, every Plan the transition kernel's, and every Event is computed by stepping the journal ([ADR 0017](../adr/0017-local-store.md) §3). What it adds is the wiring, the state directory, and the text an operator reads.

## Commands

```sh
nomos-cell traits
nomos-cell trace   --canon <artifact.cbor>
nomos-cell enforce --canon <artifact.cbor> [--bundle <dir>]
nomos-cell import  --bundle <dir>
nomos-cell events
```

Every command takes `--state <dir>`, the Cell's state directory, `/var/lib/nomos` by default. It holds the content store, `content/`, the journal, `journal` (ADR 0017), and the lease file, `lock`. [State Directory](#state-directory) says who may own it and what makes it safe to trust. The host is always `/`: the Linux Substrate, with the Cell's content store as its content source and a connection to systemd on the system bus when one can be made. Without a bus, units are unsupported, so a Canon naming one ends Indeterminate rather than guessed.

An artifact is decoded with the host reader, schema 1 to 3 and every kind but the legacy `service` ([canon-ir.md](canon-ir.md)). An artifact the reader refuses is an error, never a partial Canon.

## Trace

`trace` runs the pipeline of `enforce` with execution left out (spec §37). It restores the Cell's snapshot from the journal, as `enforce` does, so that a refresh owed from an earlier run is shown; then it steps a Tick and the Enforce input, answers the kernel's request for Observations through the observe capability alone, and steps them. The Decision that comes back is the one `enforce` would act on. `trace` prints it and stops: the Apply requests it holds are never issued, and nothing is created, written, or truncated in the state directory. N1 holds by construction, since the Linux Substrate is handed to it as `Observe` only, and the state directory is opened for reading only ([Read-Only Commands](#read-only-commands)).

For each Condition, in the Canon's order, it prints the resource, its Assessment, and the Action planned for it:

```text
file:/etc/app/app.conf
  VARIANCE content-differs
  action: converge

unit:app.service
  SATISFIED
  action: refresh (owed)

file:/etc/app/secret.conf
  INDETERMINATE collection-failed: permission-denied
  action: none
```

An Indeterminate resource has no Action: the kernel plans none for it (N13). The last line counts the Assessments and the Actions.

## Enforce

`enforce` first takes mutation authority over the state directory ([Ownership](#ownership)); a command that cannot stops there. It imports the bundle, if one is named, into the content store; a bundle refused whole stops the command. It opens the journal; a journal that fails to open stops the command, and a torn tail it truncated is reported. When the journal already holds inputs, the first input is `Recovered`, so that effects in flight when the last Cell stopped are settled as unknown (ADR 0010). Then it steps a Tick of the host's monotonic clock, since the kernel's clock starts at zero, and the Enforce input with the Plan below, and handles every input the Decisions lead to until none is left. While the run has not ended, it steps a Tick every second, for at most the Plan's timeouts.

| Plan field | Value |
| --- | --- |
| `id` | `cell` |
| `generation` | One more than the generation the journal's fence last accepted, or 1 |
| `bound` | 8 iterations |
| `policy` | 8 Actions at once; receipts due within 300 seconds, verification within 300, effects settled 300 seconds after dispatch |

It prints the outcome, and, for each Action the last round executed, its resource and stage.

## Import and Events

`import` takes mutation authority over the state directory, imports a bundle into the content store, and prints how many blobs it holds; a bundle with a misnamed or changed file is refused whole ([ADR 0017](../adr/0017-local-store.md) §2).

`events` prints the Event Log the journal stands for, one Event per line, numbered from 1, with its kind and the resource or Plan it names. A Plan is printed by its id and generation, never its Canon. It is a read-only command.

## Traits

A Trait is a value with its provenance, the instant it was observed, and its stability (spec §4). `traits` prints each Trait the Linux host reports, one per line, as `key = value (stability, source)`.

| Key | Source | Stability |
| --- | --- | --- |
| `machine.id` | `/etc/machine-id` | Identity |
| `os.id`, `os.version_id` | `/etc/os-release` | Stable |
| `architecture` | the adapter's build target, as dpkg names it | Stable |
| `hostname` | `/proc/sys/kernel/hostname` | Stable |
| `kernel.release` | `/proc/sys/kernel/osrelease` | Dynamic |
| `memory.total` | `/proc/meminfo`, in kibibytes | Stable |
| `memory.available` | `/proc/meminfo`, in kibibytes | Ephemeral |

A source the host cannot read gives no Trait, and the command says which were missing. Traits are read, never inferred: nothing a Canon says about the host changes them.

## State Directory

The state directory is the Cell's control plane: the journal decides which Obligations are owed, and the content store supplies the bytes `enforce` writes to the host. A Cell that trusts a state directory someone else can write, or that two processes mutate at once, is no longer the only author of its decisions. Three rules follow, each checked before the journal is replayed.

### Ownership

At most one process holds **mutation authority** over a state directory. The lease is an exclusive advisory lock, `flock(LOCK_EX | LOCK_NB)`, on the file `lock` in the state directory.

- `enforce` and `import` take it before anything else: before the journal is replayed, content is imported, a Plan is accepted, or a Substrate effect is issued. They hold it until the process ends.
- A process that cannot take it ends with status 1, having appended nothing, imported nothing, and issued no effect. The message names the state directory as already in use.
- The lock belongs to the open file, so the kernel releases it when its holder dies, however it dies. No stale lock is ever cleaned up by hand, and the file `lock` is never removed: removing it would let two processes lock two different files.
- Different state directories are independent. The service and a command run by hand take the same lease, so they exclude each other.
- The lease excludes other Cell processes. It does not stop a user with write access to the directory, which the next section addresses, and it is not fencing between Cells on different machines.

`FileLog` keeps the length of the journal it replayed, and an append that fails truncates back to it. That is correct for one writer and wrong for two, which is why the lease precedes the journal's open.

### Read-Only Commands

`trace` and `events` take no lease. Each opens what it needs for reading only, and never creates, writes, truncates, or repairs anything in the state directory:

- An absent state directory or journal is an empty one, and stays absent.
- A final record cut short is reported on standard error as cut short, with its length, and left in place. The batch it belonged to was never acknowledged, so the snapshot is computed without it. `enforce` truncates it, as before.
- A record that fails its checksum or cannot be decoded is an error, status 1, naming the journal and the offset. It is journal damage, never a Variance.
- A read that runs beside an `enforce` may see a final record the writer has not finished or not yet acknowledged. It is read as cut short or as acknowledged, and either is a consistent prefix of the journal.

### Safety

A state directory is trusted only if it is private to the Cell's user. Every command that reads or writes the state checks this first, with the same rule for the packaged `/var/lib/nomos` and for a path given by `--state`:

| Object | Required | Created with |
| --- | --- | --- |
| The state directory | A directory, not a symbolic link; owned by the process's effective user; no permission bit for group or others | Mode `0700`, set explicitly so the umask cannot weaken it |
| `lock`, `journal` | A regular file, not a symbolic link; owned by the effective user; no permission bit for group or others | Mode `0600` |
| `content/` and its directories | The same as the state directory | Mode `0700` |
| A blob in the content store | The same as the journal | Mode `0600` |

An object that does not meet the rule, whether by its type, a link in its place, its owner, or its mode, is refused with an error naming the path and what is wrong, before the journal is replayed and before anything is written. The Cell does not repair a mode: it cannot know who has already read or written the object.

Only the last component of the state directory is created. The directories above it belong to the operator and are not checked; that is an assumption of the [security model](../security-model.md), not a property the Cell establishes.

## Exit Status

| Status | `trace` | `enforce` |
| --- | --- | --- |
| 0 | Every Condition Satisfied, no Action planned | `Converged` |
| 1 | An error: usage, the artifact, the bundle, the state, a state directory in use or unsafe, a damaged journal | the same |
| 2 | Some Condition Indeterminate | `Indeterminate` |
| 3 | Some Variance or Action, none Indeterminate | `NonConvergent` |
| 4 | | `Failed` |

`enforce` ends `Converged`, `Indeterminate`, `NonConvergent`, or `Failed` exactly as [reconciliation.md](reconciliation.md) defines them; the status is the outcome's, never a judgment of the binary's. A run the command stopped ticking before it ended is an error, status 1, and says so. `traits`, `import`, and `events` exit 0, or 1 on an error.
