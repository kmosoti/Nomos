//! Experiment `kernel-conformance`, for the two models whose transitions
//! map one to one onto the transition rules of `nomos-core`
//! ([formal/tla/README.md](../../../../formal/tla/README.md)).
//!
//! The fixtures under `tests/fixtures/tla/` are TLC's own output: random
//! behaviors of each model, simulated from a fixed seed, and the
//! counterexample each negative-control configuration produces. Their
//! provenance is in that directory's README. Each trace is replayed here
//! through the production functions, step by step:
//!
//! - `ActionLifecycle` through [`advance`], [`signals_for`],
//!   [`Settlement`], and [`holds_reservation`];
//! - `Fencing` through [`Fence::accept`] and [`Fence::permits`], the latter
//!   at the moment of the effect, as the kernel checks it.
//!
//! A behavior of the model must replay with the kernel agreeing on every
//! variable after every step. A counterexample must diverge, because it is
//! a behavior the kernel's rules exclude. And every action of each model
//! must occur in the replayed behaviors, or the replay is vacuous for it.
//! This checks the kernel against the model within the model's bounds and
//! the simulated behaviors; it is not a proof of either.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use nomos_core::action::{Signal, Stage, Verdict, advance, holds_reservation, signals_for, verify};
use nomos_core::assessment::{Reason, Variance};
use nomos_core::condition::{Condition, Content, FileCondition};
use nomos_core::effect::{Receipt, Settlement};
use nomos_core::observation::{
    Collection, CollectorId, FileEvidence, Instant, Observation, Provenance, Window,
};
use nomos_core::plan::{Fence, Generation, PlanId};
use nomos_core::resource::{Digest, ResourcePath};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/tla")
}

/// One state of a trace: each variable's value, as TLC printed it.
type State = BTreeMap<String, String>;

/// Parses every state of a TLC trace, from a simulation dump (`STATE_n ==`)
/// or a counterexample on standard output (`State n:`).
fn parse(text: &str) -> Vec<State> {
    let mut states = Vec::new();
    let mut current: Option<State> = None;
    let mut var: Option<String> = None;
    for line in text.lines() {
        if line.starts_with("STATE_") || line.starts_with("State ") {
            states.extend(current.take());
            current = Some(State::new());
            var = None;
        } else if let (Some(state), Some(rest)) = (current.as_mut(), line.strip_prefix("/\\ ")) {
            let (name, value) = rest.split_once(" = ").expect("a variable line");
            state.insert(name.to_string(), value.to_string());
            var = Some(name.to_string());
        } else if line.trim().is_empty() {
            var = None;
            if let Some(state) = current.take() {
                states.push(state);
            }
        } else if let (Some(state), Some(name)) = (current.as_mut(), var.as_ref())
            && let Some(value) = state.get_mut(name)
        {
            value.push(' ');
            value.push_str(line.trim());
        }
    }
    states.extend(current);
    states
}

fn string(value: &str) -> &str {
    value.trim().trim_matches('"')
}

fn boolean(value: &str) -> bool {
    match value.trim() {
        "TRUE" => true,
        "FALSE" => false,
        other => panic!("not a Boolean: {other}"),
    }
}

/// A record such as `[g |-> 1, id |-> "a"]`.
fn record(value: &str) -> BTreeMap<String, String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(", ")
        .map(|field| {
            let (k, v) = field.split_once(" |-> ").expect("a record field");
            (k.trim().to_string(), string(v).to_string())
        })
        .collect()
}

/// A tuple of strings such as `<<"a", "none">>`.
fn tuple(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches("<<")
        .trim_end_matches(">>")
        .split(", ")
        .map(|v| string(v).to_string())
        .collect()
}

fn traces(dir: &str) -> Vec<(String, Vec<State>)> {
    let mut files: Vec<PathBuf> = fs::read_dir(fixtures().join(dir))
        .expect("the fixture directory")
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let text = fs::read_to_string(&f).unwrap();
            (f.display().to_string(), parse(&text))
        })
        .collect()
}

fn counterexample(name: &str) -> Vec<State> {
    let text = fs::read_to_string(fixtures().join(format!("{name}.out"))).unwrap();
    let states = parse(&text);
    assert!(states.len() >= 2, "{name}: no counterexample in the output");
    states
}

// ---------------------------------------------------------------------------
// ActionLifecycle

