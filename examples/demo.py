#!/usr/bin/env python3
"""Standard stateless MCP HTTP example; no LLM API key needed."""
import json
import os
import pathlib
import urllib.request
base = os.environ.get("MCP_URL", "http://127.0.0.1:8080/mcp")
token = os.environ.get("GRAPHITI_TOKEN")
sequence = 0

def rpc(method, params):
    global sequence
    sequence += 1
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream", "MCP-Protocol-Version": "2025-11-25"}
    if token:
        headers["Authorization"] = "Bearer " + token
    body = json.dumps({"jsonrpc": "2.0", "id": sequence, "method": method, "params": params}).encode()
    with urllib.request.urlopen(urllib.request.Request(base, body, headers), timeout=120) as response:
        result = json.load(response)
    if "error" in result:
        raise RuntimeError(result["error"])
    value = result["result"]
    if value.get("isError"):
        raise RuntimeError(value)
    return value

print(rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "graphiti-rs-demo", "version": "1"}}))
episode = json.loads(pathlib.Path(__file__).with_name("episode.json").read_text())
print(rpc("tools/call", {"name": "add_episode", "arguments": episode}))
print(rpc("tools/call", {"name": "search_facts", "arguments": {"group_id": "demo", "query": "Alice", "as_of": "2026-10-01T01:00:00Z"}}))
