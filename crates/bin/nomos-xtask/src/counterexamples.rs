//! Seven deterministic models of the draft formal documents at `d4c11fa`.
//!
//! Each function models one draft rule literally and shows the outcome the
//! rule admits. They are the Rust form of the research snapshot's
//! `reproduce_counterexamples.py`, kept so the record can be regenerated with
//! the pinned toolchain. They model prose, not Nomos code: the crates that
//! will carry these semantics are still empty. The grounding plan ports each
//! one into the crate it concerns as a negative control.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Value, json};

const CASE_SCOPE: &str =
    "small deterministic Rust model of draft semantics, not Nomos implementation";

/// The report, shaped like the snapshot's `counterexample-results.json`.
#[derive(Serialize)]
pub(crate) struct Report {
    kind: &'static str,
    scope: &'static str,
    results: Vec<Case>,
}

/// One reproduced counterexample.
#[derive(Serialize, Debug)]
pub(crate) struct Case {
    id: &'static str,
    status: &'static str,
    scope: &'static str,
    observed: Value,
    expected_contract: &'static str,
}

impl Case {
    fn new(id: &'static str, observed: Value, expected_contract: &'static str) -> Self {
        Case {
            id,
            status: "counterexample_reproduced",
            scope: CASE_SCOPE,
            observed,
            expected_contract,
        }
    }

    /// The counterexample's key in the research graph.
    #[cfg(test)]
    pub(crate) fn id(&self) -> &'static str {
        self.id
    }

    /// What the model observed.
    #[cfg(test)]
    pub(crate) fn observed(&self) -> &Value {
        &self.observed
    }
}

/// The draft loop assesses only before mutation. With k = 1, a successful
/// final mutation falls through to `NonConvergent(bound)`.
fn final_check() -> Case {
    let bound = 1;
    let mut value = 0;
    let mut reported = "NonConvergent(bound)";
    for _ in 0..bound {
        if value == 1 {
            reported = "Converged";
            break;
        }
        value = 1;
    }
    Case::new(
        "final-check",
        json!({ "final_value": value, "reported": reported }),
        "Reassess after the final permitted execution.",
    )
}

/// Terminal is not the same predicate as Changed.
fn activation_missing() -> Case {
    let (terminal, changed) = (true, false);
    let draft_ready = terminal;
    let required_ready = terminal && changed;
    debug_assert!(draft_ready && !required_ready);
    Case::new(
        "activation-missing",
        json!({ "draft_ready": draft_ready, "changed": changed }),
        "An on_change gate requires a relevant verified change.",
    )
}

/// The local precheck and the actual effect are different transitions.
fn fence_race() -> Case {
    let (mut accepted, old) = (51, 51);
    let passed = old >= accepted;
    accepted = 52;
    let emitted_fence = passed.then_some(old);
    Case::new(
        "fence-race",
        json!({ "accepted_at_effect": accepted, "old_effect_fence": emitted_fence }),
        "Effect admission must close the check-to-use gap or block conflicting supersession.",
    )
}

/// Reusing a semantic digest across independent runs suppresses fresh repair.
fn dedup_scope() -> Case {
    let semantic_key = "ensure-file-v1";
    let completed: BTreeMap<&str, &str> = BTreeMap::from([(semantic_key, "Completed")]);
    let file_value = "drifted";
    let apply_again = !completed.contains_key(semantic_key);
    Case::new(
        "dedup-scope",
        json!({ "file": file_value, "apply_again": apply_again }),
        "Deduplicate one execution, not all future repairs of the same semantic action.",
    )
}

/// The configuration changed, but the refresh did not happen before the crash.
fn lost_refresh() -> Case {
    let (disk_revision, loaded_revision) = ("v2", "v1");
    let file_variance = disk_revision != "v2";
    let transient_trigger_on_next_run = file_variance;
    Case::new(
        "lost-refresh",
        json!({
            "disk_revision": disk_revision,
            "loaded_revision": loaded_revision,
            "next_run_trigger": transient_trigger_on_next_run,
        }),
        "Persist refresh intent or observe the loaded revision explicitly.",
    )
}

