#!/usr/bin/env python3
"""Isolated image-only Python retrieval baseline, not full ingestion/MCP.

Invoke with python -S and env -i in the labelled synthetic DB container after
host ownership checks. Never invoke the operational application entrypoint.
Uses real graphiti_core driver/search bodies and the observed edge-read patch,
with supplied mock vectors and no model clients or credentials.
"""
import asyncio
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import random
import resource
import runpy
import sys
import time

assert os.environ.get("GRAPHITI_PI_SYNTHETIC") == "graphiti-rs-compat-pi"
sys.path.insert(0, "/app/mcp/.venv/lib/python3.11/site-packages")

from graphiti_core.driver.falkordb_driver import FalkorDriver
from graphiti_core.graphiti_types import GraphitiClients
from graphiti_core.search.search import search
from graphiti_core.search.search_config_recipes import EDGE_HYBRID_SEARCH_RRF
from graphiti_core.search.search_filters import SearchFilters
from graphiti_core.tracer import NoOpTracer

GROUP = "project_pi_benchmark_20261002"


async def main():
    assert importlib.metadata.version("graphiti-core") == "0.28.2"
    patch = Path("/tmp/falkor_edge_query_compat.py")
    patch_sha = hashlib.sha256(patch.read_bytes()).hexdigest()
    runpy.run_path(str(patch))["install"]()
    driver = FalkorDriver(host="127.0.0.1", port=6379, database=GROUP)
    try:
        records, _, _ = await driver.execute_query("MATCH ()-[e:RELATES_TO]->() RETURN count(e) AS count")
        assert records[0]["count"] == 1200
        records, _, _ = await driver.execute_query("MATCH ()-[e:RELATES_TO]->() RETURN e.uuid AS id,e.fact AS fact ORDER BY id")
        corpus_sha = hashlib.sha256(json.dumps(records, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        await driver.build_indices_and_constraints()
        clients = GraphitiClients.model_construct(driver=driver, llm_client=None,
            embedder=None, cross_encoder=None, tracer=NoOpTracer())
        config = EDGE_HYBRID_SEARCH_RRF.model_copy(deep=True)
        config.limit = 10
        async def query(text, vector):
            return await search(clients, text, [GROUP], config, SearchFilters(), query_vector=vector)
        outlier = await query("semantic outlier", [0., 1.] + [0.]*3070)
        outlier_ids = [e.uuid for e in outlier.edges]
        expected = f"00000000-0000-4000-8000-{101199:012x}"
        samples, ranks = [], []
        order = random.Random(20261002).sample(range(1200), 100)
        for i in order[:5]:
            await query(f"marker{i:06d}", [1.] + [0.]*3071)
        for i in order:
            began = time.perf_counter()
            result = await query(f"marker{i:06d}", [1.] + [0.]*3071)
            samples.append((time.perf_counter()-began)*1000)
            ids = [e.uuid for e in result.edges]
            target = f"00000000-0000-4000-8000-{i+100000:012x}"
            ranks.append(1/(ids.index(target)+1) if target in ids else 0)
        samples.sort()
        report = {"scope": "real Python driver + complete hybrid edge-search path on synthetic DB; no extraction/MCP/provider parity",
            "package": "graphiti-core 0.28.2", "runtime_patch_sha256": patch_sha,
            "corpus_facts": 1200, "samples": 100, "warmup": 5,
            "legacy_text_identity_corpus_sha256": corpus_sha,
            "p50_ms": samples[50], "p95_ms": samples[95], "p99_ms": samples[99],
            "recall_at_10": sum(r > 0 for r in ranks)/100, "mrr_at_10": sum(ranks)/100,
            "semantic_outlier_recall_at_10": int(expected in outlier_ids),
            "semantic_outlier_returned_uuids": outlier_ids,
            "max_rss_KiB": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
            "execution_limit": "shares isolated DB one-CPU/512MiB cgroup; SDK loopback path, not Rust HTTP/relay timing",
            "actual_provider_calls": 0}
        print(json.dumps(report, indent=2))
    finally:
        await driver.close()


if __name__ == "__main__":
    asyncio.run(main())
