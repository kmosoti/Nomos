//! Structural and referential checks for a research snapshot's NDJSON graph.
//!
//! These checks establish that the artifact is well formed: strictly parsed,
//! unique identifiers, resolvable references, bounded weights, every
//! recommendation covered by a proposed experiment, no cycle among
//! recommendation dependencies, and metadata counts that match the records.
//! They say nothing about whether the research is right.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::manifest::sha256_hex;
use crate::strict_json;

/// The semver requirement on the `jsonschema` crate. `Cargo.lock` pins the exact version.
const JSONSCHEMA_REQUIREMENT: &str = "0.58";

const CHECKS: [&str; 12] = [
    "strict_utf8_ndjson",
    "no_duplicate_json_keys",
    "no_nonfinite_numbers",
    "unique_ids",
    "weight_bounds",
    "source_resolution",
    "edge_endpoints",
    "no_duplicate_edges",
    "no_isolated_nodes",
    "recommendation_experiment_coverage",
    "acyclic_recommendation_dependencies",
    "metadata_counts",
];

const NOT_ESTABLISHED: [&str; 4] = [
    "Research claims are true",
    "Nomos builds or runs",
    "Formal models have been checked",
    "Proposed experiments have been executed",
];

/// The validation report, shaped like the one the snapshot ships.
#[derive(Serialize)]
pub(crate) struct Report {
    status: &'static str,
    scope: &'static str,
    graph_sha256: String,
    counts: Map<String, Value>,
    checks: [&'static str; 12],
    json_schema: Value,
    negative_controls: Map<String, Value>,
    not_established: [&'static str; 4],
}

impl Report {
    /// The SHA-256 of the graph bytes this report describes.
    #[cfg(test)]
    pub(crate) fn graph_sha256(&self) -> &str {
        &self.graph_sha256
    }

    /// Node-kind, node, edge, and record counts.
    #[cfg(test)]
    pub(crate) fn counts(&self) -> &Map<String, Value> {
        &self.counts
    }
}

fn require(condition: bool, message: impl FnOnce() -> String) -> Result<(), String> {
    if condition { Ok(()) } else { Err(message()) }
}

/// Python truthiness, which the shipped validator relies on for "present and non-empty".
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

fn field<'a>(record: &'a Value, key: &str, at: &str) -> Result<&'a Value, String> {
    record
        .get(key)
        .ok_or_else(|| format!("{at}: missing field '{key}'"))
}

fn text<'a>(record: &'a Value, key: &str, at: &str) -> Result<&'a str, String> {
    field(record, key, at)?
        .as_str()
        .ok_or_else(|| format!("{at}: field '{key}' is not a string"))
}

fn id_list<'a>(record: &'a Value, key: &str, at: &str) -> Result<Vec<&'a str>, String> {
    field(record, key, at)?
        .as_array()
        .ok_or_else(|| format!("{at}: field '{key}' is not a list"))?
        .iter()
        .map(|v| {
            v.as_str()
                .ok_or_else(|| format!("{at}: '{key}' holds a non-string"))
        })
        .collect()
}

fn integer_weight(value: &Value, location: &str) -> Result<(), String> {
    let ok = matches!(value, Value::Number(n) if n.as_u64().is_some_and(|w| w <= 4));
    require(ok, || {
        format!("{location}: expected integer weight in [0,4], got {value}")
    })
}

/// Strict NDJSON parse: no BOM, a final newline, no blank lines, one object per line.
pub(crate) fn parse_text(text: &str) -> Result<Vec<Value>, String> {
    require(!text.starts_with('\u{feff}'), || {
        "A UTF-8 BOM is not permitted".into()
    })?;
    require(text.ends_with('\n'), || {
        "The final NDJSON record needs a newline".into()
    })?;
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        require(!line.trim().is_empty(), || {
            format!("Blank record at line {number}")
        })?;
        let value = strict_json::parse(line).map_err(|e| format!("Line {number}: {e}"))?;
        require(value.is_object(), || {
            format!("Line {number} is not a JSON object")
        })?;
        records.push(value);
    }
    require(!records.is_empty(), || "Graph is empty".into())?;
    Ok(records)
}

