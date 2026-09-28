--------------------------- MODULE RefreshRecovery ---------------------------
(***************************************************************************)
(* One configuration write, the service refresh its change requires, the  *)
(* reservation each effect holds, an Obligation, crashes, and superseding  *)
(* authority. The grounding plan's first model: it checks                 *)
(*                                                                         *)
(*   admission safety       conflicting effects never both proceed         *)
(*   uncertainty            a deadline or a crash settles nothing          *)
(*   recovery safety        a crash cannot lose a required refresh         *)
(*   completion soundness   Converged needs Satisfied Conditions, no       *)
(*                          pending Obligation, and Settled effects        *)
(*                                                                         *)
(* The service cannot report the configuration it loaded, so the refresh   *)
(* is an Obligation recorded before the write is dispatched (ADR 0009      *)
(* section 4). A crash drops receipts and in-memory activation; the        *)
(* Obligation, the effect records, and the Action stages are in the Event  *)
(* Log and survive, and a live Action becomes TimedOut on recovery.        *)
(* The kernel observes, plans, and verifies; the model collapses those     *)
(* into the guards below.                                                  *)
(*                                                                         *)
(* Mutation selects a negative control:                                    *)
(*   "none"                  the model                                     *)
(*   "transient"             no Obligation, only in-memory activation      *)
(*   "release-on-timeout"    a timed-out write no longer holds its key     *)
(*   "discharge-on-dispatch" the Obligation is cleared when the refresh is *)
(*                           dispatched rather than when it is verified    *)
(***************************************************************************)
EXTENDS Naturals, TLC

CONSTANTS Mutation, MaxGen, MaxCrashes

VARIABLES
    disk,            \* configuration revision on disk
    loaded,          \* configuration revision the service loaded
    write,           \* this run's write Action
    writeEffect,     \* the write's effect: None, Unsettled, Settled
    writeReceipt,    \* a write completion receipt in flight
    refresh,         \* this run's refresh Action
    refreshEffect,
    refreshReceipt,
    other,           \* an independent Action sharing the write's conflict key
    obligation,      \* durable: a refresh is owed
    activated,       \* volatile: this run saw the write change the file
    gen,             \* the accepted authority generation
    crashes,
    outcome,         \* the current run
    last

vars == <<disk, loaded, write, writeEffect, writeReceipt, refresh,
          refreshEffect, refreshReceipt, other, obligation, activated,
          gen, crashes, outcome, last>>

Effects == {"None", "Unsettled", "Settled"}

TypeOK ==
    /\ disk \in {"old", "new"}
    /\ loaded \in {"old", "new"}
    /\ write \in {"Idle", "Dispatched", "Succeeded", "TimedOut"}
    /\ writeEffect \in Effects
    /\ writeReceipt \in BOOLEAN
    /\ refresh \in {"Idle", "Dispatched", "Succeeded", "TimedOut"}
    /\ refreshEffect \in Effects
    /\ refreshReceipt \in BOOLEAN
    /\ other \in {"Idle", "Running", "Done"}
    /\ obligation \in BOOLEAN
    /\ activated \in BOOLEAN
    /\ gen \in 1..MaxGen
    /\ crashes \in 0..MaxCrashes
    /\ outcome \in {"Running", "Converged", "Failed"}

Init ==
    /\ disk = "old"
    /\ loaded = "old"
    /\ write = "Idle"
    /\ writeEffect = "None"
    /\ writeReceipt = FALSE
    /\ refresh = "Idle"
    /\ refreshEffect = "None"
    /\ refreshReceipt = FALSE
    /\ other = "Idle"
    /\ obligation = FALSE
    /\ activated = FALSE
    /\ gen = 1
    /\ crashes = 0
    /\ outcome = "Running"
    /\ last = "Init"

\* The configuration file's conflict key is held by a live write, by an
\* unsettled write effect, and by the refresh, whose footprint includes the
\* file it reads.
KeyHeld ==
    \/ write = "Dispatched"
    \/ (writeEffect = "Unsettled" /\ Mutation # "release-on-timeout")
    \/ refresh = "Dispatched"
    \/ refreshEffect = "Unsettled"

\* The write is met for the refresh's on_change group: it succeeded, or the
\* file already held the desired revision and the write had nothing to do.
WriteMet == write = "Succeeded" \/ (write = "Idle" /\ disk = "new")

DispatchWrite ==
    /\ outcome = "Running"
    /\ write = "Idle"
    /\ disk = "old"
    /\ ~KeyHeld
    /\ other # "Running"
    /\ write' = "Dispatched"
    /\ writeEffect' = "Unsettled"
    /\ obligation' = (obligation \/ Mutation # "transient")
    /\ UNCHANGED <<disk, loaded, writeReceipt, refresh, refreshEffect,
                   refreshReceipt, other, activated, gen, crashes, outcome>>
    /\ last' = "DispatchWrite"

HostWrites ==
    /\ writeEffect = "Unsettled"
    /\ ~writeReceipt
    /\ disk' = "new"
    /\ writeReceipt' = TRUE
    /\ UNCHANGED <<loaded, write, writeEffect, refresh, refreshEffect,
                   refreshReceipt, other, obligation, activated, gen,
                   crashes, outcome>>
    /\ last' = "HostWrites"

\* The completion receipt settles the effect; verification of a live write
\* follows it and is folded in here.
WriteReceipt ==
    /\ writeReceipt
    /\ writeReceipt' = FALSE
    /\ writeEffect' = "Settled"
    /\ write' = IF write = "Dispatched" THEN "Succeeded" ELSE write
    /\ activated' = (activated \/ write = "Dispatched")
    /\ UNCHANGED <<disk, loaded, refresh, refreshEffect, refreshReceipt,
                   other, obligation, gen, crashes, outcome>>
    /\ last' = "WriteReceipt"

\* A deadline passes with the outcome unknown: nothing settles.
WriteDeadline ==
    /\ write = "Dispatched"
    /\ write' = "TimedOut"
    /\ UNCHANGED <<disk, loaded, writeEffect, writeReceipt, refresh,
                   refreshEffect, refreshReceipt, other, obligation,
                   activated, gen, crashes, outcome>>
    /\ last' = "Deadline"

\* Past the settle-by deadline the Substrate guarantees the effect can cause
\* no further change; a write that has not happened never will.
WriteSettleBy ==
    /\ writeEffect = "Unsettled"
    /\ write # "Dispatched"
    /\ writeEffect' = "Settled"
    /\ UNCHANGED <<disk, loaded, write, writeReceipt, refresh, refreshEffect,
                   refreshReceipt, other, obligation, activated, gen,
                   crashes, outcome>>
    /\ last' = "SettleBy"

DispatchRefresh ==
    /\ outcome = "Running"
    /\ refresh = "Idle"
    /\ ~KeyHeld
    /\ other # "Running"
    /\ WriteMet
    /\ obligation \/ activated
    /\ refresh' = "Dispatched"
    /\ refreshEffect' = "Unsettled"
    /\ obligation' = IF Mutation = "discharge-on-dispatch" THEN FALSE
                     ELSE obligation
    /\ UNCHANGED <<disk, loaded, write, writeEffect, writeReceipt,
                   refreshReceipt, other, activated, gen, crashes, outcome>>
    /\ last' = "DispatchRefresh"

HostRestarts ==
    /\ refreshEffect = "Unsettled"
    /\ ~refreshReceipt
    /\ loaded' = disk
    /\ refreshReceipt' = TRUE
    /\ UNCHANGED <<disk, write, writeEffect, writeReceipt, refresh,
                   refreshEffect, other, obligation, activated, gen,
                   crashes, outcome>>
    /\ last' = "HostRestarts"

\* A verified refresh discharges the Obligation recorded before it was
\* dispatched; nothing else does.
RefreshReceipt ==
    /\ refreshReceipt
    /\ refreshReceipt' = FALSE
    /\ refreshEffect' = "Settled"
    /\ refresh' = IF refresh = "Dispatched" THEN "Succeeded" ELSE refresh
    /\ obligation' = IF refresh = "Dispatched" THEN FALSE ELSE obligation
    /\ activated' = IF refresh = "Dispatched" THEN FALSE ELSE activated
    /\ UNCHANGED <<disk, loaded, write, writeEffect, writeReceipt, other,
                   gen, crashes, outcome>>
    /\ last' = "RefreshReceipt"

RefreshDeadline ==
    /\ refresh = "Dispatched"
    /\ refresh' = "TimedOut"
    /\ UNCHANGED <<disk, loaded, write, writeEffect, writeReceipt,
                   refreshEffect, refreshReceipt, other, obligation,
                   activated, gen, crashes, outcome>>
    /\ last' = "Deadline"

RefreshSettleBy ==
    /\ refreshEffect = "Unsettled"
    /\ refresh # "Dispatched"
    /\ refreshEffect' = "Settled"
    /\ UNCHANGED <<disk, loaded, write, writeEffect, writeReceipt, refresh,
                   refreshReceipt, other, obligation, activated, gen,
                   crashes, outcome>>
    /\ last' = "SettleBy"

DispatchOther ==
    /\ outcome = "Running"
    /\ other = "Idle"
    /\ ~KeyHeld
    /\ other' = "Running"
    /\ UNCHANGED <<disk, loaded, write, writeEffect, writeReceipt, refresh,
                   refreshEffect, refreshReceipt, obligation, activated,
                   gen, crashes, outcome>>
    /\ last' = "DispatchOther"

OtherDone ==
    /\ other = "Running"
    /\ other' = "Done"
    /\ UNCHANGED <<disk, loaded, write, writeEffect, writeReceipt, refresh,
                   refreshEffect, refreshReceipt, obligation, activated,
                   gen, crashes, outcome>>
    /\ last' = "OtherDone"

\* The process restarts. Receipts in flight and in-memory activation are
\* lost; the Event Log restores everything else, and recovery treats a live
\* Action as TimedOut (ADR 0012 section 1).
Crash ==
    /\ crashes < MaxCrashes
    /\ crashes' = crashes + 1
    /\ writeReceipt' = FALSE
    /\ refreshReceipt' = FALSE
    /\ activated' = FALSE
    /\ write' = IF write = "Dispatched" THEN "TimedOut" ELSE write
    /\ refresh' = IF refresh = "Dispatched" THEN "TimedOut" ELSE refresh
    /\ outcome' = IF outcome = "Running" THEN "Failed" ELSE outcome
    /\ UNCHANGED <<disk, loaded, writeEffect, refreshEffect, other,
                   obligation, gen>>
    /\ last' = "Crash"

\* A timed-out Action ends the run as a failure with an unknown outcome.
RunFails ==
    /\ outcome = "Running"
    /\ write = "TimedOut" \/ refresh = "TimedOut"
    /\ outcome' = "Failed"
    /\ UNCHANGED <<disk, loaded, write, writeEffect, writeReceipt, refresh,
                   refreshEffect, refreshReceipt, other, obligation,
                   activated, gen, crashes>>
    /\ last' = "RunFails"

\* The run observes: every Condition Satisfied, no Obligation, no in-run
\* activation pending, every effect Settled.
Converge ==
    /\ outcome = "Running"
    /\ disk = "new"
    /\ ~obligation
    /\ ~activated
    /\ writeEffect # "Unsettled"
    /\ refreshEffect # "Unsettled"
    /\ write # "Dispatched"
    /\ refresh # "Dispatched"
    /\ other # "Running"
    /\ outcome' = "Converged"
    /\ UNCHANGED <<disk, loaded, write, writeEffect, writeReceipt, refresh,
                   refreshEffect, refreshReceipt, other, obligation,
                   activated, gen, crashes>>
    /\ last' = "Converge"

\* A newer authority, or the next local run. Effects in flight keep running
\* and keep their reservations; this run's Actions start over.
NewRun ==
    /\ gen < MaxGen
    /\ gen' = gen + 1
    /\ outcome' = "Running"
    /\ write' = "Idle"
    /\ refresh' = "Idle"
    /\ activated' = FALSE
    /\ other' = IF other = "Running" THEN "Running" ELSE "Idle"
    /\ UNCHANGED <<disk, loaded, writeEffect, writeReceipt, refreshEffect,
                   refreshReceipt, obligation, crashes>>
    /\ last' = "NewRun"

Next ==
    \/ DispatchWrite \/ HostWrites \/ WriteReceipt \/ WriteDeadline
    \/ WriteSettleBy \/ DispatchRefresh \/ HostRestarts \/ RefreshReceipt
    \/ RefreshDeadline \/ RefreshSettleBy \/ DispatchOther \/ OtherDone
    \/ Crash \/ RunFails \/ Converge \/ NewRun

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
(* Properties *)

\* Admission safety: the conflicting Action never runs beside the write's
\* live or unsettled effect.
AdmissionSafety ==
    ~(other = "Running" /\ (write = "Dispatched" \/ writeEffect = "Unsettled"))

\* Completion soundness and recovery safety: Converged means the file holds
\* the desired revision, the service loaded it, no refresh is owed, and every
\* effect is Settled.
ConvergedIsSound ==
    outcome = "Converged" =>
        /\ disk = "new"
        /\ loaded = disk
        /\ ~obligation
        /\ writeEffect # "Unsettled"
        /\ refreshEffect # "Unsettled"

\* Uncertainty preservation: a deadline or a crash settles no effect.
UncertaintyPreserved ==
    [][last' \in {"Deadline", "Crash"} =>
          /\ writeEffect' = writeEffect
          /\ refreshEffect' = refreshEffect]_vars

\* An Obligation disappears only when a verified refresh discharges it.
ObligationDischargedOnlyByRefresh ==
    [][(obligation /\ ~obligation') => last' = "RefreshReceipt"]_vars
=============================================================================