const LIFECYCLE_ACTIONS: [&str; 14] = [
    "Dispatch",
    "Cancel",
    "Accept",
    "Start",
    "Complete",
    "VerifyHolds",
    "VerifyFails",
    "VerifyUnknown",
    "Fail",
    "Refuse",
    "Deadline",
    "LateReceipt",
    "SettleBy",
    "Release",
];

fn stage_name(stage: &Stage) -> &'static str {
    match stage {
        Stage::Prepared => "Prepared",
        Stage::Dispatched => "Dispatched",
        Stage::Accepted => "Accepted",
        Stage::Running => "Running",
        Stage::Verifying { .. } => "Verifying",
        Stage::Succeeded { .. } => "Succeeded",
        Stage::Failed(_) => "Failed",
        Stage::TimedOut => "TimedOut",
        Stage::Cancelled => "Cancelled",
        Stage::Rejected => "Rejected",
    }
}

fn effect_name(settlement: &Option<Settlement>) -> &'static str {
    match settlement {
        None => "None",
        Some(s) if s.is_settled() => "Settled",
        Some(_) => "Unsettled",
    }
}

/// A verdict that holds, obtained the only way there is: `verify` on a
/// fresh Observation that satisfies the Condition.
fn holding() -> Verdict {
    let path = ResourcePath::new("/etc/model").unwrap();
    let condition = Condition::file(path.clone(), FileCondition::present(Content::Any));
    let observation = Observation::file(
        path,
        Collection::Collected(FileEvidence::present(Digest::from_bytes([0; 32]), 0)),
        Provenance::new(
            CollectorId::new("model").unwrap(),
            Window::new(Instant(0), Instant(0)).unwrap(),
        ),
    );
    verify(&condition, &[observation], Instant(0))
}

/// The lifecycle actions the model enables in a stage, transcribed from the
/// guards of `ActionLifecycle.tla`.
fn model_enabled(stage: &str) -> BTreeSet<&'static str> {
    let actions: &[&'static str] = match stage {
        "Prepared" => &["Dispatch", "Cancel"],
        "Dispatched" => &["Accept", "Refuse", "Deadline"],
        "Accepted" => &["Start", "Fail", "Refuse", "Deadline"],
        "Running" => &["Complete", "Fail", "Deadline"],
        "Verifying" => &["VerifyHolds", "VerifyFails", "VerifyUnknown", "Deadline"],
        _ => &[],
    };
    actions.iter().copied().collect()
}

/// The lifecycle actions the kernel accepts from `stage`.
fn kernel_enabled(stage: &Stage) -> BTreeSet<&'static str> {
    let signals: [(&'static str, Signal); 11] = [
        ("Dispatch", Signal::Dispatch),
        ("Cancel", Signal::Cancel),
        ("Accept", Signal::Accept),
        ("Start", Signal::Start),
        ("Complete", Signal::Complete { changed: true }),
        ("VerifyHolds", Signal::Verify(holding())),
        (
            "VerifyFails",
            Signal::Verify(Verdict::Fails(Variance::Missing)),
        ),
        (
            "VerifyUnknown",
            Signal::Verify(Verdict::Unknown(Reason::NoObservation)),
        ),
        ("Fail", Signal::Fail),
        ("Refuse", Signal::Refuse),
        ("Deadline", Signal::Deadline),
    ];
    signals
        .into_iter()
        .filter(|(_, signal)| advance(stage, signal.clone()).is_ok())
        .map(|(name, _)| name)
        .collect()
}

/// Whether the kernel's state agrees with the model's `state`: every
/// variable, the lifecycle actions each enables, and whether a reservation
/// may be released.
fn agree_lifecycle(
    i: usize,
    state: &State,
    stage: &Stage,
    settlement: &Option<Settlement>,
    reserved: bool,
) -> Result<(), String> {
    let action = string(&state["last"]);
    let agree = [
        ("stage", stage_name(stage) == string(&state["stage"])),
        (
            "verified",
            matches!(stage, Stage::Succeeded { .. }) == boolean(&state["verified"]),
        ),
        (
            "effect",
            effect_name(settlement) == string(&state["effect"]),
        ),
        ("reserved", reserved == boolean(&state["reserved"])),
    ];
    if let Some((name, _)) = agree.iter().find(|(_, ok)| !ok) {
        return Err(format!("step {i} {action}: the kernel disagrees on {name}"));
    }
    let model = model_enabled(string(&state["stage"]));
    let kernel = kernel_enabled(stage);
    if model != kernel {
        return Err(format!(
            "step {i} {action}: the kernel enables {kernel:?}, the model {model:?}"
        ));
    }
    if let Some(s) = settlement {
        let held = holds_reservation(Some(stage), s);
        if held && !reserved {
            return Err(format!(
                "step {i} {action}: a held reservation was released"
            ));
        }
        let model_release =
            reserved && string(&state["effect"]) == "Settled" && stage.is_terminal();
        if reserved && model_release == held {
            return Err(format!(
                "step {i} {action}: the kernel and the model disagree on release"
            ));
        }
    }
    Ok(())
}

