//! Structural and referential checks for a research snapshot's NDJSON graph.
//!
//! These checks establish that the artifact is well formed: strictly parsed,
//! unique identifiers, resolvable references, bounded weights, every
//! recommendation covered by a proposed experiment, no cycle among
//! recommendation dependencies, metadata counts that match the records, and
//! every record valid against the snapshot's JSON Schema. They say nothing
//! about whether the research is right.
//!
//! Every failure carries a stable [`Code`]. The built-in negative controls
//! check that each deliberately corrupted input is rejected *for its reason*,
//! not merely rejected.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::error::{Code, VerificationError, Verified, expect_rejection, require};
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

/// The graph stage's report, shaped like the validation report the snapshot ships.
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

    /// Negative-control outcomes, by name.
    #[cfg(test)]
    pub(crate) fn negative_controls(&self) -> &Map<String, Value> {
        &self.negative_controls
    }
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

fn field<'a>(record: &'a Value, key: &str, at: &str) -> Verified<&'a Value> {
    record.get(key).ok_or_else(|| {
        VerificationError::new(Code::MissingField, format!("{at}: missing field '{key}'"))
    })
}

fn text<'a>(record: &'a Value, key: &str, at: &str) -> Verified<&'a str> {
    field(record, key, at)?.as_str().ok_or_else(|| {
        VerificationError::new(
            Code::WrongFieldType,
            format!("{at}: field '{key}' is not a string"),
        )
    })
}

fn id_list<'a>(record: &'a Value, key: &str, at: &str) -> Verified<Vec<&'a str>> {
    field(record, key, at)?
        .as_array()
        .ok_or_else(|| {
            VerificationError::new(
                Code::WrongFieldType,
                format!("{at}: field '{key}' is not a list"),
            )
        })?
        .iter()
        .map(|v| {
            v.as_str().ok_or_else(|| {
                VerificationError::new(
                    Code::WrongFieldType,
                    format!("{at}: '{key}' holds a non-string"),
                )
            })
        })
        .collect()
}

fn integer_weight(value: &Value, location: &str) -> Verified<()> {
    let ok = matches!(value, Value::Number(n) if n.as_u64().is_some_and(|w| w <= 4));
    require(ok, Code::WeightOutOfRange, || {
        format!("{location}: expected integer weight in [0,4], got {value}")
    })
}

/// Strict NDJSON parse: no BOM, a final newline, no blank lines, one object per line.
pub(crate) fn parse_text(text: &str) -> Verified<Vec<Value>> {
    require(!text.starts_with('\u{feff}'), Code::NdjsonBom, || {
        "A UTF-8 BOM is not permitted".into()
    })?;
    require(!text.is_empty(), Code::NdjsonEmpty, || {
        "Graph is empty".into()
    })?;
    require(text.ends_with('\n'), Code::NdjsonNoFinalNewline, || {
        "The final NDJSON record needs a newline".into()
    })?;
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        require(!line.trim().is_empty(), Code::NdjsonBlankRecord, || {
            format!("Blank record at line {number}")
        })?;
        let value = strict_json::parse(line)
            .map_err(|e| VerificationError::new(e.code(), format!("Line {number}: {e}")))?;
        require(value.is_object(), Code::NdjsonNotObject, || {
            format!("Line {number} is not a JSON object")
        })?;
        records.push(value);
    }
    Ok(records)
}

