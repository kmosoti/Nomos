//! Experiment `kernel-conformance`, for `RefreshRecovery.tla`
//! ([formal/tla/README.md](../../../../formal/tla/README.md)).
//!
//! The model collapses observation, planning, and verification into guards,
//! and leaves open which Ready Action runs first, so its steps do not map
//! one to one onto `step` as the lifecycle and fence models do. What is
//! replayed is the model's *environment*: each host action, receipt,
//! deadline, settle-by instant, crash, and superseding Plan in a TLC trace
//! is performed on the simulator in the same order, the kernel makes its
//! own decisions in between, and after every step the model's four
//! properties are checked on the kernel's state and the host's. A trace
//! whose environment step has nothing to act on, because the kernel chose
//! differently where the model is free, is replayed up to that step and
//! counted.
//!
//! The negative-control counterexamples replay to states the model's
//! mutated kernels reach and the real kernel must not.

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nomos_app::kernel::{Canon, RunOutcome};
use nomos_core::effect::EffectKey;
use nomos_core::observation::Instant;
use nomos_warp::graph::EdgeKind;
use support::*;

const OTHER: &str = "/zz/other";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/tla")
}

/// The `last` label of every state in a TLC trace, in order.
fn labels(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("/\\ last = "))
        .map(|v| v.trim().trim_matches('"').to_string())
        .collect()
}

fn canon() -> Canon {
    Canon::new(
        vec![
            file(CONF, d(NEW), &["file:/etc/svc.conf"]),
            service(SVC, &["systemd:svc"]),
            file(OTHER, d(NEW), &["file:/etc/svc.conf"]),
        ],
        vec![edge(CONF, SVC, EdgeKind::OnChange)],
    )
}

fn plan_for(generation: u64) -> nomos_app::kernel::Plan {
    let mut plan = plan(&format!("g{generation}"), generation, canon(), 4);
    // Settle-by far beyond any run of deadlines, so that only the model's
    // SettleBy step settles an effect by time.
    plan.policy.settle_after = 1000;
    plan
}

struct Replay {
    sim: Sim,
    generation: u64,
}

/// What a replay established.
#[derive(Debug, Default)]
struct Summary {
    steps: usize,
    replayed: usize,
    complete: bool,
    stopped_at: Option<String>,
}

impl Replay {
    fn new() -> Self {
        let mut host = refresh_host();
        host.write(&p(OTHER), d(OLD));
        let mut sim = Sim::new(host);
        sim.enforce(plan_for(1));
        let mut replay = Replay { sim, generation: 1 };
        replay.drain();
        replay
    }

    /// Delivers the kernel's own traffic: reads and Observations.
    fn drain(&mut self) {
        loop {
            let at = self.sim.queue.iter().position(|i| {
                matches!(
                    i,
                    Item::Read { .. } | Item::Deliver(nomos_app::kernel::Input::Observed(_))
                )
            });
            match at {
                Some(i) => {
                    let item = self.sim.queue.remove(i).unwrap();
                    self.sim.deliver(item);
                }
                None => return,
            }
        }
    }

    /// Delivers the host work for `path`; false if there is none.
    fn execute(&mut self, path: &str) -> bool {
        let at = self.sim.queue.iter().position(
            |i| matches!(i, Item::Execute { request, .. } if request.key.resource() == &k(path)),
        );
        match at {
            Some(i) => {
                let item = self.sim.queue.remove(i).unwrap();
                self.sim.deliver(item);
                true
            }
            None => false,
        }
    }

    /// Delivers every receipt for `path`; false if there is none.
    fn receipts(&mut self, path: &str) -> bool {
        let mut any = false;
        while let Some(i) = self.sim.queue.iter().position(|i| {
            matches!(i, Item::Deliver(nomos_app::kernel::Input::Receipt(e, _)) if e.resource() == &k(path))
        }) {
            let item = self.sim.queue.remove(i).unwrap();
            self.sim.deliver(item);
            any = true;
        }
        any
    }

    fn settled(&self) -> BTreeSet<EffectKey> {
        let s = &self.sim.snapshot;
        s.effects()
            .iter()
            .filter(|(_, e)| e.settlement.is_settled())
            .map(|(k, _)| k.clone())
            .chain(s.released().iter().cloned())
            .collect()
    }

    /// Performs one environment step of the model; false if the kernel's
    /// state gives it nothing to act on.
    fn perform(&mut self, label: &str) -> bool {
        let done = match label {
            "HostWrites" => self.execute(CONF),
            "WriteReceipt" => self.receipts(CONF),
            "HostRestarts" => self.execute(SVC),
            "RefreshReceipt" => self.receipts(SVC),
            "OtherDone" => self.execute(OTHER) && self.receipts(OTHER),
            "Deadline" => {
                self.sim.tick(6);
                true
            }
            "SettleBy" => {
                let latest = self
                    .sim
                    .snapshot
                    .effects()
                    .values()
                    .filter_map(|e| match e.settlement {
                        nomos_core::effect::Settlement::Unsettled { settle_by } => Some(settle_by),
                        _ => None,
                    })
                    .max();
                if let Some(Instant(t)) = latest {
                    let dt = t.saturating_sub(self.sim.now);
                    self.sim.tick(dt.max(1));
                }
                true
            }
            "Crash" => {
                self.sim.crash();
                true
            }
            "NewRun" => {
                self.generation += 1;
                self.sim.enforce(plan_for(self.generation));
                true
            }
            // The kernel's own decisions: it makes them as its inputs allow.
            _ => true,
        };
        self.drain();
        done
    }

