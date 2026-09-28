------------------------------- MODULE Fencing -------------------------------
(***************************************************************************)
(* Competing authority and generation acceptance at one Cell, for N5.      *)
(*                                                                         *)
(* Authorities present Plans with generations; delivery may delay,         *)
(* duplicate, and reorder them, so any Plan may arrive at any time. The    *)
(* Cell persists the highest accepted generation and the Plan accepted at  *)
(* it (fencing-and-idempotency.md). Each Plan has Actions to admit; an     *)
(* admitted Action's effect is recorded with the accepted generation at    *)
(* the moment the effect happens.                                          *)
(*                                                                         *)
(* ADR 0010 section 3: the generation is re-checked inside the one         *)
(* serialized admission path, atomically with effect admission. Mutation   *)
(* "split" separates the check from the effect, which is the fence-race    *)
(* counterexample; "stale-by-one" accepts a Plan one generation behind.    *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets, TLC

CONSTANTS Mutation, MaxGen, Ids, ActionsPerPlan

\* lastPlan names the Plan the last step acted on, so that a trace can be
\* replayed through the kernel's fence step by step.
VARIABLES gacc, planAt, checked, done, executed, last, lastPlan

vars == <<gacc, planAt, checked, done, executed, last, lastPlan>>

NoPlan == [g |-> 0, id |-> "none"]

Plans == [g : 1..MaxGen, id : Ids]

TypeOK ==
    /\ gacc \in 0..MaxGen
    /\ planAt \in [1..MaxGen -> Ids \cup {"none"}]
    /\ checked \subseteq Plans
    /\ done \in [Plans -> 0..ActionsPerPlan]
    /\ executed \subseteq (Plans \X (0..MaxGen))
    /\ last \in STRING
    /\ lastPlan \in Plans \cup {NoPlan}

Init ==
    /\ gacc = 0
    /\ planAt = [g \in 1..MaxGen |-> "none"]
    /\ checked = {}
    /\ done = [p \in Plans |-> 0]
    /\ executed = {}
    /\ last = "Init"
    /\ lastPlan = NoPlan

StaleBound == IF Mutation = "stale-by-one" THEN gacc - 1 ELSE gacc

\* on_plan: reject a stale generation or a conflicting Plan at the accepted
\* generation; otherwise persist the generation and the Plan before anything
\* executes.
Accept(p) ==
    /\ p.g >= StaleBound
    /\ ~(p.g = gacc /\ planAt[p.g] # "none" /\ planAt[p.g] # p.id)
    /\ p.g >= 1
    /\ gacc' = IF p.g > gacc THEN p.g ELSE gacc
    /\ planAt' = [planAt EXCEPT ![p.g] = p.id]
    /\ UNCHANGED <<checked, done, executed>>
    /\ last' = "Accept"
    /\ lastPlan' = p

Current(p) == gacc = p.g /\ planAt[p.g] = p.id

\* The model: check and effect in one serialized admission step.
Admit(p) ==
    /\ Mutation # "split"
    /\ Current(p)
    /\ done[p] < ActionsPerPlan
    /\ done' = [done EXCEPT ![p] = @ + 1]
    /\ executed' = executed \cup {<<p, gacc>>}
    /\ UNCHANGED <<gacc, planAt, checked>>
    /\ last' = "Admit"
    /\ lastPlan' = p

\* Mutation "split": the check passes, then the effect happens later.
Check(p) ==
    /\ Mutation = "split"
    /\ Current(p)
    /\ done[p] < ActionsPerPlan
    /\ checked' = checked \cup {p}
    /\ UNCHANGED <<gacc, planAt, done, executed>>
    /\ last' = "Check"
    /\ lastPlan' = p

Effect(p) ==
    /\ Mutation = "split"
    /\ p \in checked
    /\ checked' = checked \ {p}
    /\ done' = [done EXCEPT ![p] = @ + 1]
    /\ executed' = executed \cup {<<p, gacc>>}
    /\ UNCHANGED <<gacc, planAt>>
    /\ last' = "Effect"
    /\ lastPlan' = p

Next ==
    \E p \in Plans : Accept(p) \/ Admit(p) \/ Check(p) \/ Effect(p)

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
(* Properties *)

\* N5: an effect executes only under a generation at least the accepted one.
StaleNeverExecutes == \A e \in executed : e[1].g >= e[2]

\* Lemma 1: the accepted generation never decreases.
Monotonic == [][gacc' >= gacc]_vars

\* N5 at acceptance: a Plan behind the accepted generation is never recorded.
NoStaleAcceptance ==
    [][\A g \in 1..MaxGen : planAt'[g] # planAt[g] => g >= gacc]_vars

\* Single active authority: one Plan per accepted generation.
OnePlanPerGeneration ==
    [][\A g \in 1..MaxGen :
          planAt[g] # "none" => planAt'[g] = planAt[g]]_vars
=============================================================================