/// Kleene-style conjunction: a definite false is already decisive.
fn partial_assessment() -> Case {
    fn conjunction(values: &[&str]) -> &'static str {
        if values.contains(&"false") {
            "false"
        } else if values.contains(&"unknown") {
            "unknown"
        } else {
            "true"
        }
    }
    let inputs = ["false", "unknown"];
    let result = conjunction(&inputs);
    Case::new(
        "partial-assessment",
        json!({ "inputs": inputs, "conjunction": result }),
        "Preserve per-condition findings; unknown does not erase an independent known mismatch.",
    )
}

/// A correct admission cannot prevent a later independent hardware failure.
fn budget_not_world() -> Case {
    let budget = 1;
    let reserved: BTreeSet<&str> = BTreeSet::from(["node-a"]);
    let external_failed: BTreeSet<&str> = BTreeSet::from(["node-b"]);
    let actual_unavailable = reserved.union(&external_failed).count();
    Case::new(
        "budget-not-world",
        json!({ "budget": budget, "reserved": reserved.len(), "actual_unavailable": actual_unavailable }),
        "Guarantee admission safety under explicit assumptions, not unconditional availability of reality.",
    )
}

/// Runs all seven models in the snapshot's order.
pub(crate) fn run() -> Vec<Case> {
    vec![
        final_check(),
        activation_missing(),
        fence_race(),
        dedup_scope(),
        lost_refresh(),
        partial_assessment(),
        budget_not_world(),
    ]
}

/// The full report.
pub(crate) fn report() -> Report {
    Report {
        kind: "local_reproduction_report",
        scope: "Seven small deterministic models of the draft formal documents, not tests of Nomos code, a TLA+ check, or a Linux test.",
        results: run(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn final_check_misreports_the_last_successful_iteration() {
        let c = final_check();
        assert_eq!(c.observed()["final_value"], 1);
        assert_ne!(c.observed()["reported"], "Converged");
    }

    #[test]
    fn draft_readiness_ignores_activation() {
        let c = activation_missing();
        assert_eq!(c.observed()["draft_ready"], true);
        assert_eq!(c.observed()["changed"], false);
    }

    #[test]
    fn precheck_passes_before_a_newer_fence_is_accepted() {
        let c = fence_race();
        assert_eq!(c.observed()["old_effect_fence"], 51);
        assert_eq!(c.observed()["accepted_at_effect"], 52);
    }

    #[test]
    fn semantic_key_suppresses_a_fresh_repair() {
        let c = dedup_scope();
        assert_eq!(c.observed()["apply_again"], false);
        assert_ne!(c.observed()["file"], "desired");
    }

    #[test]
    fn transient_trigger_is_lost_across_the_crash() {
        let c = lost_refresh();
        assert_eq!(c.observed()["next_run_trigger"], false);
        assert_ne!(
            c.observed()["loaded_revision"],
            c.observed()["disk_revision"]
        );
    }

    #[test]
    fn known_false_survives_an_unrelated_unknown() {
        assert_eq!(partial_assessment().observed()["conjunction"], "false");
    }

    #[test]
    fn admission_within_budget_does_not_bound_the_world() {
        let c = budget_not_world();
        assert!(c.observed()["reserved"].as_u64() <= c.observed()["budget"].as_u64());
        assert!(c.observed()["actual_unavailable"].as_u64() > c.observed()["budget"].as_u64());
    }

    #[test]
    fn observations_match_the_shipped_python_results() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../../docs/research/2026-09-28-typed-core/snapshot/counterexample-results.json",
        );
        let shipped: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let shipped = shipped["results"].as_array().unwrap();
        let ours = run();
        assert_eq!(ours.len(), shipped.len());
        for (mine, theirs) in ours.iter().zip(shipped) {
            assert_eq!(json!(mine.id()), theirs["id"]);
            assert_eq!(mine.observed(), &theirs["observed"], "{}", mine.id());
            assert_eq!(json!(mine.expected_contract), theirs["expected_contract"]);
        }
    }
}
