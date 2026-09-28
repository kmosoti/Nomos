#!/usr/bin/env python3
"""Counterexamples to draft semantics. This does NOT execute Nomos Rust code."""
from __future__ import annotations
import json
from pathlib import Path

def run() -> list[dict]:
    out = []
    def record(key: str, observed: dict, expected: str) -> None:
        out.append({"id": key, "status": "counterexample_reproduced", "scope": "small deterministic Python model of draft semantics, not Nomos implementation", "observed": observed, "expected_contract": expected})

    # The draft only assesses before mutation; k=1 then falls through.
    value, reported = 0, "NonConvergent(bound)"
    for _ in range(1):
        if value == 1:
            reported = "Converged"
            break
        value = 1
    assert value == 1 and reported != "Converged"
    record("final-check", {"final_value": value, "reported": reported}, "Reassess after the final permitted execution.")

    # Terminal is not the same predicate as Changed.
    terminal, changed = True, False
    draft_ready = terminal
    required_ready = terminal and changed
    assert draft_ready and not required_ready
    record("activation-missing", {"draft_ready": draft_ready, "changed": changed}, "An on_change gate requires a relevant verified change.")

    # The local precheck and actual effect are different transitions.
    accepted, old = 51, 51
    passed = old >= accepted
    accepted = 52
    emitted_fence = old if passed else None
    assert emitted_fence == 51 and emitted_fence < accepted
    record("fence-race", {"accepted_at_effect": accepted, "old_effect_fence": emitted_fence}, "Effect admission must close the check-to-use gap or block conflicting supersession.")

    # Reusing a semantic digest across independent runs suppresses fresh repair.
    semantic_key = "ensure-file-v1"
    completed = {semantic_key: "Completed"}
    file_value = "drifted"
    apply_again = semantic_key not in completed
    assert not apply_again and file_value != "desired"
    record("dedup-scope", {"file": file_value, "apply_again": apply_again}, "Deduplicate one execution, not all future repairs of the same semantic action.")

    # Config changed, but refresh was not performed before the crash.
    disk_revision, loaded_revision = "v2", "v1"
    file_variance = disk_revision != "v2"
    transient_trigger_on_next_run = file_variance
    assert not transient_trigger_on_next_run and loaded_revision != disk_revision
    record("lost-refresh", {"disk_revision": disk_revision, "loaded_revision": loaded_revision, "next_run_trigger": transient_trigger_on_next_run}, "Persist refresh intent or observe the loaded revision explicitly.")

    # Kleene-style conjunction: a definite false is already decisive.
    def conjunction(values: list[str]) -> str:
        if "false" in values: return "false"
        if "unknown" in values: return "unknown"
        return "true"
    result = conjunction(["false", "unknown"])
    assert result == "false"
    record("partial-assessment", {"inputs": ["false", "unknown"], "conjunction": result}, "Preserve per-condition findings; unknown does not erase an independent known mismatch.")

    # A correct admission cannot prevent later independent hardware failure.
    budget, reserved, external_failed = 1, {"node-a"}, {"node-b"}
    actual_unavailable = len(reserved | external_failed)
    assert len(reserved) <= budget and actual_unavailable > budget
    record("budget-not-world", {"budget": budget, "reserved": len(reserved), "actual_unavailable": actual_unavailable}, "Guarantee admission safety under explicit assumptions, not unconditional availability of reality.")
    return out

if __name__ == "__main__":
    results = run()
    target = Path(__file__).with_name("counterexample-results.json")
    target.write_text(json.dumps({"kind":"local_reproduction_report","scope":"Seven small deterministic models, not a Rust build, TLA+ check, or Linux test.","results":results}, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"counterexamples_reproduced":len(results),"result_file":str(target)}))