/// Every cross-record invariant. Returns the counts derived from the records.
pub(crate) fn check_records(records: &[Value]) -> Result<Map<String, Value>, String> {
    let metadata = &records[0];
    require(
        metadata.get("record_type") == Some(&json!("metadata")),
        || "Metadata must be first".into(),
    )?;
    let metadata_count = records
        .iter()
        .filter(|r| r.get("record_type") == Some(&json!("metadata")))
        .count();
    require(metadata_count == 1, || {
        "Exactly one metadata record is required".into()
    })?;

    let mut ids = Vec::with_capacity(records.len());
    for record in records {
        let id = record.get("id").and_then(Value::as_str).unwrap_or("");
        require(!id.is_empty(), || "Every record needs an ID".into())?;
        ids.push(id);
    }
    let unique: HashSet<&str> = ids.iter().copied().collect();
    require(unique.len() == ids.len(), || "Duplicate record ID".into())?;
    for record in records {
        let kind = record
            .get("record_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        require(matches!(kind, "metadata" | "node" | "edge"), || {
            format!("Unknown record type: {kind}")
        })?;
    }

    let node_list: Vec<&Value> = records
        .iter()
        .filter(|r| r["record_type"] == "node")
        .collect();
    let nodes: HashMap<&str, &Value> = node_list
        .iter()
        .map(|n| (n["id"].as_str().unwrap_or(""), *n))
        .collect();
    let edges: Vec<&Value> = records
        .iter()
        .filter(|r| r["record_type"] == "edge")
        .collect();
    let kind_set = |kind: &str| -> HashSet<&str> {
        nodes
            .iter()
            .filter(|(_, n)| n.get("node_kind") == Some(&json!(kind)))
            .map(|(id, _)| *id)
            .collect()
    };
    let sources = kind_set("source");
    let experiments = kind_set("experiment");
    let recommendations = kind_set("recommendation");

    for node in &node_list {
        let id = node["id"].as_str().unwrap_or("");
        let weights = field(node, "weights", id)?;
        for key in ["importance", "salience"] {
            integer_weight(field(weights, key, id)?, &format!("{id}.{key}"))?;
        }
        require(truthy(weights.get("rationale")), || {
            format!("Missing rationale: {id}")
        })?;
        let source_ids = id_list(node, "source_ids", id)?;
        require(source_ids.iter().all(|s| sources.contains(s)), || {
            format!("Missing/non-source reference at {id}")
        })?;
        let details = field(node, "details", id)?;
        match text(node, "node_kind", id)? {
            "source" => require(
                truthy(details.get("url")) && truthy(details.get("locator")),
                || format!("Missing source URL/locator: {id}"),
            )?,
            "recommendation" => {
                let experiment_ids = id_list(details, "experiment_ids", id)?;
                require(!experiment_ids.is_empty(), || {
                    format!("Recommendation has no proposed check: {id}")
                })?;
                require(
                    experiment_ids.iter().all(|e| experiments.contains(e)),
                    || format!("Unresolved experiment at {id}"),
                )?;
                require(truthy(details.get("acceptance_criteria")), || {
                    format!("No acceptance criteria: {id}")
                })?;
            }
            "open_question" | "adr_candidate" => {
                let recommendation_ids = id_list(details, "recommendation_ids", id)?;
                require(
                    recommendation_ids
                        .iter()
                        .all(|r| recommendations.contains(r)),
                    || format!("Unresolved recommendation at {id}"),
                )?;
            }
            "experiment" => {
                let proposal_only = field(details, "execution_status", id)? == "not_run"
                    && field(details, "results", id)?.is_null();
                require(proposal_only, || {
                    format!("This snapshot labels experiments as proposals: {id}")
                })?;
            }
            "counterexample" => require(
                details.get("scope").is_some() && details.get("reproducer").is_some(),
                || format!("Counterexample needs scope and reproducer: {id}"),
            )?,
            _ => {}
        }
    }

    let mut triples: HashSet<(&str, &str, &str)> = HashSet::new();
    let mut dependency_edges: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut degree: HashMap<&str, usize> = HashMap::new();
    let mut tested_pairs: HashSet<(&str, &str)> = HashSet::new();
    for edge in &edges {
        let id = edge["id"].as_str().unwrap_or("");
        let from = text(edge, "from", id)?;
        let to = text(edge, "to", id)?;
        require(nodes.contains_key(from) && nodes.contains_key(to), || {
            format!("Dangling graph endpoint: {id}")
        })?;
        require(from != to, || format!("Self-loop: {id}"))?;
        let evidence = id_list(edge, "evidence_ids", id)?;
        require(evidence.iter().all(|e| nodes.contains_key(e)), || {
            format!("Dangling evidence reference: {id}")
        })?;
        integer_weight(
            field(edge, "semantic_weight", id)?,
            &format!("{id}.semantic_weight"),
        )?;
        let relation = text(edge, "relation", id)?;
        require(triples.insert((from, relation, to)), || {
            format!("Duplicate relationship: ('{from}', '{relation}', '{to}')")
        })?;
        *degree.entry(from).or_default() += 1;
        *degree.entry(to).or_default() += 1;
        match relation {
            "tested_by" => {
                tested_pairs.insert((from, to));
            }
            "depends_on" => {
                require(
                    recommendations.contains(from) && recommendations.contains(to),
                    || "Recommendation dependency endpoints have incorrect kinds".into(),
                )?;
                dependency_edges.entry(from).or_default().push(to);
            }
            _ => {}
        }
    }
    for rid in &recommendations {
        for eid in id_list(&nodes[rid]["details"], "experiment_ids", rid)? {
            require(tested_pairs.contains(&(rid, eid)), || {
                format!("Missing tested_by edge: {rid} -> {eid}")
            })?;
        }
    }
    require(
        nodes
            .keys()
            .all(|id| degree.get(id).copied().unwrap_or(0) > 0),
        || "Graph contains an isolated node".into(),
    )?;

    // Only the recommendation dependency subgraph must be acyclic.
    let mut active: HashSet<&str> = HashSet::new();
    let mut finished: HashSet<&str> = HashSet::new();
    fn visit<'a>(
        current: &'a str,
        edges: &HashMap<&'a str, Vec<&'a str>>,
        active: &mut HashSet<&'a str>,
        finished: &mut HashSet<&'a str>,
    ) -> Result<(), String> {
        require(!active.contains(current), || {
            "Cycle in recommendation dependencies".into()
        })?;
        if finished.contains(current) {
            return Ok(());
        }
        active.insert(current);
        for prerequisite in edges.get(current).into_iter().flatten() {
            visit(prerequisite, edges, active, finished)?;
        }
        active.remove(current);
        finished.insert(current);
        Ok(())
    }
    let mut ordered: Vec<&str> = recommendations.iter().copied().collect();
    ordered.sort_unstable();
    for rid in ordered {
        visit(rid, &dependency_edges, &mut active, &mut finished)?;
    }

    let mut counts = Map::new();
    for node in &node_list {
        let kind = node["node_kind"].as_str().unwrap_or("");
        let entry = counts.entry(kind).or_insert_with(|| json!(0));
        *entry = json!(entry.as_u64().unwrap_or(0) + 1);
    }
    counts.insert("nodes".into(), json!(node_list.len()));
    counts.insert("edges".into(), json!(edges.len()));
    counts.insert("records".into(), json!(records.len()));
    require(
        metadata.get("counts") == Some(&Value::Object(counts.clone())),
        || "Metadata counts do not match records".into(),
    )?;
    Ok(counts)
}

