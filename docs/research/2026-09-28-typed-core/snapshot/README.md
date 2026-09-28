# Nomos Typed-Core Research Graph

Research snapshot: **2026-09-28 UTC**. Repository reviewed: `kmosoti/Nomos`, `main` at commit `d4c11fa221c4cdc056bc85b6df84e127a6709d00`.

This bundle connects primary sources, repository findings, recommendations, open questions, proposed experiments, and candidate architecture decision records. It takes the conversation's Rust-authored Canon and Condition/Observation terminology as the intended direction. Those decisions are **not assumed to have been implemented in the pinned repository**.

No repository files were modified. This is a research and design artifact, not an implementation or verification certificate.

## Contents

| File | Purpose |
| --- | --- |
| `nomos-research.ndjson` | The weighted property graph: one JSON object per line |
| `record.schema.json` | JSON Schema for each record, draft 2020-12 |
| `RECOMMENDATIONS.md` | Human-readable recommendation index with source links |
| `validate_graph.py` | Strict parser, schema option, reference checks, and negative controls |
| `validation-report.json` | Actual validation outcome for the supplied graph |
| `reproduce_counterexamples.py` | Seven small deterministic Python counterexamples |
| `counterexample-results.json` | Actual outputs of those local models |
| `graph-stats.json` | Node, edge, and record counts |
| `MANIFEST.sha256` | Checksums of the bundle files, excluding the manifest itself |

The graph contains **169 nodes, 348 edges, and one metadata record**, totaling **518 NDJSON records**.

| Node kind | Count | Interpretation |
| --- | ---: | --- |
| Source | 37 | 30 external primary references, six pinned repository documents, and one conversation decision record |
| Concept | 18 | Nomos vocabulary and relevant internal design concepts |
| Finding | 26 | Documented mechanisms, repository observations, or labeled deductions |
| Recommendation | 26 | Proposed direction, constraints, acceptance criteria, and target paths |
| Experiment | 21 | Proposed experiments; none are marked as executed |
| Open question | 14 | Unresolved decisions and closure criteria |
| Research direction | 6 | Selective avenues beyond the baseline kernel |
| Correction | 6 | Explicit qualifications or corrections to earlier design guidance |
| Counterexample | 7 | Locally executed toy models, not Nomos code tests |
| ADR candidate | 8 | Grouped decision topics, not accepted or committed ADRs |

## Weights

Weights are **ordinal editorial judgments**, not measured embeddings, probabilities, benchmark results, or calibrated model confidence. They are intended to make review order inspectable, not to replace engineering judgment.

| Field | Meaning |
| --- | --- |
| `weights.importance` | Long-term consequence for correctness, safety, or architectural coherence |
| `weights.salience` | Relevance and urgency for the current Phase 0 and pre-mutation design decisions |
| `weights.rationale` | Why the item received its priority |
| `semantic_weight` on an edge | Directness and relevance of that relationship |
| `epistemic.evidence_strength` | Categorical support description, deliberately not a numerical weight |
| `epistemic.uncertainty` | What has not been established |

The common 0–4 scale is: **0 out of scope, 1 peripheral, 2 useful but non-blocking, 3 high leverage, 4 foundational or safety-critical**. Interpret the scale in the context of each field. An important eventual requirement can have lower immediate salience. A highly salient issue can remain unresolved.

Compare weights within a useful category. Source weights are not rankings of scientific credibility. In particular, filter recommendations by `details.disposition` before treating a high score as an implementation instruction. `research_required` is not an accepted design, and `defer_until_measured` is not a Phase 0 dependency.

No combined truth/priority score is computed. Graph degree, source count, and repeated citations are not evidence strength.

## Record Model

The file is UTF-8 NDJSON, not a JSON array, RDF serialization, or JSON-LD document. Each line is a complete object with `record_type` equal to `metadata`, `node`, or `edge`. The first line is metadata.

IDs are stable within this snapshot, for example:

```text
urn:moiric:nomos:research:2026-09-28:rec:typed-canon
```

A recommendation node carries a statement, weights, epistemic status, source IDs, constraints, acceptance criteria, experiment IDs, and proposed repository paths. A source node carries a URL, locator, read depth, retrieval date, source type, version limitation, and a supplemental citation marker. **URLs and locators are the portable provenance**; the citation marker is specific to the originating conversation interface.

Edges carry `from`, `to`, `relation`, `semantic_weight`, rationale, and optional evidence IDs. Important directions:

| Relation | Direction |
| --- | --- |
| `documents` | A source documents a mechanism or repository statement |
| `informs` | A source or idea informs a synthesis without proving it |
| `motivates` | A finding motivates a recommendation |
| `applies_to` | A recommendation concerns a concept |
| `tested_by` | A recommendation is assigned a proposed experiment |
| `depends_on` | A recommendation depends on another recommendation |
| `addresses` | A recommendation addresses a concern or question |
| `resolved_by` | An open question identifies recommendations that may resolve it |
| `record_in` | A recommendation belongs in a proposed ADR topic |
| `supports` | A local counterexample supports a bounded deduction |

`tested_by` means a test is specified, not that it has passed. The `execution_status` field is decisive. The overall research graph is not required to be acyclic; only recommendation dependency edges are checked for cycles.

## Where to Start

Start with `rec:algebraic-model`, `rec:validated-boundary`, and `rec:evidence-assessment`. They define legal domain values and what the kernel can conclude from evidence. Then review `rec:canonical-profile`, `rec:pure-kernel`, and `rec:warp-semantics` before implementing the mock pipeline. Review `rec:effect-recovery`, `rec:durable-refresh`, and `rec:log-boundary` before introducing effects that need crash recovery.