/// Every cross-record invariant. Returns the counts derived from the records.
pub(crate) fn check_records(records: &[Value]) -> Verified<Map<String, Value>> {
    let metadata = &records[0];
    require(
        metadata.get("record_type") == Some(&json!("metadata")),
        Code::MetadataNotFirst,
        || "Metadata must be first".into(),
    )?;
    let metadata_count = records
        .iter()
        .filter(|r| r.get("record_type") == Some(&json!("metadata")))
        .count();
    require(metadata_count == 1, Code::MetadataCount, || {
        "Exactly one metadata record is required".into()
    })?;

    let mut ids = Vec::with_capacity(records.len());
    for record in records {
        let id = record.get("id").and_then(Value::as_str).unwrap_or("");
        require(!id.is_empty(), Code::MissingId, || {
            "Every record needs an ID".into()
        })?;
        ids.push(id);
    }
    let mut unique: HashSet<&str> = HashSet::new();
    for id in &ids {
        require(unique.insert(id), Code::DuplicateId, || {
            format!("Duplicate record ID: {id}")
        })?;
    }
    for record in records {
        let kind = record
            .get("record_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        require(
            matches!(kind, "metadata" | "node" | "edge"),
            Code::UnknownRecordType,
            || format!("Unknown record type: {kind}"),
        )?;
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
        require(
            truthy(weights.get("rationale")),
            Code::MissingRationale,
            || format!("Missing rationale: {id}"),
        )?;
        let source_ids = id_list(node, "source_ids", id)?;
        require(
            source_ids.iter().all(|s| sources.contains(s)),
            Code::UnresolvedSource,
            || format!("Missing/non-source reference at {id}"),
        )?;
        let details = field(node, "details", id)?;
        match text(node, "node_kind", id)? {
            "source" => require(
                truthy(details.get("url")) && truthy(details.get("locator")),
                Code::SourceWithoutLocator,
                || format!("Missing source URL/locator: {id}"),
            )?,
            "recommendation" => {
                let experiment_ids = id_list(details, "experiment_ids", id)?;
                require(
                    !experiment_ids.is_empty(),
                    Code::RecommendationWithoutExperiment,
                    || format!("Recommendation has no proposed check: {id}"),
                )?;
                require(
                    experiment_ids.iter().all(|e| experiments.contains(e)),
                    Code::UnresolvedExperiment,
                    || format!("Unresolved experiment at {id}"),
                )?;
                require(
                    truthy(details.get("acceptance_criteria")),
                    Code::MissingAcceptanceCriteria,
                    || format!("No acceptance criteria: {id}"),
                )?;
            }
            "open_question" | "adr_candidate" => {
                let recommendation_ids = id_list(details, "recommendation_ids", id)?;
                require(
                    recommendation_ids
                        .iter()
                        .all(|r| recommendations.contains(r)),
                    Code::UnresolvedRecommendation,
                    || format!("Unresolved recommendation at {id}"),
                )?;
            }
            "experiment" => {
                let proposal_only = field(details, "execution_status", id)? == "not_run"
                    && field(details, "results", id)?.is_null();
                require(proposal_only, Code::ExperimentNotProposal, || {
                    format!("This snapshot labels experiments as proposals: {id}")
                })?;
            }
            "counterexample" => require(
                details.get("scope").is_some() && details.get("reproducer").is_some(),
                Code::CounterexampleIncomplete,
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
        require(
            nodes.contains_key(from) && nodes.contains_key(to),
            Code::DanglingEndpoint,
            || format!("Dangling graph endpoint: {id}"),
        )?;
        require(from != to, Code::SelfLoop, || format!("Self-loop: {id}"))?;
        let evidence = id_list(edge, "evidence_ids", id)?;
        require(
            evidence.iter().all(|e| nodes.contains_key(e)),
            Code::DanglingEvidence,
            || format!("Dangling evidence reference: {id}"),
        )?;
        integer_weight(
            field(edge, "semantic_weight", id)?,
            &format!("{id}.semantic_weight"),
        )?;
        let relation = text(edge, "relation", id)?;
        require(
            triples.insert((from, relation, to)),
            Code::DuplicateEdge,
            || format!("Duplicate relationship: ('{from}', '{relation}', '{to}')"),
        )?;
        *degree.entry(from).or_default() += 1;
        *degree.entry(to).or_default() += 1;
        match relation {
            "tested_by" => {
                tested_pairs.insert((from, to));
            }
            "depends_on" => {
                require(
                    recommendations.contains(from) && recommendations.contains(to),
                    Code::DependencyEndpointKind,
                    || "Recommendation dependency endpoints have incorrect kinds".into(),
                )?;
                dependency_edges.entry(from).or_default().push(to);
            }
            _ => {}
        }
    }
    let mut ordered: Vec<&str> = recommendations.iter().copied().collect();
    ordered.sort_unstable();
    for rid in &ordered {
        for eid in id_list(&nodes[rid]["details"], "experiment_ids", rid)? {
            require(
                tested_pairs.contains(&(*rid, eid)),
                Code::MissingTestedBy,
                || format!("Missing tested_by edge: {rid} -> {eid}"),
            )?;
        }
    }
    let mut isolated: Vec<&str> = nodes
        .keys()
        .copied()
        .filter(|id| degree.get(id).copied().unwrap_or(0) == 0)
        .collect();
    isolated.sort_unstable();
    require(isolated.is_empty(), Code::IsolatedNode, || {
        format!("Graph contains an isolated node: {}", isolated.join(", "))
    })?;

    // Only the recommendation dependency subgraph must be acyclic.
    let mut active: HashSet<&str> = HashSet::new();
    let mut finished: HashSet<&str> = HashSet::new();
    fn visit<'a>(
        current: &'a str,
        edges: &HashMap<&'a str, Vec<&'a str>>,
        active: &mut HashSet<&'a str>,
        finished: &mut HashSet<&'a str>,
    ) -> Verified<()> {
        require(!active.contains(current), Code::DependencyCycle, || {
            format!("Cycle in recommendation dependencies through {current}")
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
        Code::CountsMismatch,
        || "Metadata counts do not match records".into(),
    )?;
    Ok(counts)
}

/// Deliberately corrupted copies that the checks must reject, each for its own reason.
pub(crate) fn negative_controls(records: &[Value]) -> Verified<Map<String, Value>> {
    let mut outcomes = Map::new();
    let mut cases: Vec<(&str, Vec<Value>, Code)> = Vec::new();

    let mut bad = records.to_vec();
    if let Some(edge) = bad.iter_mut().find(|r| r["record_type"] == "edge") {
        edge["to"] = json!("urn:missing:endpoint");
    }
    cases.push(("dangling_endpoint", bad, Code::DanglingEndpoint));

    let mut bad = records.to_vec();
    if let Some(node) = bad.iter_mut().find(|r| r["record_type"] == "node") {
        node["weights"]["salience"] = json!(5);
    }
    cases.push(("out_of_range_weight", bad, Code::WeightOutOfRange));

    let mut bad = records.to_vec();
    if bad.len() > 2 {
        let first = bad[1]["id"].clone();
        bad[2]["id"] = first;
    }
    cases.push(("duplicate_id", bad, Code::DuplicateId));

    for (name, bad, expected) in cases {
        expect_rejection(name, check_records(&bad), expected)?;
        outcomes.insert(name.into(), json!("rejected_as_expected"));
    }
    for (name, corrupt, expected) in [
        (
            "duplicate_json_key",
            "{\"x\":1,\"x\":2}\n",
            Code::JsonDuplicateKey,
        ),
        (
            "non_finite_number",
            "{\"x\":NaN}\n",
            Code::JsonNonFiniteNumber,
        ),
        ("blank_record", "{}\n\n", Code::NdjsonBlankRecord),
    ] {
        expect_rejection(name, parse_text(corrupt), expected)?;
        outcomes.insert(name.into(), json!("rejected_as_expected"));
    }
    Ok(outcomes)
}

fn validate_schema(records: &[Value], schema_path: &Path) -> Verified<Value> {
    let bytes = std::fs::read(schema_path).map_err(|e| {
        VerificationError::new(
            Code::SchemaUnreadable,
            format!("{}: {e}", schema_path.display()),
        )
    })?;
    let schema: Value = serde_json::from_slice(&bytes).map_err(|e| {
        VerificationError::new(
            Code::SchemaInvalid,
            format!("{}: {e}", schema_path.display()),
        )
    })?;
    jsonschema::meta::validate(&schema).map_err(|e| {
        VerificationError::new(Code::SchemaInvalid, format!("Invalid JSON Schema: {e}"))
    })?;
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .map_err(|e| {
            VerificationError::new(Code::SchemaInvalid, format!("Invalid JSON Schema: {e}"))
        })?;
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
        require(errors.is_empty(), Code::SchemaViolation, || {
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

/// Runs every graph-stage check, schema validation included. There is no
/// weaker mode: a snapshot either passes all of it or fails.
pub(crate) fn verify(graph_path: &Path, schema_path: &Path) -> Verified<Report> {
    let bytes = std::fs::read(graph_path).map_err(|e| {
        VerificationError::new(
            Code::GraphUnreadable,
            format!("{}: {e}", graph_path.display()),
        )
    })?;
    let text = String::from_utf8(bytes.clone()).map_err(|e| {
        VerificationError::new(
            Code::GraphUnreadable,
            format!("{}: {e}", graph_path.display()),
        )
    })?;
    let records = parse_text(&text)?;
    let counts = check_records(&records)?;
    let json_schema = validate_schema(&records, schema_path)?;
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
pub(crate) fn recommendation_index(graph_path: &Path) -> Verified<Vec<String>> {
    let text = std::fs::read_to_string(graph_path).map_err(|e| {
        VerificationError::new(
            Code::GraphUnreadable,
            format!("{}: {e}", graph_path.display()),
        )
    })?;
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
    use super::{check_records, parse_text};
    use crate::error::Code;

    fn parse_code(text: &str) -> Code {
        parse_text(text).expect_err("expected a rejection").code()
    }

    #[test]
    fn the_parser_names_each_documented_corruption() {
        assert_eq!(parse_code("\u{feff}{}\n"), Code::NdjsonBom);
        assert_eq!(parse_code("{}"), Code::NdjsonNoFinalNewline);
        assert_eq!(parse_code("{}\n\n"), Code::NdjsonBlankRecord);
        assert_eq!(parse_code("[]\n"), Code::NdjsonNotObject);
        assert_eq!(parse_code(""), Code::NdjsonEmpty);
        assert_eq!(parse_code("{\"a\":1,\"a\":2}\n"), Code::JsonDuplicateKey);
        assert_eq!(parse_code("{\"a\":NaN}\n"), Code::JsonNonFiniteNumber);
        assert_eq!(parse_code("{\"a\":}\n"), Code::JsonSyntax);
    }

    #[test]
    fn metadata_must_come_first() {
        let records = parse_text("{\"record_type\":\"node\",\"id\":\"a\"}\n").unwrap();
        let error = check_records(&records).unwrap_err();
        assert_eq!(error.code(), Code::MetadataNotFirst);
    }
}