/// Deliberately corrupted copies that the checks must reject.
pub(crate) fn negative_controls(records: &[Value]) -> Result<Map<String, Value>, String> {
    let mut outcomes = Map::new();
    let mut cases: Vec<(&str, Vec<Value>)> = Vec::new();

    let mut bad = records.to_vec();
    if let Some(edge) = bad.iter_mut().find(|r| r["record_type"] == "edge") {
        edge["to"] = json!("urn:missing:endpoint");
    }
    cases.push(("dangling_endpoint", bad));

    let mut bad = records.to_vec();
    if let Some(node) = bad.iter_mut().find(|r| r["record_type"] == "node") {
        node["weights"]["salience"] = json!(5);
    }
    cases.push(("out_of_range_weight", bad));

    let mut bad = records.to_vec();
    if bad.len() > 2 {
        let first = bad[1]["id"].clone();
        bad[2]["id"] = first;
    }
    cases.push(("duplicate_id", bad));

    for (name, bad) in cases {
        require(check_records(&bad).is_err(), || {
            format!("Negative control accepted: {name}")
        })?;
        outcomes.insert(name.into(), json!("rejected_as_expected"));
    }
    for (name, corrupt) in [
        ("duplicate_json_key", "{\"x\":1,\"x\":2}\n"),
        ("non_finite_number", "{\"x\":NaN}\n"),
        ("blank_record", "{}\n\n"),
    ] {
        require(parse_text(corrupt).is_err(), || {
            format!("Parser negative control accepted: {name}")
        })?;
        outcomes.insert(name.into(), json!("rejected_as_expected"));
    }
    Ok(outcomes)
}