    /// The model's properties, on the kernel and the host.
    fn check(&self, label: &str, before: &Snapshot) -> Result<(), String> {
        let s = &self.sim.snapshot;
        let effects: Vec<_> = s.effects().values().collect();
        for (i, a) in effects.iter().enumerate() {
            for b in &effects[i + 1..] {
                if !a.keys.is_disjoint(&b.keys) {
                    return Err(format!("{label}: AdmissionSafety"));
                }
            }
        }
        if self.sim.outcome() == Some(RunOutcome::Converged) {
            let sound = self.sim.host.file(&p(CONF)) == Some(d(NEW))
                && self.sim.refresh_consumed()
                && s.obligations().is_empty()
                && s.effects().values().all(|e| e.settlement.is_settled());
            if !sound {
                return Err(format!("{label}: ConvergedIsSound"));
            }
        }
        if matches!(label, "Deadline" | "Crash") && self.settled() != before.settled {
            return Err(format!("{label}: UncertaintyPreserved"));
        }
        if s.obligations().len() < before.obligations && label != "RefreshReceipt" {
            return Err(format!("{label}: ObligationDischargedOnlyByRefresh"));
        }
        Ok(())
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            settled: self.settled(),
            obligations: self.sim.snapshot.obligations().len(),
        }
    }
}

struct Snapshot {
    settled: BTreeSet<EffectKey>,
    obligations: usize,
}

/// Replays the environment of one trace, checking after every step.
fn replay(labels: &[String]) -> Result<(Replay, Summary), String> {
    let mut r = Replay::new();
    let mut summary = Summary {
        steps: labels.len().saturating_sub(1),
        ..Summary::default()
    };
    for label in labels.iter().skip(1) {
        let before = r.snapshot();
        if !r.perform(label) {
            summary.stopped_at = Some(label.clone());
            return Ok((r, summary));
        }
        r.check(label, &before)?;
        summary.replayed += 1;
    }
    summary.complete = true;
    Ok((r, summary))
}

#[test]
fn simulated_refresh_behaviors_keep_the_properties_on_the_kernel() {
    let mut files: Vec<PathBuf> = fs::read_dir(fixtures().join("RefreshRecovery"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 80);
    let (mut steps, mut replayed, mut complete) = (0, 0, 0);
    let mut seen = BTreeSet::new();
    let mut stops: std::collections::BTreeMap<String, usize> = Default::default();
    for f in &files {
        let labels = labels(&fs::read_to_string(f).unwrap());
        let (_, summary) = replay(&labels).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        steps += summary.steps;
        replayed += summary.replayed;
        complete += usize::from(summary.complete);
        if let Some(label) = summary.stopped_at {
            *stops.entry(label).or_default() += 1;
        }
        seen.extend(labels.iter().take(summary.replayed + 1).cloned());
    }
    for action in [
        "HostWrites",
        "WriteReceipt",
        "Deadline",
        "SettleBy",
        "HostRestarts",
        "RefreshReceipt",
        "Crash",
        "NewRun",
    ] {
        assert!(seen.contains(action), "no replayed step performs {action}");
    }
    println!(
        "RefreshRecovery: {} behaviors, {complete} replayed whole; {replayed} of {steps} steps; stopped at {stops:?}",
        files.len()
    );
}

/// `transient`: the model loses the refresh when a superseding Plan arrives
/// after the write. The kernel keeps the Obligation, so the same schedule
/// never reaches Converged with the old revision loaded, and once the rest
/// is delivered the refresh happens.
#[test]
fn the_transient_counterexample_does_not_lose_the_refresh() {
    let text = fs::read_to_string(fixtures().join("RefreshRecovery-transient.out")).unwrap();
    let labels = labels(&text);
    assert_eq!(labels.last().map(String::as_str), Some("Converge"));
    let (mut r, _) = replay(&labels).expect("the properties hold on the kernel");
    assert!(!r.sim.snapshot.obligations().is_empty() || r.sim.refresh_consumed());
    assert_eq!(r.sim.run(), RunOutcome::Converged);
    assert!(r.sim.refresh_consumed());
}

/// `release-on-timeout`: the model admits the conflicting Action once the
/// write times out. The kernel does not.
#[test]
fn the_release_on_timeout_counterexample_admits_nothing_conflicting() {
    let text =
        fs::read_to_string(fixtures().join("RefreshRecovery-release-on-timeout.out")).unwrap();
    let labels = labels(&text);
    assert_eq!(labels.last().map(String::as_str), Some("DispatchOther"));
    let (r, _) = replay(&labels).expect("the properties hold on the kernel");
    assert!(r.sim.issued.iter().all(|e| e.resource() != &k(OTHER)));
}

/// `discharge-on-dispatch`: the model clears the Obligation when the refresh
/// is dispatched. The kernel still holds it then.
#[test]
fn the_discharge_on_dispatch_counterexample_keeps_the_obligation() {
    let text =
        fs::read_to_string(fixtures().join("RefreshRecovery-discharge-on-dispatch.out")).unwrap();
    let labels = labels(&text);
    assert_eq!(labels.last().map(String::as_str), Some("DispatchRefresh"));
    let (r, _) = replay(&labels).expect("the properties hold on the kernel");
    assert!(
        r.sim.issued.iter().any(|e| e.resource() == &k(SVC)),
        "dispatched"
    );
    assert_eq!(r.sim.snapshot.obligations().len(), 1, "and still owed");
}
