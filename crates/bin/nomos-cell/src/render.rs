//! What the Cell's commands print, and the exit status of each outcome
//! ([cell-commands.md]). Pure functions of the kernel's values, so that
//! the text and the statuses are tested apart from any host.
//!
//! [cell-commands.md]: ../../../../docs/formal/cell-commands.md

use std::collections::BTreeMap;
use std::fmt::Write;

use nomos_app::kernel::{ActionRecord, Event, Nonconvergence, RunOutcome};
use nomos_core::action::Stage;
use nomos_core::assessment::{AccountField, Assessment, MetadataField, Reason, Report, Variance};
use nomos_core::effect::Operation;
use nomos_core::observation::CollectionFailure;
use nomos_core::resource::ResourceKey;
use nomos_core::traits::Trait;

/// The exit status of an error: usage, the artifact, the bundle, the state.
pub const ERROR: i32 = 1;

/// A collection failure's name.
pub fn failure(f: CollectionFailure) -> &'static str {
    match f {
        CollectionFailure::PermissionDenied => "permission-denied",
        CollectionFailure::TimedOut => "timed-out",
        CollectionFailure::Unsupported => "unsupported",
        CollectionFailure::Io => "io",
        CollectionFailure::Unavailable => "unavailable",
    }
}

/// A digest as the Canon names it, `sha256:<hex>`.
pub fn digest(d: &nomos_core::resource::Digest) -> String {
    let hex: String = d.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// An Indeterminate reason's text.
pub fn reason(r: &Reason) -> String {
    match r {
        Reason::NoObservation => "no-observation".into(),
        Reason::CollectionFailed(f) => format!("collection-failed: {}", failure(*f)),
        Reason::Conflicting => "conflicting".into(),
        Reason::WrongFamily => "wrong-family".into(),
    }
}

/// A Variance's text.
pub fn variance(v: &Variance) -> String {
    match v {
        Variance::Missing => "missing".into(),
        Variance::Unexpected => "unexpected".into(),
        Variance::ContentDiffers { expected, observed } => {
            format!(
                "content-differs: observed {}, desired {}",
                digest(observed),
                digest(expected)
            )
        }
        Variance::MetadataDiffers(field) => format!(
            "metadata-differs: {}",
            match field {
                MetadataField::Owner => "owner",
                MetadataField::Group => "group",
                MetadataField::Mode => "mode",
            }
        ),
        Variance::UnitDiffers {
            activity,
            enablement,
        } => {
            let axes: Vec<&str> = [(*activity, "activity"), (*enablement, "enablement")]
                .into_iter()
                .filter_map(|(differs, name)| differs.then_some(name))
                .collect();
            format!("unit-differs: {}", axes.join(", "))
        }
        Variance::ValueDiffers => "value-differs".into(),
        Variance::AccountDiffers(field) => format!(
            "account-differs: {}",
            match field {
                AccountField::Class => "class",
                AccountField::Home => "home",
                AccountField::Shell => "shell",
            }
        ),
        Variance::VersionDiffers => "version-differs".into(),
        Variance::Broken => "broken".into(),
    }
}

/// The Action planned for a resource, as trace prints it.
pub fn action(record: Option<&ActionRecord>) -> String {
    match record {
        None => "none".into(),
        Some(r) => {
            let verb = match r.operation {
                Operation::Converge(_) => "converge",
                Operation::Refresh(_) => "refresh",
            };
            if r.discharges.is_empty() {
                verb.into()
            } else {
                format!("{verb} (owed)")
            }
        }
    }
}

/// The report `trace` prints, and its exit status.
pub fn trace(report: &Report, actions: &BTreeMap<ResourceKey, ActionRecord>) -> (String, i32) {
    let mut out = String::new();
    let (mut satisfied, mut varying, mut unknown) = (0, 0, 0);
    for (condition, assessment) in report.entries() {
        let key = condition.key();
        let _ = writeln!(out, "{key}");
        let _ = match assessment {
            Assessment::Satisfied => {
                satisfied += 1;
                writeln!(out, "  SATISFIED")
            }
            Assessment::Variance(v) => {
                varying += 1;
                writeln!(out, "  VARIANCE {}", variance(v))
            }
            Assessment::Indeterminate(r) => {
                unknown += 1;
                writeln!(out, "  INDETERMINATE {}", reason(r))
            }
        };
        let _ = writeln!(out, "  action: {}\n", action(actions.get(key)));
    }
    let _ = writeln!(
        out,
        "{satisfied} satisfied, {varying} variance, {unknown} indeterminate; {} actions planned",
        actions.len()
    );
    let status = if unknown > 0 {
        2
    } else if varying > 0 || !actions.is_empty() {
        3
    } else {
        0
    };
    (out, status)
}

/// A Stage's name.
pub fn stage(s: &Stage) -> String {
    match s {
        Stage::Prepared => "prepared".into(),
        Stage::Dispatched => "dispatched".into(),
        Stage::Accepted => "accepted".into(),
        Stage::Running => "running".into(),
        Stage::Verifying { .. } => "verifying".into(),
        Stage::Succeeded { changed, .. } => {
            if *changed {
                "succeeded, changed".into()
            } else {
                "succeeded, unchanged".into()
            }
        }
        Stage::Failed(f) => format!("failed: {f:?}"),
        Stage::TimedOut => "timed-out".into(),
        Stage::Cancelled => "cancelled".into(),
        Stage::Rejected => "rejected".into(),
    }
}

/// The outcome `enforce` prints, and its exit status (cell-commands.md,
/// Exit Status).
pub fn outcome(o: &RunOutcome) -> (String, i32) {
    let keys = |ks: &[ResourceKey], label: &str| -> String {
        ks.iter().map(|k| format!("  {label} {k}\n")).collect()
    };
    match o {
        RunOutcome::Converged => ("outcome: converged\n".into(), 0),
        RunOutcome::Indeterminate(list) => {
            let mut out = String::from("outcome: indeterminate\n");
            for (k, r) in list {
                let _ = writeln!(out, "  {k}: {}", reason(r));
            }
            (out, 2)
        }
        RunOutcome::NonConvergent(n) => (
            format!(
                "outcome: non-convergent: {}\n",
                match n {
                    Nonconvergence::Bound => "bound",
                    Nonconvergence::Oscillation => "oscillation",
                }
            ),
            3,
        ),
        RunOutcome::Failed { failed, unknown } => (
            format!(
                "outcome: failed\n{}{}",
                keys(failed, "failed:"),
                keys(unknown, "unknown:")
            ),
            4,
        ),
        RunOutcome::Superseded => ("outcome: superseded\n".into(), ERROR),
    }
}

/// One Event's line, without its Plan's Canon.
pub fn event(e: &Event) -> String {
    match e {
        Event::Clock(t) => format!("clock {}", t.0),
        Event::PlanRejected {
            plan,
            generation,
            reason,
        } => format!(
            "plan-rejected {plan} generation {} {reason:?}",
            generation.0
        ),
        Event::PlanRedelivered { plan } => format!("plan-redelivered {plan}"),
        Event::PlanAccepted(p) => format!(
            "plan-accepted {} generation {}, {} resources",
            p.id,
            p.generation.0,
            p.canon.keys().len()
        ),
        Event::RunEnded(o) => outcome(o).0.lines().collect::<Vec<_>>().join(";"),
        Event::ObservationRequested { since } => format!("observation-requested since {}", since.0),
        Event::RoundPlanned { round, .. } => format!(
            "round-planned iteration {}, {} actions, {} indeterminate",
            round.iteration,
            round.actions.len(),
            round.indeterminate.len()
        ),
        Event::IterationAdvanced => "iteration-advanced".into(),
        Event::ObligationRecorded(o) => format!("obligation-recorded {}", o.target),
        Event::ObligationWithdrawn(o) => format!("obligation-withdrawn {}", o.target),
        Event::ObligationDischarged(o) => format!("obligation-discharged {}", o.target),
        Event::ActionDispatched { resource, .. } => format!("action-dispatched {resource}"),
        Event::ActionAdvanced {
            resource, stage: s, ..
        } => format!("action-advanced {resource} {}", stage(s)),
        Event::EffectRequested { key, .. } => format!("effect-requested {}", key.resource()),
        Event::EffectSettled { key, by } => {
            format!("effect-settled {} {by:?}", key.resource())
        }
        Event::EffectReleased { key } => format!("effect-released {}", key.resource()),
        Event::ReceiptIgnored { key, why, .. } => {
            format!("receipt-ignored {} {why:?}", key.resource())
        }
        Event::Recovered => "recovered".into(),
    }
}

/// One Trait's line.
pub fn traits(ts: &[Trait]) -> String {
    ts.iter()
        .map(|t| {
            format!(
                "{} = {} ({}, {})\n",
                t.key,
                t.value,
                t.stability.as_str(),
                t.source
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status table of cell-commands.md, one row per outcome.
    #[test]
    fn each_outcome_exits_with_the_status_the_table_names() {
        let k = ResourceKey::Sysctl(nomos_core::resource::SysctlKey::new("net.x").unwrap());
        assert_eq!(outcome(&RunOutcome::Converged).1, 0);
        assert_eq!(
            outcome(&RunOutcome::Indeterminate(vec![(
                k.clone(),
                Reason::CollectionFailed(CollectionFailure::PermissionDenied)
            )]))
            .1,
            2
        );
        assert_eq!(
            outcome(&RunOutcome::NonConvergent(Nonconvergence::Bound)).1,
            3
        );
        assert_eq!(
            outcome(&RunOutcome::NonConvergent(Nonconvergence::Oscillation)).1,
            3
        );
        assert_eq!(
            outcome(&RunOutcome::Failed {
                failed: vec![k.clone()],
                unknown: vec![]
            })
            .1,
            4
        );
        assert_eq!(outcome(&RunOutcome::Superseded).1, ERROR);
        let (text, _) = outcome(&RunOutcome::Failed {
            failed: vec![],
            unknown: vec![k],
        });
        assert_eq!(text, "outcome: failed\n  unknown: sysctl:net.x\n");
    }
}
