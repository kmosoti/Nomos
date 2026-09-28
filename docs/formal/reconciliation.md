# Reconciliation

## Model

For each managed resource $r$, a driver provides (spec §9):

$$
\begin{aligned}
O_r &= \mathrm{observe}(r) \\
V_r &= \mathrm{diff}(D_r, O_r) \\
A_r &= \mathrm{plan}(V_r) \\
&\phantom{=}\ \mathrm{apply}(A_r) \\
&\phantom{=}\ \mathrm{verify}(D_r, O'_r)
\end{aligned}
$$

**Soundness requirement on `diff`.** For every driver:

$$
\mathrm{diff}(D, O) = \varnothing \iff O \models D
$$

Here $O \models D$ means "the observation satisfies the desired spec". Every correctness result below rests on this requirement, so every driver tests it directly.

**Plan purity.** $\mathrm{plan}$ is a function of Variance only, and $\mathrm{plan}(\varnothing) = \emptyset$.

## Algorithm

```text
enforce(C, bound k):
    history := ∅
    for i in 1..=k:
        O := observe(C)
        V := diff(C, O)
        if V = ∅:            return Converged
        h := H(O)
        if h ∈ history:      return NonConvergent(oscillation)
        history := history ∪ {h}
        P := warp(plan(V))
        execute(P)           # each Action: apply, then verify
        if a hard failure:   return Failed
    return NonConvergent(bound)
```

Trace is the same procedure with `execute` removed and the loop run once (spec §37).

```mermaid
flowchart LR
    O["observe"] --> D{"V = ∅?"}
    D --> C(["yes<br/>Converged"])
    D --> Hs{"no<br/>H(O) seen?"}
    Hs --> N1(["seen<br/>NonConvergent: oscillation"])
    Hs --> P["new state<br/>plan + Warp"] --> X["execute + verify"] --> B{"i < k?"}
    B --> Loop["i < k<br/>next iteration"] --> O
    B --> N2(["i = k<br/>NonConvergent: bound"])
```

## Fixed Point (N3)

**Claim.** If $\mathrm{Enforce}(C, S) = S'$ with $\mathrm{diff}(C, S') = \varnothing$, then $\mathrm{Enforce}(C, S')$ performs no mutating Action.

*Proof sketch.* In state $S'$, the first iteration observes $O = S'$, computes $\mathrm{diff}(C, S') = \varnothing$ and returns `Converged` before `plan` or `execute` is reached. No Action is produced, so none mutates. $\square$

The property test is therefore $\mathrm{enforce}(\mathrm{enforce}(S)) = \mathrm{enforce}(S)$, with zero mutating Actions in the second run.

## Termination

**Claim.** `enforce` terminates after at most $k$ iterations.

*Proof.* The loop variable is bounded by $k$. Every exit path either returns or increments $i$. $\square$

Oscillation detection is a sharper early exit. Suppose an external writer $B$ restores a previous state $X$ every time Nomos changes it. Some later observation then repeats an earlier one, so $H(O_j) = H(O_i)$ for some $j > i$, and Nomos reports oscillation instead of running to the bound. Oscillation that never repeats an exact state still stops at $k$. Nomos may lose an argument with another controller. It will not lose it forever.

## Verification (N6)

An Action reaches `Succeeded` only through `Verifying`. `Verifying` succeeds only if $\mathrm{verify}(D_r, O'_r)$ holds on a fresh observation. An exit code is never evidence.
