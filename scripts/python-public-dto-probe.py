#!/usr/bin/env python3
"""Emit synthetic public DTOs using the observed Python MCP formatter.

Run inside the observed Graphiti image; imports never initialize its server or
call providers. Output contains only constants from this script.
"""
import datetime
import hashlib
import json
import sys
from pathlib import Path

sys.path[:0] = [
    "/app/mcp/.venv/lib/python3.11/site-packages",
    "/app/mcp/src",
]
from graphiti_core.edges import EntityEdge
from graphiti_core.nodes import EntityNode
from utils.formatting import format_fact_result, format_node_result

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
    cases.append({
        "stored_timestamp": timestamp,
        "expected_node": format_node_result(node),
        "expected_fact": format_fact_result(edge),
    })
print(json.dumps({
    "scope": "synthetic observed Python SDK public DTO fixtures",
    "formatter_sha256": hashlib.sha256(
        Path("/app/mcp/src/utils/formatting.py").read_bytes()
    ).hexdigest(),
    "cases": cases,
}, ensure_ascii=False))