fn validate_schema(records: &[Value], schema_path: &Path) -> Result<Value, String> {
    let bytes =
        std::fs::read(schema_path).map_err(|e| format!("{}: {e}", schema_path.display()))?;
    let schema: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", schema_path.display()))?;
    jsonschema::meta::validate(&schema).map_err(|e| format!("Invalid JSON Schema: {e}"))?;
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .map_err(|e| format!("Invalid JSON Schema: {e}"))?;
    for (index, record) in records.iter().enumerate() {
        let errors: Vec<String> = validator
            .iter_errors(record)
            .map(|e| {
                // The Display form embeds the whole instance. Name the record and the
                // schema location instead; the record is on the line the message gives.
                let full = e.to_string();
                let brief = full
                    .rsplit_once(" is not valid ")
                    .map_or(full.as_str(), |(_, tail)| tail);
                format!("not valid {brief} (schema path {})", e.schema_path())
            })
            .collect();
        require(errors.is_empty(), || {
            let id = record["id"].as_str().unwrap_or("?");
            format!(
                "JSON Schema failure at line {} ({id}): {}",
                index + 1,
                errors.join("; ")
            )
        })?;
    }
    Ok(json!({
        "status": "passed",
        "records_validated": records.len(),
        "validator": "jsonschema (Rust crate)",
        "validator_version": JSONSCHEMA_REQUIREMENT,
        "schema_sha256": sha256_hex(&bytes),
    }))
}

/// Runs every check on a graph file and returns the report.
pub(crate) fn verify(graph_path: &Path, schema_path: Option<&Path>) -> Result<Report, String> {
    let bytes = std::fs::read(graph_path).map_err(|e| format!("{}: {e}", graph_path.display()))?;
    let text =
        String::from_utf8(bytes.clone()).map_err(|e| format!("{}: {e}", graph_path.display()))?;
    let records = parse_text(&text)?;
    let counts = check_records(&records)?;
    let json_schema = match schema_path {
        Some(path) => validate_schema(&records, path)?,
        None => json!({ "status": "not_requested" }),
    };
    let negative_controls = negative_controls(&records)?;
    Ok(Report {
        status: "passed",
        scope: "Artifact integrity, not Nomos correctness",
        graph_sha256: sha256_hex(&bytes),
        counts,
        checks: CHECKS,
        json_schema,
        negative_controls,
        not_established: NOT_ESTABLISHED,
    })
}

/// Recommendations in review order: salience, then importance, descending; then ID.
pub(crate) fn recommendation_index(graph_path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(graph_path)
        .map_err(|e| format!("{}: {e}", graph_path.display()))?;
    let records = parse_text(&text)?;
    let mut items: Vec<&Value> = records
        .iter()
        .filter(|r| r.get("node_kind") == Some(&json!("recommendation")))
        .collect();
    let weight = |r: &Value, key: &str| r["weights"][key].as_u64().unwrap_or(0);
    items.sort_by(|a, b| {
        weight(b, "salience")
            .cmp(&weight(a, "salience"))
            .then(weight(b, "importance").cmp(&weight(a, "importance")))
            .then(a["id"].as_str().cmp(&b["id"].as_str()))
    });
    Ok(items
        .into_iter()
        .map(|r| {
            let d = &r["details"];
            format!(
                "[{}/{}] {:<20} {:<26} {}",
                weight(r, "salience"),
                weight(r, "importance"),
                d["disposition"].as_str().unwrap_or("?"),
                d["phase"].as_str().unwrap_or("?"),
                r["title"].as_str().unwrap_or("?"),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::{check_records, parse_text, verify};

    fn snapshot() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/research/2026-09-28-typed-core/snapshot")
    }

    #[test]
    fn shipped_snapshot_verifies_and_matches_its_own_report() {
        let dir = snapshot();
        let report = verify(
            &dir.join("nomos-research.ndjson"),
            Some(&dir.join("record.schema.json")),
        )
        .unwrap();
        let shipped: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("validation-report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            report.graph_sha256(),
            shipped["graph_sha256"].as_str().unwrap()
        );
        assert_eq!(Value::Object(report.counts().clone()), shipped["counts"]);
        assert_eq!(shipped["negative_controls"].as_object().unwrap().len(), 6);
    }

    #[test]
    fn parser_rejects_the_documented_corruptions() {
        assert!(parse_text("\u{feff}{}\n").is_err());
        assert!(parse_text("{}").is_err());
        assert!(parse_text("{}\n\n").is_err());
        assert!(parse_text("[]\n").is_err());
        assert!(parse_text("").is_err());
    }

    #[test]
    fn metadata_must_come_first() {
        let records = parse_text("{\"record_type\":\"node\",\"id\":\"a\"}\n").unwrap();
        assert_eq!(
            check_records(&records).unwrap_err(),
            "Metadata must be first"
        );
    }
}
