#!/usr/bin/env python3
"""Emit synthetic public DTOs using the observed Python MCP formatter.

Run inside the observed Graphiti image; imports never initialize its server or
call providers. Output contains only constants from this script.
"""
import ast
import asyncio
import datetime
import hashlib
import json
import sys
from pathlib import Path
from types import SimpleNamespace
from typing import Any

sys.path[:0] = [
    "/app/mcp/.venv/lib/python3.11/site-packages",
    "/app/mcp/src",
]
from graphiti_core.edges import EntityEdge
from graphiti_core.nodes import EntityNode
from utils.formatting import format_fact_result
from models.response_types import NodeResult, NodeSearchResponse, ErrorResponse
from graphiti_core.search.search_filters import SearchFilters
from graphiti_core.helpers import validate_group_id
sys.path.insert(0, '/tmp/read-source')
from group_read_routing import normalize_group_ids, ensure_readable_group, GroupScopeViolation

# Execute the deployed group-aware wrapper, not the unused generic node formatter.
source_path = Path('/tmp/read-source/hermes_main.py')
source = source_path.read_text()
wrapper = next(n for n in ast.parse(source).body if isinstance(n, ast.AsyncFunctionDef)
               and n.name == '_search_nodes_group_aware')
namespace = dict(globals())
class Service:
    async def get_client(self): return SimpleNamespace(driver=None)
upstream = SimpleNamespace(graphiti_service=Service(), NodeResult=NodeResult,
    NodeSearchResponse=NodeSearchResponse, ErrorResponse=ErrorResponse,
    config=SimpleNamespace(graphiti=SimpleNamespace(group_id='user_primary')))
selected_node = None
async def collect_group_scoped(*args): return [selected_node]
namespace.update(upstream=upstream, collect_group_scoped=collect_group_scoped)
exec(compile(ast.Module(body=[wrapper], type_ignores=[]), 'observed-node-wrapper', 'exec'), namespace)

cases = []
for timestamp in [
    "2026-01-01T00:00:00+00:00",
    "2026-01-01T00:00:00.123+00:00",
    "2026-01-01T00:00:00.000001Z",
    "2026-01-01T09:00:00+09:00",
]:
    instant = datetime.datetime.fromisoformat(timestamp)
    node = EntityNode(
        uuid="synthetic-node", name="Synthetic", group_id="user_primary",
        created_at=instant, summary="", labels=["Entity"],
        attributes={"embedding_note": "keep", "name_embedding": [1.0]},
    )
    edge = EntityEdge(
        uuid="synthetic-edge", source_node_uuid="source", target_node_uuid="target",
        name="TEST", fact="Synthetic", group_id="user_primary", created_at=instant,
        valid_at=instant, invalid_at=instant, expired_at=instant,
        episodes=["synthetic-episode"],
        attributes={"embedding_note": "keep", "fact_embedding": [1.0]},
    )
    selected_node = node
    node_response = asyncio.run(namespace['_search_nodes_group_aware']('synthetic'))
    cases.append({
        "stored_timestamp": timestamp,
        "expected_node": node_response['nodes'][0],
        "expected_fact": format_fact_result(edge),
    })
print(json.dumps({
    "scope": "synthetic observed Python SDK public DTO fixtures",
    "formatter_sha256": hashlib.sha256(
        Path("/app/mcp/src/utils/formatting.py").read_bytes()
    ).hexdigest(),
    "node_wrapper_sha256": hashlib.sha256(source_path.read_bytes()).hexdigest(),
    "cases": cases,
}, ensure_ascii=False))