/// Replays one lifecycle behavior; `Err` names the first step at which the
/// kernel disagrees with the model.
fn replay_lifecycle(states: &[State]) -> Result<(), String> {
    let mut stage = Stage::Prepared;
    let mut settlement: Option<Settlement> = None;
    let mut reserved = false;
    let settle_by = Instant(100);
    agree_lifecycle(0, &states[0], &stage, &settlement, reserved)?;
    for (i, state) in states.iter().enumerate().skip(1) {
        let action = string(&state["last"]);
        let mut apply = |signal: Signal| -> Result<(), String> {
            stage = advance(&stage, signal.clone())
                .map_err(|e| format!("step {i} {action}: {signal:?} refused: {e}"))?;
            Ok(())
        };
        match action {
            "Dispatch" => {
                apply(Signal::Dispatch)?;
                settlement = Some(Settlement::new(settle_by));
                reserved = true;
            }
            "Cancel" => apply(Signal::Cancel)?,
            "Accept" => apply(Signal::Accept)?,
            "Start" => apply(Signal::Start)?,
            "Complete" => {
                apply(Signal::Complete { changed: true })?;
                settlement =
                    settlement.map(|s| s.on_receipt(&Receipt::Completed { changed: true }));
            }
            "VerifyHolds" => apply(Signal::Verify(holding()))?,
            "VerifyFails" => apply(Signal::Verify(Verdict::Fails(Variance::Missing)))?,
            "VerifyUnknown" => apply(Signal::Verify(Verdict::Unknown(Reason::NoObservation)))?,
            "Fail" => {
                apply(Signal::Fail)?;
                settlement = settlement.map(|s| s.on_receipt(&Receipt::Failed));
            }
            "Refuse" => {
                apply(Signal::Refuse)?;
                settlement = settlement.map(|s| s.on_receipt(&Receipt::Refused));
            }
            "Deadline" => apply(Signal::Deadline)?,
            "LateReceipt" => {
                let late = Receipt::Completed { changed: true };
                if !signals_for(&stage, &late).is_empty() {
                    return Err(format!("step {i}: a late receipt moved the lifecycle"));
                }
                settlement = settlement.map(|s| s.on_receipt(&late));
            }
            "SettleBy" => settlement = settlement.map(|s| s.at(settle_by)),
            "Release" => {
                let Some(s) = &settlement else {
                    return Err(format!("step {i}: release without an effect"));
                };
                if holds_reservation(Some(&stage), s) {
                    return Err(format!("step {i}: released a reservation the rule holds"));
                }
                reserved = false;
            }
            other => return Err(format!("step {i}: unknown action {other}")),
        }
        agree_lifecycle(i, state, &stage, &settlement, reserved)?;
    }
    Ok(())
}

#[test]
fn every_simulated_lifecycle_behavior_replays() {
    let traces = traces("ActionLifecycle");
    assert_eq!(traces.len(), 150);
    let mut seen = BTreeSet::new();
    let mut steps = 0;
    for (file, states) in &traces {
        replay_lifecycle(states).unwrap_or_else(|e| panic!("{file}: {e}"));
        steps += states.len() - 1;
        seen.extend(states.iter().map(|s| string(&s["last"]).to_string()));
    }
    for action in LIFECYCLE_ACTIONS {
        assert!(seen.contains(action), "no replayed behavior takes {action}");
    }
    println!("ActionLifecycle: {} behaviors, {steps} steps", traces.len());
}

#[test]
fn every_lifecycle_counterexample_diverges() {
    for (name, at) in [
        ("ActionLifecycle-shortcut", "Complete"),
        ("ActionLifecycle-timeout-fails", "Deadline"),
        ("ActionLifecycle-release-on-timeout", "Deadline"),
    ] {
        let states = counterexample(name);
        let error = replay_lifecycle(&states).expect_err(name);
        assert!(error.contains(at), "{name} diverged elsewhere: {error}");
    }
}

