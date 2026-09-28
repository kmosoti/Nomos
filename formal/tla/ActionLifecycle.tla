--------------------------- MODULE ActionLifecycle ---------------------------
(***************************************************************************)
(* The lifecycle of one Action and the reservation its effect holds.       *)
(*                                                                         *)
(* Spec section 18 gives the stages; N6 says Succeeded needs a verified    *)
(* postcondition; N10 says a timed-out outcome stays unknown; warp.md says *)
(* the Action's conflict keys are held from dispatch until its effect is   *)
(* Settled and the Action is no longer live. Settlement evidence is a      *)
(* terminal receipt, or the settle-by deadline past which the Substrate    *)
(* guarantees the effect can cause no further change (ADR 0006 note).     *)
(*                                                                         *)
(* Mutation selects a known-bad transition for the negative controls:      *)
(*   "none"              the model                                         *)
(*   "shortcut"          Running goes straight to Succeeded (breaks N6)    *)
(*   "timeout-fails"     a deadline records Failed (breaks N10)            *)
(*   "release-on-timeout" a deadline releases the reservation (breaks the  *)
(*                        reservation property)                            *)
(***************************************************************************)
EXTENDS TLC

CONSTANT Mutation

VARIABLES stage, verified, effect, reserved, last

vars == <<stage, verified, effect, reserved, last>>

Live == {"Dispatched", "Accepted", "Running", "Verifying"}
Terminal == {"Succeeded", "Failed", "TimedOut", "Cancelled", "Rejected"}
Stages == {"Prepared"} \cup Live \cup Terminal

TypeOK ==
    /\ stage \in Stages
    /\ verified \in BOOLEAN
    /\ effect \in {"None", "Unsettled", "Settled"}
    /\ reserved \in BOOLEAN
    /\ last \in STRING

Init ==
    /\ stage = "Prepared"
    /\ verified = FALSE
    /\ effect = "None"
    /\ reserved = FALSE
    /\ last = "Init"

\* The kernel dispatches: the effect request is issued and the keys are held.
Dispatch ==
    /\ stage = "Prepared"
    /\ stage' = "Dispatched"
    /\ effect' = "Unsettled"
    /\ reserved' = TRUE
    /\ UNCHANGED verified
    /\ last' = "Dispatch"

Cancel ==
    /\ stage = "Prepared"
    /\ stage' = "Cancelled"
    /\ UNCHANGED <<verified, effect, reserved>>
    /\ last' = "Cancel"

Accept ==
    /\ stage = "Dispatched"
    /\ stage' = "Accepted"
    /\ UNCHANGED <<verified, effect, reserved>>
    /\ last' = "Accept"

Start ==
    /\ stage = "Accepted"
    /\ stage' = "Running"
    /\ UNCHANGED <<verified, effect, reserved>>
    /\ last' = "Start"

\* A completion receipt: the effect is Settled, and the Action verifies next.
Complete ==
    /\ stage = "Running"
    /\ stage' = IF Mutation = "shortcut" THEN "Succeeded" ELSE "Verifying"
    /\ effect' = "Settled"
    /\ UNCHANGED <<verified, reserved>>
    /\ last' = "Complete"

\* A fresh Observation satisfies the Condition.
VerifyHolds ==
    /\ stage = "Verifying"
    /\ stage' = "Succeeded"
    /\ verified' = TRUE
    /\ UNCHANGED <<effect, reserved>>
    /\ last' = "VerifyHolds"

\* A fresh Observation shows a Variance.
VerifyFails ==
    /\ stage = "Verifying"
    /\ stage' = "Failed"
    /\ UNCHANGED <<verified, effect, reserved>>
    /\ last' = "VerifyFails"

\* The Observation is Indeterminate: nothing is established, the stage holds.
VerifyUnknown ==
    /\ stage = "Verifying"
    /\ UNCHANGED <<stage, verified, effect, reserved>>
    /\ last' = "VerifyUnknown"

\* A failure receipt: the outcome is known and the effect is Settled.
Fail ==
    /\ stage \in {"Accepted", "Running"}
    /\ stage' = "Failed"
    /\ effect' = "Settled"
    /\ UNCHANGED <<verified, reserved>>
    /\ last' = "Fail"

\* The Substrate refused the request: no effect started.
Refuse ==
    /\ stage \in {"Dispatched", "Accepted"}
    /\ stage' = "Rejected"
    /\ effect' = "Settled"
    /\ UNCHANGED <<verified, reserved>>
    /\ last' = "Refuse"

\* The receipt or verification deadline passed with the outcome unknown.
Deadline ==
    /\ stage \in Live
    /\ stage' = IF Mutation = "timeout-fails" THEN "Failed" ELSE "TimedOut"
    /\ reserved' = IF Mutation = "release-on-timeout" THEN FALSE ELSE reserved
    /\ UNCHANGED <<verified, effect>>
    /\ last' = "Deadline"

\* A receipt that arrives after the Action timed out settles the effect and
\* changes nothing else: the Action's outcome stays unknown.
LateReceipt ==
    /\ stage = "TimedOut"
    /\ effect = "Unsettled"
    /\ effect' = "Settled"
    /\ UNCHANGED <<stage, verified, reserved>>
    /\ last' = "LateReceipt"

\* The settle-by deadline passed: the effect can cause no further change.
SettleBy ==
    /\ stage \in Terminal
    /\ effect = "Unsettled"
    /\ effect' = "Settled"
    /\ UNCHANGED <<stage, verified, reserved>>
    /\ last' = "SettleBy"

\* Keys are released once the effect is Settled and the Action is not live.
Release ==
    /\ reserved
    /\ effect = "Settled"
    /\ stage \in Terminal
    /\ reserved' = FALSE
    /\ UNCHANGED <<stage, verified, effect>>
    /\ last' = "Release"

Next ==
    \/ Dispatch \/ Cancel \/ Accept \/ Start \/ Complete
    \/ VerifyHolds \/ VerifyFails \/ VerifyUnknown
    \/ Fail \/ Refuse \/ Deadline \/ LateReceipt \/ SettleBy \/ Release

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
(* Properties *)

\* N6: Succeeded only with a verified postcondition.
SucceededIsVerified == stage = "Succeeded" => verified

\* N10: a timed-out Action is never later taken for Succeeded or Failed.
TimedOutStaysUnknown == [][stage = "TimedOut" => stage' = "TimedOut"]_vars

\* N10: a deadline establishes neither success nor failure.
DeadlineEstablishesNothing ==
    [][(stage \in Live /\ stage' \notin Live /\ last' = "Deadline")
          => stage' = "TimedOut"]_vars

\* Reservation: an unsettled effect, or a live Action, holds its keys.
UnsettledIsReserved == (effect = "Unsettled" \/ stage \in Live) => reserved
=============================================================================