Several decisions need explicit qualification:

**Rust authoring is code execution.** Strongly typed Rust is a suitable authoring surface, not a purity guarantee. The build environment and generator need isolation, bounded execution, explicit inputs, and no host-management credentials. Loom and Cell should consume an inert artifact. Cargo's build-script behavior is linked through `src:cargo-build`.

**Validated constructors must survive decoding.** Private fields and constructors alone are insufficient when derived deserialization can populate fields directly. Decode into untrusted data-transfer objects and use fallible conversion into validated domain types. See `src:serde` and `rec:validated-boundary`.

**Algebraic alternatives should remove contradictions.** `Absent | Present(FileRequirements)` avoids an absent file carrying required contents. An Event envelope with one payload enum avoids mismatched event-kind/payload pairs. A known mismatch in one Condition must not disappear because another Condition is indeterminate.

**Change triggers need durable semantics.** A file can be replaced before a crash, while its service keeps using old configuration. The next observation sees no file Variance and may never reactivate a restart. Either observe the applied configuration revision or preserve an explicit refresh obligation. This is not solved by general idempotency rhetoric.

**Fencing concerns effects, not just packets.** A generation check before dispatch leaves a check-to-use gap. A higher accepted generation does not revoke a previously issued OS job. Epoch and incarnation fields also need trustworthy issuance, authorization, and recovery rules.

**Convergence needs assumptions.** A finite loop count does not bound a blocking operation, and separate controllers can each converge in isolation while conflicting in composition. Ownership, interference, fairness, and outstanding work belong in the model.

## Contemporary Research to Investigate Selectively

`lineage:anvil-welder` links Anvil and Welder, research directly concerned with reconciliation liveness and compositional controller reasoning. Transfer their discipline of explicit assumptions and guarantees; do not assume Kubernetes object semantics hold for arbitrary Linux resources. The reviewed Welder author-hosted PDF names conference dates after this snapshot. Its publication metadata is not presented as evidence that the conference has already taken place, and its evaluation was not reproduced.

`lineage:agent-verification` links VeruSAGE's treatment of agent-assisted Rust verification and invalid proof shortcuts. The recommendation is to protect specifications and review new assumptions independently from generated proof annotations. Tool success cannot substitute for specification integrity.

`lineage:incremental` uses Salsa as a tracked-input precedent. Adopt incremental planning only after full recomputation is a trusted reference and an ablation shows a useful improvement.

`lineage:proof-carrying-plan` is explicitly a **Nomos hypothesis**: an independent checker might validate a compact Plan witness. It is not an existing Nomos feature, a reproduced research result, or proof that observations describe reality.

Transparency logs and bounded expression environments are included as separate, optional research directions. No consensus system, expression runtime, or verification framework is made mandatory merely because it appears in the graph.

## Validation and Reproduction

Run standard-library integrity checks:

```sh
python validate_graph.py
python reproduce_counterexamples.py
```

For full per-record schema validation, make the Python `jsonschema` package available, then run:

```sh
python validate_graph.py --require-schema --report validation-report.json
```

The supplied report records the actual validator version and graph/schema hashes. It checks strict JSON parsing, duplicate keys, non-finite numbers, IDs, weights, references, experiment coverage, metadata counts, and recommendation dependency cycles. Six deliberately corrupted inputs were rejected as negative controls.

The seven local models reproduce: a missing final convergence check; an omitted change-activation check; a fencing check-to-use race; over-broad historical deduplication; a lost refresh obligation; incorrect blanket unknown propagation; and a disruption-budget claim invalidated by independent failure.

These are **small deterministic Python examples of draft semantics**. They are not Rust integration tests, exhaustive distributed model checks, Linux failure tests, or proof that the proposed repairs work. The 21 larger experiments remain proposals with hypotheses, rival hypotheses, measurements, and decision rules.

## Query Example

This prints the highest-salience adopt-direction recommendations without installing a graph database:

```python
import json
from pathlib import Path

records = [json.loads(line) for line in
           Path("nomos-research.ndjson").read_text(encoding="utf-8").splitlines()]
items = [r for r in records
         if r.get("node_kind") == "recommendation"
         and r["details"]["disposition"] == "adopt_direction"]
items.sort(key=lambda r: (-r["weights"]["salience"],
                          -r["weights"]["importance"], r["id"]))
for item in items:
    print(item["weights"]["salience"], item["weights"]["importance"],
          item["title"])
```

For graph import, ingest nodes by `id`, then edges by `from` and `to`; preserve the metadata record separately. Resolve each recommendation's `source_ids` to source nodes, and follow `tested_by` or `depends_on` to build review tasks. Preserve uncertainty and disposition when summarizing.

## Limits and Revision Policy

This was a targeted primary-source survey, not a systematic literature review, dependency-version audit, license clearance, or performance comparison. Standards, vendor/maintainer documentation, research papers, and repository text establish different kinds of evidence. Recommendations transfer ideas into Nomos and remain design judgments.

No Nomos Rust build, TLA+ model, Kani harness, Verus proof, or Linux integration test was executed. Proposed target files and ADR topics are not claims that those files exist or decisions were committed.

Preserve this dated snapshot. Revisions should create a new snapshot with explicit changes rather than silently rewriting old evidence. A future cross-snapshot `supersedes` relation requires a schema extension; it is not part of this file's relation vocabulary.