// ---------------------------------------------------------------------------
// Fencing

fn plan_of(state: &State) -> (Generation, PlanId) {
    let p = record(&state["lastPlan"]);
    (
        Generation(p["g"].parse().unwrap()),
        PlanId::new(&p["id"]).unwrap(),
    )
}

/// Replays one fencing behavior through the production fence.
fn replay_fencing(states: &[State]) -> Result<(), String> {
    let mut fence = Fence::new();
    agree_fencing(0, "Init", &states[0], &fence)?;
    for (i, state) in states.iter().enumerate().skip(1) {
        let action = string(&state["last"]);
        let (g, id) = plan_of(state);
        match action {
            "Accept" => {
                fence = fence
                    .accept(g, &id)
                    .map_err(|e| format!("step {i} Accept: refused: {e}"))?
                    .0;
            }
            // The model admits in one step; the kernel checks at admission.
            "Admit" => {
                if !fence.permits(g, &id) {
                    return Err(format!("step {i} Admit: not permitted"));
                }
            }
            "Check" => {
                if !fence.permits(g, &id) {
                    return Err(format!("step {i} Check: not permitted"));
                }
            }
            // The kernel re-checks where the effect is admitted, whatever an
            // earlier check said (ADR 0010 §3).
            "Effect" => {
                if !fence.permits(g, &id) {
                    return Err(format!("step {i} Effect: the re-check refuses it"));
                }
            }
            other => return Err(format!("step {i}: unknown action {other}")),
        }
        agree_fencing(i, action, state, &fence)?;
    }
    Ok(())
}

/// The Plans the model can offer.
fn plans() -> Vec<(Generation, PlanId)> {
    (1..=3)
        .flat_map(|g| ["a", "b"].map(|id| (Generation(g), PlanId::new(id).unwrap())))
        .collect()
}

/// Whether the kernel's fence agrees with the model's `state`: the accepted
/// generation and Plan, which Plans are accepted, and which are permitted
/// to admit an effect. The model's guards, transcribed from `Fencing.tla`:
/// Accept(p) needs p.g at least the accepted generation, and no other Plan
/// at it; Current(p) needs p to be the accepted Plan at the accepted
/// generation.
fn agree_fencing(i: usize, action: &str, state: &State, fence: &Fence) -> Result<(), String> {
    let gacc: u64 = state["gacc"].trim().parse().unwrap();
    let plan_at = tuple(&state["planAt"]);
    for (g, id) in plans() {
        let at = &plan_at[g.0 as usize - 1];
        let model_accepts = g.0 >= gacc && !(g.0 == gacc && at != "none" && at != id.as_str());
        if fence.accept(g, &id).is_ok() != model_accepts {
            return Err(format!(
                "step {i} {action}: the kernel and the model disagree on accepting {id} at {}",
                g.0
            ));
        }
        let model_current = gacc == g.0 && at == id.as_str();
        if fence.permits(g, &id) != model_current {
            return Err(format!(
                "step {i} {action}: the kernel and the model disagree on admitting {id} at {}",
                g.0
            ));
        }
    }
    let expected = (gacc > 0).then(|| (gacc, plan_at[gacc as usize - 1].clone()));
    let actual = fence
        .accepted()
        .map(|(g, id)| (g.0, id.as_str().to_string()));
    if expected != actual {
        return Err(format!(
            "step {i} {action}: the kernel holds {actual:?}, the model {expected:?}"
        ));
    }
    Ok(())
}

#[test]
fn every_simulated_fencing_behavior_replays() {
    let traces = traces("Fencing");
    assert_eq!(traces.len(), 60);
    let mut seen = BTreeSet::new();
    let mut steps = 0;
    for (file, states) in &traces {
        replay_fencing(states).unwrap_or_else(|e| panic!("{file}: {e}"));
        steps += states.len() - 1;
        seen.extend(states.iter().map(|s| string(&s["last"]).to_string()));
    }
    for action in ["Accept", "Admit"] {
        assert!(seen.contains(action), "no replayed behavior takes {action}");
    }
    println!("Fencing: {} behaviors, {steps} steps", traces.len());
}

#[test]
fn every_fencing_counterexample_diverges() {
    for (name, at) in [
        ("Fencing-split", "Effect"),
        ("Fencing-stale-by-one", "Accept"),
    ] {
        let states = counterexample(name);
        let error = replay_fencing(&states).expect_err(name);
        assert!(error.contains(at), "{name} diverged elsewhere: {error}");
    }
}
