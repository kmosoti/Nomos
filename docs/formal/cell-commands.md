# Cell Commands

- **Status.** Working definition for milestone `14-cell-commands`, written before the code (2026-09-30).
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

Every command takes `--state <dir>`, the Cell's state directory, `/var/lib/nomos` by default. It holds the content store, `content/`, and the journal, `journal` (ADR 0017). The host is always `/`: the Linux Substrate, with the Cell's content store as its content source and a connection to systemd on the system bus when one can be made. Without a bus, units are unsupported, so a Canon naming one ends Indeterminate rather than guessed.

An artifact is decoded with the host reader, schema 1 to 3 and every kind but the legacy `service` ([canon-ir.md](canon-ir.md)). An artifact the reader refuses is an error, never a partial Canon.

## Trace

`trace` runs the pipeline of `enforce` with execution left out (spec §37). It restores the Cell's snapshot from the journal, as `enforce` does, so that a refresh owed from an earlier run is shown; then it steps a Tick and the Enforce input, answers the kernel's request for Observations through the observe capability alone, and steps them. The Decision that comes back is the one `enforce` would act on. `trace` prints it and stops: the Apply requests it holds are never issued, and nothing is appended to the journal. N1 holds by construction, since the Linux Substrate is handed to it as `Observe` only.

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

`enforce` imports the bundle, if one is named, into the content store before anything else; a bundle refused whole stops the command. It opens the journal; a journal that fails to open stops the command, and a torn tail it truncated is reported. When the journal already holds inputs, the first input is `Recovered`, so that effects in flight when the last Cell stopped are settled as unknown (ADR 0010). Then it steps a Tick of the host's monotonic clock, since the kernel's clock starts at zero, and the Enforce input with the Plan below, and handles every input the Decisions lead to until none is left. While the run has not ended, it steps a Tick every second, for at most the Plan's timeouts.

| Plan field | Value |
| --- | --- |
| `id` | `cell` |
| `generation` | One more than the generation the journal's fence last accepted, or 1 |
| `bound` | 8 iterations |
| `policy` | 8 Actions at once; receipts due within 300 seconds, verification within 300, effects settled 300 seconds after dispatch |

It prints the outcome, and, for each Action the last round executed, its resource and stage.

## Import and Events

`import` imports a bundle into the content store and prints how many blobs it holds; a bundle with a misnamed or changed file is refused whole ([ADR 0017](../adr/0017-local-store.md) §2).

`events` prints the Event Log the journal stands for, one Event per line, numbered from 1, with its kind and the resource or Plan it names. A Plan is printed by its id and generation, never its Canon.

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

## Exit Status

| Status | `trace` | `enforce` |
| --- | --- | --- |
| 0 | Every Condition Satisfied, no Action planned | `Converged` |
| 1 | An error: usage, the artifact, the bundle, the state | the same |
| 2 | Some Condition Indeterminate | `Indeterminate` |
| 3 | Some Variance or Action, none Indeterminate | `NonConvergent` |
| 4 | | `Failed` |

`enforce` ends `Converged`, `Indeterminate`, `NonConvergent`, or `Failed` exactly as [reconciliation.md](reconciliation.md) defines them; the status is the outcome's, never a judgment of the binary's. A run the command stopped ticking before it ended is an error, status 1, and says so. `traits`, `import`, and `events` exit 0, or 1 on an error.
