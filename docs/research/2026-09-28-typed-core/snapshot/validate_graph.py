#!/usr/bin/env python3
"""Validate a Nomos NDJSON research graph, not its research conclusions.

Python 3.10+; standard-library checks always run. Full per-record JSON Schema
validation is enabled by --require-schema and requires the jsonschema package.
No source code, network request, or Nomos execution is performed.
"""
from __future__ import annotations

import argparse
import collections
import copy
import hashlib
import importlib.metadata
import json
import sys
from pathlib import Path
from typing import Any


class GraphError(ValueError):
    """A graph-format, reference, or metadata invariant failed."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise GraphError(message)


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        require(key not in value, f"Duplicate JSON object key: {key}")
        value[key] = item
    return value


def reject_constant(value: str) -> None:
    raise GraphError(f"Non-finite JSON constant: {value}")


def parse_text(text: str) -> list[dict[str, Any]]:
    require(not text.startswith('\ufeff'), 'A UTF-8 BOM is not permitted')
    require(text.endswith('\n'), 'The final NDJSON record needs a newline')
    records: list[dict[str, Any]] = []
    for number, line in enumerate(text.splitlines(), 1):
        require(bool(line.strip()), f'Blank record at line {number}')
        try:
            value = json.loads(line, object_pairs_hook=unique_object,
                               parse_constant=reject_constant)
        except (ValueError, TypeError) as error:
            raise GraphError(f'Line {number}: {error}') from error
        require(isinstance(value, dict), f'Line {number} is not a JSON object')
        records.append(value)
    require(bool(records), 'Graph is empty')
    return records


def integer_weight(value: Any, location: str) -> None:
    require(type(value) is int and 0 <= value <= 4,
            f'{location}: expected integer weight in [0,4], got {value!r}')


def check_records(records: list[dict[str, Any]]) -> dict[str, Any]:
    require(records[0].get('record_type') == 'metadata', 'Metadata must be first')
    require(sum(r.get('record_type') == 'metadata' for r in records) == 1,
            'Exactly one metadata record is required')
    ids = [r.get('id') for r in records]
    require(all(isinstance(i, str) and i for i in ids), 'Every record needs an ID')
    require(len(ids) == len(set(ids)), 'Duplicate record ID')
    for record in records:
        require(record.get('record_type') in {'metadata', 'node', 'edge'},
                f"Unknown record type: {record.get('record_type')}")
    nodes = {r['id']: r for r in records if r['record_type'] == 'node'}
    edges = [r for r in records if r['record_type'] == 'edge']
    metadata = records[0]
    sources = {k for k, n in nodes.items() if n['node_kind'] == 'source'}
    experiments = {k for k, n in nodes.items() if n['node_kind'] == 'experiment'}
    recommendations = {k for k, n in nodes.items()
                       if n['node_kind'] == 'recommendation'}
    triples: set[tuple[str, str, str]] = set()
    dependency_edges: dict[str, list[str]] = collections.defaultdict(list)
    degree: collections.Counter[str] = collections.Counter()
    tested_pairs: set[tuple[str, str]] = set()

    for n in nodes.values():
        for key in ('importance', 'salience'):
            integer_weight(n['weights'][key], f"{n['id']}.{key}")
        require(bool(n['weights']['rationale']), f"Missing rationale: {n['id']}")
        require(set(n['source_ids']).issubset(sources),
                f"Missing/non-source reference at {n['id']}")
        details = n['details']
        kind = n['node_kind']
        if kind == 'source':
            require(bool(details.get('url')) and bool(details.get('locator')),
                    f"Missing source URL/locator: {n['id']}")
        if kind == 'recommendation':
            require(bool(details['experiment_ids']),
                    f"Recommendation has no proposed check: {n['id']}")
            require(set(details['experiment_ids']).issubset(experiments),
                    f"Unresolved experiment at {n['id']}")
            require(bool(details['acceptance_criteria']),
                    f"No acceptance criteria: {n['id']}")
        if kind in {'open_question', 'adr_candidate'}:
            require(set(details['recommendation_ids']).issubset(recommendations),
                    f"Unresolved recommendation at {n['id']}")
        if kind == 'experiment':
            require(details['execution_status'] == 'not_run'
                    and details['results'] is None,
                    f"This snapshot labels experiments as proposals: {n['id']}")
        if kind == 'counterexample':
            require('scope' in details and 'reproducer' in details,
                    f"Counterexample needs scope and reproducer: {n['id']}")

    for e in edges:
        require(e['from'] in nodes and e['to'] in nodes,
                f"Dangling graph endpoint: {e['id']}")
        require(e['from'] != e['to'], f"Self-loop: {e['id']}")
        require(set(e['evidence_ids']).issubset(nodes),
                f"Dangling evidence reference: {e['id']}")
        integer_weight(e['semantic_weight'], f"{e['id']}.semantic_weight")
        triple = (e['from'], e['relation'], e['to'])
        require(triple not in triples, f'Duplicate relationship: {triple}')
        triples.add(triple)
        degree.update([e['from'], e['to']])
        if e['relation'] == 'tested_by':
            tested_pairs.add((e['from'], e['to']))
        if e['relation'] == 'depends_on':
            require(e['from'] in recommendations and e['to'] in recommendations,
                    'Recommendation dependency endpoints have incorrect kinds')
            dependency_edges[e['from']].append(e['to'])
    for rid in recommendations:
        for eid in nodes[rid]['details']['experiment_ids']:
            require((rid, eid) in tested_pairs,
                    f'Missing tested_by edge: {rid} -> {eid}')
    require(all(degree[i] > 0 for i in nodes), 'Graph contains an isolated node')

    # Only the recommendation dependency subgraph must be acyclic.
    active: set[str] = set()
    finished: set[str] = set()
    def visit(current: str) -> None:
        require(current not in active, 'Cycle in recommendation dependencies')
        if current in finished:
            return
        active.add(current)
        for prerequisite in dependency_edges[current]:
            visit(prerequisite)
        active.remove(current)
        finished.add(current)
    for rid in recommendations:
        visit(rid)

    counts = dict(collections.Counter(n['node_kind'] for n in nodes.values()),
                  nodes=len(nodes), edges=len(edges), records=len(records))
    require(metadata['counts'] == counts, 'Metadata counts do not match records')
    return counts


def negative_controls(records: list[dict[str, Any]]) -> dict[str, str]:
    """Check that the validator rejects selected deliberately corrupted copies."""
    cases: dict[str, list[dict[str, Any]]] = {}
    bad = copy.deepcopy(records)
    edge = next(r for r in bad if r['record_type'] == 'edge')
    edge['to'] = 'urn:missing:endpoint'
    cases['dangling_endpoint'] = bad
    bad = copy.deepcopy(records)
    next(r for r in bad if r['record_type'] == 'node')['weights']['salience'] = 5
    cases['out_of_range_weight'] = bad
    bad = copy.deepcopy(records)
    bad[2]['id'] = bad[1]['id']
    cases['duplicate_id'] = bad
    outcomes = {}
    for name, bad in cases.items():
        try:
            check_records(bad)
        except GraphError:
            outcomes[name] = 'rejected_as_expected'
        else:
            raise GraphError(f'Negative control accepted: {name}')
    for name, text in {
        'duplicate_json_key': '{"x":1,"x":2}\n',
        'non_finite_number': '{"x":NaN}\n',
        'blank_record': '{}\n\n',
    }.items():
        try:
            parse_text(text)
        except GraphError:
            outcomes[name] = 'rejected_as_expected'
        else:
            raise GraphError(f'Parser negative control accepted: {name}')
    return outcomes


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('graph', nargs='?', type=Path,
                        default=Path(__file__).with_name('nomos-research.ndjson'))
    parser.add_argument('--schema', type=Path, default=None)
    parser.add_argument('--require-schema', action='store_true')
    parser.add_argument('--report', type=Path, default=None)
    args = parser.parse_args()
    try:
        raw = args.graph.read_bytes()
        records = parse_text(raw.decode('utf-8'))
        counts = check_records(records)
        schema_result: dict[str, Any] = {'status': 'not_requested'}
        if args.require_schema or args.schema is not None:
            try:
                import jsonschema
            except ImportError as error:
                raise GraphError('Full schema validation requires jsonschema') from error
            schema_path = args.schema or args.graph.with_name('record.schema.json')
            schema = json.loads(schema_path.read_text(encoding='utf-8'))
            jsonschema.Draft202012Validator.check_schema(schema)
            validator = jsonschema.Draft202012Validator(
                schema, format_checker=jsonschema.FormatChecker())
            for line, record in enumerate(records, 1):
                errors = sorted(validator.iter_errors(record), key=lambda x: str(x.path))
                require(not errors, f'JSON Schema failure at line {line}: {errors}')
            schema_result = {'status': 'passed', 'records_validated': len(records),
                             'validator': 'jsonschema',
                             'validator_version': importlib.metadata.version('jsonschema'),
                             'schema_sha256': hashlib.sha256(schema_path.read_bytes()).hexdigest()}
        report = {
            'status': 'passed', 'scope': 'Artifact integrity, not Nomos correctness',
            'graph_sha256': hashlib.sha256(raw).hexdigest(), 'counts': counts,
            'checks': ['strict_utf8_ndjson', 'no_duplicate_json_keys',
                       'no_nonfinite_numbers', 'unique_ids', 'weight_bounds',
                       'source_resolution', 'edge_endpoints', 'no_duplicate_edges',
                       'no_isolated_nodes', 'recommendation_experiment_coverage',
                       'acyclic_recommendation_dependencies', 'metadata_counts'],
            'json_schema': schema_result,
            'negative_controls': negative_controls(records),
            'not_established': ['Research claims are true', 'Nomos builds or runs',
                                'Formal models have been checked',
                                'Proposed experiments have been executed'],
        }
        rendered = json.dumps(report, indent=2, allow_nan=False) + '\n'
        if args.report:
            args.report.write_text(rendered, encoding='utf-8')
        print(rendered, end='')
        return 0
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f'Validation failed: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
